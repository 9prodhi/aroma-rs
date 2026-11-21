//! Phase III cluster and intersect (paper section 3.3.3; similar.py:689-748).
//!
//! A cluster is a tuple of reranked candidates (i1 < i2 < ... < ik). It is valid when
//!   s = |∩ pruned bags| / |first pruned bag| > tau3 (0.9)   members cover the same query part
//!   l = |∩ unpruned bags| / |∩ pruned bags|  > tau2 (1.5)   the shared code extends the query
//! Every candidate seeds a tuple; each round extends a tuple with the single later
//! candidate that maximises |∩ unpruned bags| (paper: maximise l). Singletons are never
//! emitted by the reference (the paper allows them).
//!
//! ```text
//!  seeds (0) (1) (2) ...            round 1: (0,k0) (1,k1) ...
//!     round 2: (0,k0,k0') ...   until no tuple grows
//!  sort by (first index asc, length desc) -> drop tuples with index-set Jaccard > 0.5
//!  against ANY earlier tuple (dropped ones too) -> keep top 5 -> intersect each
//! ```

use crate::features::{FeatureConfig, Vocab};
use crate::multiset::Bag;
use crate::prune::{prune, Candidate, Objective};

#[allow(clippy::needless_range_loop)] // `k` is the tuple member being pushed, not just an index
pub fn cluster(cands: &[Candidate<'_>], tau3: f64, tau2: f64, top_n: usize) -> Vec<Vec<usize>> {
    let n = cands.len();
    // (tuple, ∩ pruned bags, ∩ unpruned bags), grown incrementally.
    let mut frontier: Vec<(Vec<usize>, Bag, Bag)> = (0..n)
        .map(|i| (vec![i], cands[i].pruned.bag.clone(), cands[i].bag.clone()))
        .collect();
    let mut acc: Vec<Vec<usize>> = Vec::new();
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for (tuple, pruned_inter, full_inter) in &frontier {
            let qlen = cands[tuple[0]].pruned.bag.len();
            if qlen == 0 {
                continue;
            }
            let last = *tuple.last().expect("tuples are never empty");
            let mut best: Option<(usize, u64)> = None;
            for k in last + 1..n {
                let iscore = pruned_inter.intersection_size(&cands[k].pruned.bag);
                if iscore as f64 / qlen as f64 <= tau3 {
                    continue;
                }
                let tmp = full_inter.intersection_size(&cands[k].bag);
                if tmp as f64 > tau2 * iscore as f64 && tmp > best.map_or(0, |b| b.1) {
                    best = Some((k, tmp));
                }
            }
            if let Some((k, _)) = best {
                let mut t = tuple.clone();
                t.push(k);
                next.push((
                    t,
                    pruned_inter.intersect(&cands[k].pruned.bag),
                    full_inter.intersect(&cands[k].bag),
                ));
            }
        }
        acc.extend(next.iter().map(|(t, _, _)| t.clone()));
        frontier = next;
    }

    acc.sort_by_key(|t| (t[0], std::cmp::Reverse(t.len())));
    let mut out = Vec::new();
    for i in 0..acc.len() {
        let dup = (0..i).any(|j| index_jaccard(&acc[i], &acc[j]) > 0.5);
        if !dup {
            out.push(acc[i].clone());
            if out.len() >= top_n {
                break;
            }
        }
    }
    out
}

fn index_jaccard(a: &[usize], b: &[usize]) -> f64 {
    let inter = a.iter().filter(|x| b.contains(x)).count();
    let union = a.len() + b.len() - inter;
    inter as f64 / union as f64
}

/// Carve the recommendation out of the first member. Reference: start from the
/// UNPRUNED first member and, for each later member j, prune towards
/// [F(q), F(member j)] sequentially. Paper: Prune(F(member j) ⊎ F(q), previous result).
/// The query features are always a target, so query fragments are not dropped.
pub fn intersect(
    query: &Bag,
    tuple: &[usize],
    cands: &[Candidate<'_>],
    cfg: &FeatureConfig,
    vocab: &mut Vocab,
) -> Vec<bool> {
    let first = &cands[tuple[0]];
    let mut mask: Option<Vec<bool>> = None;
    for &j in &tuple[1..] {
        let p = prune(&[query, &cands[j].bag], first.spt, mask.as_deref(), cfg, vocab, Objective::Jaccard);
        mask = Some(p.kept);
    }
    mask.unwrap_or_else(|| first.pruned.kept.clone())
}
