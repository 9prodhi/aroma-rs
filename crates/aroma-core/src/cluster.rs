//! Phase III cluster and intersect (paper section 3.3.3; reference `get_completions3`,
//! similar.py:689-748), plus the D26/D27 recommendation gate.
//!
//! A cluster is a tuple of reranked candidates `(i1 < i2 < ... < ik)`, `k >= 2`. Adding
//! candidate `k` to a tuple `t` is allowed when, with `qlen` = size of the FIRST
//! member's PRUNED bag:
//!   iscore = |∩ pruned bags of t + k|,  iscore / qlen > tau3 (0.9)   (strict)
//!   tmp    = |∩ unpruned bags of t + k|, tmp > tau2 * iscore (1.5)  (strict)
//! Every candidate seeds a tuple; each round extends every tuple of the previous round
//! with the single later candidate of maximal `tmp` (strict `>`, so the earliest index
//! wins ties); every extension is kept. Seeds whose pruned bag is empty never grow.
//! Singletons are never emitted.
//!
//! ```text
//!  seeds (0) (1) (2) ...            round 1: (0,k0) (1,k1) ...
//!     round 2: (0,k0,k0') ...   until no tuple grows
//!  sort by (first index asc, length desc) -> drop tuples with index-set Jaccard > 0.5
//!  against ANY earlier tuple (dropped ones too) -> keep top_n -> intersect each
//! ```
//!
//! The reference sorts by the key `first * 1000 - len`; that equals the order
//! `(first asc, len desc)` whenever there are at most 1000 candidates, which the
//! Phase I cap guarantees for both profiles. `(first, len)` is unique per tuple (each
//! seed grows one chain), so the order is total.
//!
//! The deadline (stage `"cluster"`) is checked at the start of every round and before
//! each tuple's extension scan; `intersect` checks stage `"intersect"` before each
//! member, and the prune it calls checks its own stage.

use crate::deadline::{Cancelled, Deadline};
use crate::features::FeatureConfig;
use crate::lang::Lang;
use crate::multiset::Bag;
use crate::prune::{prune, Candidate, Objective};
use crate::unit::UnitId;
use std::collections::HashSet;

const CLUSTER_STAGE: &str = "cluster";
const INTERSECT_STAGE: &str = "intersect";

/// Confidence gate for `--experimental-recommend` (D26/D27).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecGate {
    /// Minimum number of members with distinct `body_hash`.
    pub min_independent: usize,
    /// Added leaves may be at most this multiple of the query's leaves.
    pub max_extension_ratio: f64,
}

impl RecGate {
    pub const DEFAULT: RecGate = RecGate {
        min_independent: 3,
        max_extension_ratio: 2.0,
    };
}

/// One growing tuple with its running intersections.
struct Growing {
    tuple: Vec<usize>,
    /// ∩ of the members' pruned bags.
    pruned_inter: Bag,
    /// ∩ of the members' unpruned bags.
    full_inter: Bag,
}

/// Grow, deduplicate and rank candidate tuples; indices refer to `cands`, which must be
/// in rerank order (score descending). Returns at most `top_n` tuples, each of length
/// at least 2, in output order.
pub fn cluster(
    cands: &[Candidate],
    tau3: f64,
    tau2: f64,
    top_n: usize,
    deadline: &Deadline,
) -> Result<Vec<Vec<usize>>, Cancelled> {
    if top_n == 0 {
        return Ok(Vec::new());
    }
    let mut frontier: Vec<Growing> = cands
        .iter()
        .enumerate()
        .map(|(i, c)| Growing {
            tuple: vec![i],
            pruned_inter: c.pruned.bag.clone(),
            full_inter: c.bag.clone(),
        })
        .collect();
    let mut acc: Vec<Vec<usize>> = Vec::new();
    while !frontier.is_empty() {
        deadline.check(CLUSTER_STAGE)?;
        let mut next = Vec::new();
        for g in &frontier {
            deadline.check(CLUSTER_STAGE)?;
            if let Some(k) = best_extension(g, cands, tau3, tau2) {
                let mut tuple = g.tuple.clone();
                tuple.push(k);
                next.push(Growing {
                    tuple,
                    pruned_inter: g.pruned_inter.intersect(&cands[k].pruned.bag),
                    full_inter: g.full_inter.intersect(&cands[k].bag),
                });
            }
        }
        acc.extend(next.iter().map(|g| g.tuple.clone()));
        frontier = next;
    }

    acc.sort_by_key(|t| (t[0], std::cmp::Reverse(t.len())));
    let mut out = Vec::new();
    for (i, t) in acc.iter().enumerate() {
        if out.len() >= top_n {
            break;
        }
        let dup = acc[..i].iter().any(|prev| index_jaccard(t, prev) > 0.5);
        if !dup {
            out.push(t.clone());
        }
    }
    Ok(out)
}

