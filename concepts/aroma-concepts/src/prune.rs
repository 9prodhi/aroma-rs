//! Phase II prune and rerank (paper Algorithm 1; similar.py:791-827, 846-872).
//!
//! Prune keeps the subset R of a candidate's leaves whose features best cover a target
//! feature multiset, then retains every ancestor path of R. It is greedy: each round
//! adds the leaf with the best score after adding it, while the score strictly improves.
//!
//! Objective differs between sources:
//!   paper     SimScore = |T ∩ F(R)|      (monotone, diminishing returns: lazy greedy is valid)
//!   reference Jaccard  = Σmin / Σmax     (penalises extra features; not submodular)
//!
//! Each candidate's marginal score is computed incrementally from running Σmin and Σmax,
//! so one round costs O(Σ_n |F(n)|) instead of rebuilding Counters per trial.

use crate::features::{featurize, FeatureConfig, Vocab};
use crate::multiset::{jaccard, Bag, FeatureId};
use crate::spt::Spt;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Objective {
    Jaccard,
    IntersectionSize,
}

fn score(obj: Objective, smin: u64, smax: u64) -> f64 {
    match obj {
        Objective::Jaccard if smax == 0 => 0.0,
        Objective::Jaccard => smin as f64 / smax as f64,
        Objective::IntersectionSize => smin as f64,
    }
}

/// Greedy leaf selection. `targets` are processed in order with one shared accumulator,
/// as `prune_last_jd([query, member], tree)` does when intersecting clusters.
/// Ties go to the earliest leaf in source order (strict `>`).
pub fn select_leaves(
    targets: &[&Bag],
    leaf_bags: &[Bag],
    candidates: &[bool],
    obj: Objective,
) -> Vec<bool> {
    let mut kept = vec![false; leaf_bags.len()];
    let mut acc: HashMap<FeatureId, u32> = HashMap::new();
    for target in targets {
        // Running Σmin and Σmax of (target, acc) over the union of their keys.
        let mut smin: u64 = 0;
        let mut smax: u64 = 0;
        for (f, t) in target.iter() {
            let c = acc.get(&f).copied().unwrap_or(0);
            smin += t.min(c) as u64;
            smax += t.max(c) as u64;
        }
        smax += acc
            .iter()
            .filter(|(f, _)| target.get(**f) == 0)
            .map(|(_, &c)| c as u64)
            .sum::<u64>();
        let mut current = score(obj, smin, smax);

        loop {
            let mut best: Option<(usize, f64, u64, u64)> = None;
            for (i, lb) in leaf_bags.iter().enumerate() {
                if kept[i] || !candidates[i] {
                    continue;
                }
                let (mut dmin, mut dmax) = (0u64, 0u64);
                for (f, l) in lb.iter() {
                    let t = target.get(f);
                    let c = acc.get(&f).copied().unwrap_or(0);
                    dmin += (t.min(c + l) - t.min(c)) as u64;
                    dmax += (t.max(c + l) - t.max(c)) as u64;
                }
                let s = score(obj, smin + dmin, smax + dmax);
                if s > best.map_or(current, |b| b.1) {
                    best = Some((i, s, dmin, dmax));
                }
            }
            let Some((i, s, dmin, dmax)) = best else { break };
            kept[i] = true;
            current = s;
            smin += dmin;
            smax += dmax;
            for (f, c) in leaf_bags[i].iter() {
                *acc.entry(f).or_default() += c;
            }
        }
    }
    kept
}

#[derive(Clone, Debug)]
pub struct Pruned {
    /// Kept-leaf mask; ancestors of kept leaves are implicitly retained.
    pub kept: Vec<bool>,
    /// Features of the pruned tree, re-extracted (similar.py:825-826) because sibling
    /// and variable-usage links change once leaves disappear.
    pub bag: Bag,
}

/// Prune `spt` (optionally an already-pruned view of it) towards `targets`.
pub fn prune(
    targets: &[&Bag],
    spt: &Spt,
    mask: Option<&[bool]>,
    cfg: &FeatureConfig,
    vocab: &mut Vocab,
    obj: Objective,
) -> Pruned {
    let per_leaf = featurize(spt, cfg, mask);
    let leaf_bags: Vec<Bag> = per_leaf.iter().map(|fs| vocab.bag(fs)).collect();
    let candidates = mask.map_or_else(|| vec![true; spt.leaves.len()], <[bool]>::to_vec);
    let kept = select_leaves(targets, &leaf_bags, &candidates, obj);
    let bag = vocab.unit_bag(&featurize(spt, cfg, Some(&kept)));
    Pruned { kept, bag }
}

#[derive(Clone, Debug)]
pub struct Candidate<'a> {
    pub unit: u32,
    pub spt: &'a Spt,
    /// F(m) of the unpruned unit.
    pub bag: Bag,
    pub pruned: Pruned,
    pub score: f64,
}

/// Rerank Phase I hits by Jaccard(F(q), F(pruned)), keep those above `min_pruned`
/// (reference 0.65, the paper's tau1), stable-sorted by score descending.
/// Production: run the prunes in parallel (rayon) with a per-candidate time budget.
pub fn rerank<'a>(
    query: &Bag,
    hits: impl IntoIterator<Item = (u32, &'a Spt, Bag)>,
    cfg: &FeatureConfig,
    vocab: &mut Vocab,
    min_pruned: f64,
) -> Vec<Candidate<'a>> {
    let mut out: Vec<Candidate<'a>> = hits
        .into_iter()
        .filter_map(|(unit, spt, bag)| {
            let pruned = prune(&[query], spt, None, cfg, vocab, Objective::Jaccard);
            let score = jaccard(query, &pruned.bag);
            (score > min_pruned).then_some(Candidate { unit, spt, bag, pruned, score })
        })
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out
}
