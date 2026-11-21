//! Phase II prune and rerank (paper Algorithm 1; `similar.py:791-827, 846-872`).
//!
//! Prune keeps the subset R of a candidate's leaves whose features best cover a target
//! multiset, then retains every ancestor path of R. It is greedy: each round adds the
//! leaf with the best score after adding it, while the score strictly improves; ties go
//! to the earliest leaf in source order.
//!
//! Objectives:
//!   `Jaccard`          reference weighted Jaccard Σmin/Σmax (not submodular): eager scan.
//!   `IntersectionSize` paper SimScore |T ∩ F(R)| (monotone submodular): lazy greedy over a
//!                      `BinaryHeap<(gain, Reverse(leaf), stamp)>`; leaves with zero gain
//!                      are dropped for good (gains only shrink). Selects exactly what
//!                      the eager scan selects, including the earliest-leaf tie rule.
//!
//! Feature ids are remapped to a dense per-call index so every marginal gain is two
//! array reads per feature. The deadline is checked at the start of every greedy round
//! (stage `"prune"`) and before every rerank candidate (stage `"rerank"`).

use crate::deadline::{Cancelled, Deadline};
use crate::features::{featurize, leaf_bags, unit_bag, FeatureConfig};
use crate::lang::Lang;
use crate::multiset::{jaccard, Bag, FeatureId};
use crate::spt::Spt;
use crate::unit::UnitId;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Objective {
    Jaccard,
    IntersectionSize,
}

const PRUNE_STAGE: &str = "prune";
const RERANK_STAGE: &str = "rerank";

fn score(obj: Objective, smin: u64, smax: u64) -> f64 {
    match obj {
        Objective::Jaccard if smax == 0 => 0.0,
        Objective::Jaccard => smin as f64 / smax as f64,
        Objective::IntersectionSize => smin as f64,
    }
}

/// Dense remap of every feature that occurs in a target or a leaf bag.
struct Dense {
    targets: Vec<Vec<u64>>,
    leaves: Vec<Vec<(usize, u64)>>,
    n_features: usize,
}

impl Dense {
    fn new(targets: &[&Bag], leaf_bags: &[Bag]) -> Dense {
        let mut ids: HashMap<FeatureId, usize> = HashMap::new();
        let mut intern = |f: FeatureId| {
            let next = ids.len();
            *ids.entry(f).or_insert(next)
        };
        let sparse_targets: Vec<Vec<(usize, u64)>> = targets
            .iter()
            .map(|t| t.iter().map(|(f, c)| (intern(f), u64::from(c))).collect())
            .collect();
        let leaves: Vec<Vec<(usize, u64)>> = leaf_bags
            .iter()
            .map(|b| b.iter().map(|(f, c)| (intern(f), u64::from(c))).collect())
            .collect();
        let n_features = ids.len();
        let targets = sparse_targets
            .into_iter()
            .map(|st| {
                let mut dense = vec![0u64; n_features];
                for (i, c) in st {
                    dense[i] = c;
                }
                dense
            })
            .collect();
        Dense {
            targets,
            leaves,
            n_features,
        }
    }
}

/// Marginal (Δ Σmin, Δ Σmax) of adding `leaf` to `acc` against target `t`.
fn delta(t: &[u64], acc: &[u64], leaf: &[(usize, u64)]) -> (u64, u64) {
    let (mut dmin, mut dmax) = (0u64, 0u64);
    for &(f, l) in leaf {
        let (tc, c) = (t[f], acc[f]);
        dmin += tc.min(c + l) - tc.min(c);
        dmax += tc.max(c + l) - tc.max(c);
    }
    (dmin, dmax)
}