/// The later candidate with maximal unpruned intersection that passes both thresholds.
fn best_extension(g: &Growing, cands: &[Candidate], tau3: f64, tau2: f64) -> Option<usize> {
    let qlen = cands[g.tuple[0]].pruned.bag.len();
    if qlen == 0 {
        return None;
    }
    let last = *g.tuple.last().expect("tuples are never empty");
    let mut best: Option<(usize, u64)> = None;
    for (k, cand) in cands.iter().enumerate().skip(last + 1) {
        let iscore = g.pruned_inter.intersection_size(&cand.pruned.bag);
        if iscore as f64 / qlen as f64 <= tau3 {
            continue;
        }
        let tmp = g.full_inter.intersection_size(&cand.bag);
        if tmp as f64 > tau2 * iscore as f64 && tmp > best.map_or(0, |b| b.1) {
            best = Some((k, tmp));
        }
    }
    best.map(|b| b.0)
}

/// Jaccard of two tuples as index sets (tuple indices are distinct).
fn index_jaccard(a: &[usize], b: &[usize]) -> f64 {
    let inter = a.iter().filter(|x| b.contains(x)).count();
    let union = a.len() + b.len() - inter;
    if union == 0 {
        return 0.0;
    }
    inter as f64 / union as f64
}

/// Carve the recommendation out of the first member (reference): start from the
/// UNPRUNED first member and, for each later member j, prune towards [F(q), F(member j)]
/// sequentially (`prune_last_jd`, Jaccard objective whatever the profile). The query
/// features are always the first target, so query fragments stay. Returns the kept-leaf
/// mask over the first member's leaves; a one-member tuple returns that member's rerank
/// mask and an empty tuple an empty mask.
pub fn intersect(
    query: &Bag,
    tuple: &[usize],
    cands: &[Candidate],
    lang: Lang,
    cfg: &FeatureConfig,
    deadline: &Deadline,
) -> Result<Vec<bool>, Cancelled> {
    let Some((&first_idx, rest)) = tuple.split_first() else {
        return Ok(Vec::new());
    };
    let first = &cands[first_idx];
    let mut mask: Option<Vec<bool>> = None;
    for &j in rest {
        deadline.check(INTERSECT_STAGE)?;
        let p = prune(
            &[query, &cands[j].bag],
            &first.spt,
            mask.as_deref(),
            lang,
            cfg,
            Objective::Jaccard,
            deadline,
        )?;
        mask = Some(p.kept);
    }
    Ok(mask.unwrap_or_else(|| first.pruned.kept.clone()))
}

/// Number of tuple members with pairwise distinct `body_hash` (clones count once).
pub fn independent_support(
    tuple: &[usize],
    cands: &[Candidate],
    body_hash: &dyn Fn(UnitId) -> u64,
) -> usize {
    tuple
        .iter()
        .map(|&i| body_hash(cands[i].unit))
        .collect::<HashSet<u64>>()
        .len()
}

