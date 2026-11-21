//! Explain a match (`aroma query --explain`): which features the query and the pruned
//! candidate share, which only one side has, and their weighted Jaccard.
//!
//! ```text
//!   Q = feature strings of the whole query      C = feature strings of the candidate
//!                                                   restricted to its kept leaves
//!   matched = Q ∩ C    query_only = Q \ C    candidate_only = C \ Q   (sets, sorted)
//!   jaccard = Σmin / Σmax over the multisets of feature ids (as in rerank)
//! ```
//!
//! The candidate is featurized with the kept mask, exactly like the rerank re-extracts
//! the pruned tree's features, so `jaccard` equals the rerank score under the Jaccard
//! objective. Lists hold feature strings; they are only produced on request.

use crate::features::{featurize, unit_bag, FeatureConfig};
use crate::lang::Lang;
use crate::multiset::jaccard;
use crate::spt::Spt;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Explanation {
    /// Features present in both the query and the pruned candidate.
    pub matched: Vec<String>,
    /// Query features the pruned candidate lacks.
    pub query_only: Vec<String>,
    /// Pruned-candidate features the query lacks.
    pub candidate_only: Vec<String>,
    /// Weighted Jaccard of the query bag and the pruned candidate bag.
    pub jaccard: f64,
}

/// `kept` is the candidate's kept-leaf mask (entries missing at the end count as
/// dropped).
pub fn explain(
    lang: Lang,
    cfg: &FeatureConfig,
    query: &Spt,
    cand: &Spt,
    kept: &[bool],
) -> Explanation {
    let mask: Vec<bool> = (0..cand.leaves.len())
        .map(|i| kept.get(i).copied().unwrap_or(false))
        .collect();
    let q_feats = featurize(query, cfg, None);
    let c_feats = featurize(cand, cfg, Some(&mask));
    let q_set: BTreeSet<&str> = q_feats.iter().flatten().map(String::as_str).collect();
    let c_set: BTreeSet<&str> = c_feats.iter().flatten().map(String::as_str).collect();
    Explanation {
        matched: to_strings(q_set.intersection(&c_set)),
        query_only: to_strings(q_set.difference(&c_set)),
        candidate_only: to_strings(c_set.difference(&q_set)),
        jaccard: jaccard(&unit_bag(lang, &q_feats), &unit_bag(lang, &c_feats)),
    }
}

fn to_strings<'a, 'b: 'a>(it: impl Iterator<Item = &'a &'b str>) -> Vec<String> {
    it.map(|s| (*s).to_owned()).collect()
}