/// Greedy leaf selection. `targets` are processed in order with one shared
/// accumulator, as `prune_last_jd([query, member], tree)` does when intersecting
/// clusters. Only leaves with `candidates[i]` (missing entries count as `false`) can be
/// selected. Returns the kept mask, or `Cancelled { stage: "prune" }`.
pub fn select_leaves(
    targets: &[&Bag],
    leaf_bags: &[Bag],
    candidates: &[bool],
    obj: Objective,
    deadline: &Deadline,
) -> Result<Vec<bool>, Cancelled> {
    deadline.check(PRUNE_STAGE)?;
    let dense = Dense::new(targets, leaf_bags);
    let mut kept = vec![false; leaf_bags.len()];
    let mut acc = vec![0u64; dense.n_features];
    let open = |kept: &[bool], i: usize| !kept[i] && candidates.get(i).copied().unwrap_or(false);
    for t in &dense.targets {
        match obj {
            Objective::Jaccard => {
                eager_jaccard(t, &dense.leaves, &mut acc, &mut kept, &open, deadline)?
            }
            Objective::IntersectionSize => {
                lazy_intersection(t, &dense.leaves, &mut acc, &mut kept, &open, deadline)?
            }
        }
    }
    Ok(kept)
}

fn accept(i: usize, leaves: &[Vec<(usize, u64)>], acc: &mut [u64], kept: &mut [bool]) {
    kept[i] = true;
    for &(f, c) in &leaves[i] {
        acc[f] += c;
    }
}

fn eager_jaccard(
    t: &[u64],
    leaves: &[Vec<(usize, u64)>],
    acc: &mut [u64],
    kept: &mut [bool],
    open: &dyn Fn(&[bool], usize) -> bool,
    deadline: &Deadline,
) -> Result<(), Cancelled> {
    let mut smin: u64 = t.iter().zip(acc.iter()).map(|(&a, &b)| a.min(b)).sum();
    let mut smax: u64 = t.iter().zip(acc.iter()).map(|(&a, &b)| a.max(b)).sum();
    let mut current = score(Objective::Jaccard, smin, smax);
    loop {
        deadline.check(PRUNE_STAGE)?;
        let mut best: Option<(usize, f64, u64, u64)> = None;
        for (i, leaf) in leaves.iter().enumerate() {
            if !open(kept, i) {
                continue;
            }
            let (dmin, dmax) = delta(t, acc, leaf);
            let s = score(Objective::Jaccard, smin + dmin, smax + dmax);
            if s > best.map_or(current, |b| b.1) {
                best = Some((i, s, dmin, dmax));
            }
        }
        let Some((i, s, dmin, dmax)) = best else {
            return Ok(());
        };
        accept(i, leaves, acc, kept);
        current = s;
        smin += dmin;
        smax += dmax;
    }
}

fn lazy_intersection(
    t: &[u64],
    leaves: &[Vec<(usize, u64)>],
    acc: &mut [u64],
    kept: &mut [bool],
    open: &dyn Fn(&[bool], usize) -> bool,
    deadline: &Deadline,
) -> Result<(), Cancelled> {
    let mut round: u64 = 0;
    let mut heap: BinaryHeap<(u64, Reverse<usize>, u64)> = BinaryHeap::new();
    for (i, leaf) in leaves.iter().enumerate() {
        if !open(kept, i) {
            continue;
        }
        let (gain, _) = delta(t, acc, leaf);
        if gain > 0 {
            heap.push((gain, Reverse(i), round));
        }
    }
    loop {
        deadline.check(PRUNE_STAGE)?;
        let mut accepted = false;
        while let Some((gain, Reverse(i), stamp)) = heap.pop() {
            if stamp == round {
                debug_assert!(gain > 0);
                accept(i, leaves, acc, kept);
                round += 1;
                accepted = true;
                break;
            }
            let (fresh, _) = delta(t, acc, &leaves[i]);
            if fresh > 0 {
                heap.push((fresh, Reverse(i), round));
            }
        }
        if !accepted {
            return Ok(());
        }
    }
}

#[derive(Clone, Debug)]
pub struct Pruned {
    /// Kept-leaf mask; ancestors of kept leaves are implicitly retained.
    pub kept: Vec<bool>,
    /// Features of the pruned tree, re-extracted (`similar.py:825-826`): sibling and
    /// usage links change once leaves disappear.
    pub bag: Bag,
}