/// Recommendation gate (D26/D27): at least `min_independent` independent members, and
/// the recommendation adds between 1 and `max_extension_ratio * query_leaves` kept
/// leaves beyond the query's size. A query without leaves never passes.
pub fn passes_gate(gate: &RecGate, support: usize, kept: &[bool], query_leaves: usize) -> bool {
    let added = kept
        .iter()
        .filter(|&&k| k)
        .count()
        .saturating_sub(query_leaves);
    support >= gate.min_independent
        && added > 0
        && added as f64 <= gate.max_extension_ratio * query_leaves as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prune::Pruned;
    use crate::spt::{Leaf, Raw, Spt};
    use std::time::Duration;

    fn tiny_spt() -> Spt {
        Spt::build(Raw::Leaf(Leaf {
            text: "x".into(),
            is_var: false,
            binding: None,
            bytes: 0..0,
            line: 1,
        }))
        .unwrap()
    }

    fn cand(unit: UnitId, full: &[u64], pruned: &[u64]) -> Candidate {
        Candidate {
            unit,
            spt: tiny_spt(),
            bag: Bag::from_ids(full.iter().copied()),
            pruned: Pruned {
                kept: vec![true],
                bag: Bag::from_ids(pruned.iter().copied()),
            },
            score: 1.0,
        }
    }

    #[test]
    fn gate_default_values() {
        assert_eq!(
            RecGate::DEFAULT,
            RecGate {
                min_independent: 3,
                max_extension_ratio: 2.0
            }
        );
    }

    #[test]
    fn three_extending_members_form_one_cluster() {
        let full = [1, 2, 3, 4, 5, 6, 7, 8];
        let cands = vec![
            cand(10, &full, &[1, 2, 3]),
            cand(11, &full, &[1, 2, 3]),
            cand(12, &full, &[1, 2, 3]),
            cand(13, &[1, 2, 3], &[1, 2, 3]),
        ];
        let out = cluster(&cands, 0.9, 1.5, 5, &Deadline::none()).unwrap();
        assert_eq!(out, vec![vec![0, 1, 2]]);
    }

    #[test]
    fn no_singletons_when_nothing_extends() {
        let cands = vec![
            cand(1, &[1, 2, 3], &[1, 2, 3]),
            cand(2, &[1, 2, 3], &[1, 2, 3]),
        ];
        let out = cluster(&cands, 0.9, 1.5, 5, &Deadline::none()).unwrap();
        assert!(out.is_empty());
        let lone = vec![cand(1, &[1, 2, 3, 4, 5, 6, 7], &[1, 2, 3])];
        assert!(cluster(&lone, 0.9, 1.5, 5, &Deadline::none())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn cluster_honours_the_deadline() {
        let cands = vec![cand(1, &[1], &[1])];
        assert_eq!(
            cluster(&cands, 0.9, 1.5, 5, &Deadline::after(Duration::ZERO)),
            Err(Cancelled { stage: "cluster" })
        );
    }

    #[test]
    fn independent_support_counts_distinct_body_hashes() {
        let cands = vec![
            cand(0, &[1], &[1]),
            cand(1, &[1], &[1]),
            cand(2, &[1], &[1]),
        ];
        assert_eq!(
            independent_support(&[0, 1, 2], &cands, &|u| u64::from(u)),
            3
        );
        assert_eq!(
            independent_support(&[0, 1, 2], &cands, &|u| u64::from(u.min(1))),
            2
        );
    }

    #[test]
    fn passes_gate_rules() {
        let g = RecGate::DEFAULT;
        let kept = |n: usize| vec![true; n];
        assert!(passes_gate(&g, 3, &kept(6), 3)); // added 3 <= 6
        assert!(passes_gate(&g, 3, &kept(9), 3)); // added 6 == 2 * 3
        assert!(!passes_gate(&g, 3, &kept(10), 3)); // added 7 > 6
        assert!(!passes_gate(&g, 3, &kept(3), 3)); // nothing added
        assert!(!passes_gate(&g, 2, &kept(6), 3)); // too little support
        assert!(passes_gate(&g, 3, &[true, false, true, false], 1)); // 2 kept, added 1
    }
}