/// Prune `spt` (optionally an already-pruned view `mask` of it) towards `targets`.
pub fn prune(
    targets: &[&Bag],
    spt: &Spt,
    mask: Option<&[bool]>,
    lang: Lang,
    cfg: &FeatureConfig,
    obj: Objective,
    deadline: &Deadline,
) -> Result<Pruned, Cancelled> {
    // Normalise the mask to one entry per leaf (missing entries are hidden) so the
    // featurizer never sees a short mask.
    let candidates: Vec<bool> = match mask {
        Some(m) => (0..spt.leaves.len())
            .map(|i| m.get(i).copied().unwrap_or(false))
            .collect(),
        None => vec![true; spt.leaves.len()],
    };
    let per_leaf = featurize(spt, cfg, mask.map(|_| candidates.as_slice()));
    let bags = leaf_bags(lang, &per_leaf);
    let kept = select_leaves(targets, &bags, &candidates, obj, deadline)?;
    let bag = unit_bag(lang, &featurize(spt, cfg, Some(&kept)));
    Ok(Pruned { kept, bag })
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub unit: UnitId,
    pub spt: Spt,
    /// F(m) of the unpruned unit.
    pub bag: Bag,
    pub pruned: Pruned,
    pub score: f64,
}

/// Rerank score of a pruned candidate: weighted Jaccard (reference) or
/// SimScore / |F(q)| (paper profile, plan §3.4); 0 for an empty query.
fn rerank_score(query: &Bag, pruned: &Bag, obj: Objective) -> f64 {
    match obj {
        Objective::Jaccard => jaccard(query, pruned),
        Objective::IntersectionSize if query.is_empty() => 0.0,
        Objective::IntersectionSize => query.intersection_size(pruned) as f64 / query.len() as f64,
    }
}

/// Rerank Phase I hits: prune each towards `query`, keep those with score
/// `> min_pruned` (reference 0.65), stable-sorted by score descending (equal scores keep
/// Phase I order). When the deadline fires, the candidates finished so far are returned
/// (sorted) together with the `Cancelled` that stopped the loop.
pub fn rerank(
    query: &Bag,
    hits: Vec<(UnitId, Spt, Bag)>,
    lang: Lang,
    cfg: &FeatureConfig,
    obj: Objective,
    min_pruned: f64,
    deadline: &Deadline,
) -> (Vec<Candidate>, Option<Cancelled>) {
    let mut out = Vec::new();
    let mut cancelled = None;
    for (unit, spt, bag) in hits {
        if let Err(c) = deadline.check(RERANK_STAGE) {
            cancelled = Some(c);
            break;
        }
        let pruned = match prune(&[query], &spt, None, lang, cfg, obj, deadline) {
            Ok(p) => p,
            Err(c) => {
                cancelled = Some(c);
                break;
            }
        };
        let score = rerank_score(query, &pruned.bag, obj);
        if score > min_pruned {
            out.push(Candidate {
                unit,
                spt,
                bag,
                pruned,
                score,
            });
        }
    }
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    (out, cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_remap_interns_targets_first() {
        let d = Dense::new(&[&Bag::from_ids([5, 5, 9])], &[Bag::from_ids([9, 11])]);
        assert_eq!(d.n_features, 3);
        assert_eq!(d.targets, [vec![2, 1, 0]]);
        assert_eq!(d.leaves, [vec![(1, 1), (2, 1)]]);
    }

    #[test]
    fn lazy_greedy_drops_zero_gain_leaves() {
        let t = [1u64, 1, 0];
        let leaves = vec![vec![(2usize, 1u64)], vec![(0, 1)], vec![(0, 1), (1, 1)]];
        let mut acc = vec![0u64; 3];
        let mut kept = vec![false; 3];
        let open = |kept: &[bool], i: usize| !kept[i];
        lazy_intersection(&t, &leaves, &mut acc, &mut kept, &open, &Deadline::none()).unwrap();
        assert_eq!(kept, [false, false, true]);
        assert_eq!(acc, [1, 1, 0]);
    }

    #[test]
    fn delta_reports_min_and_max_gains() {
        assert_eq!(delta(&[2, 0], &[1, 0], &[(0, 3), (1, 1)]), (1, 3));
    }
}
