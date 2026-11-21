//! Phase I light-weight search (paper section 3.3.1; similar.py:769-782).
//!
//! The paper computes `D · v_q` with a sparse binary method-by-feature matrix. With
//! binary entries that is exactly an inverted index: for every distinct query feature,
//! add 1 to each unit in its posting list. The score is |S(F(m)) ∩ S(F(q))|: counts
//! are ignored so a method repeating a feature n times gets no extra credit.

use crate::multiset::{Bag, FeatureId};
use std::collections::HashMap;

pub type UnitId = u32;

#[derive(Default, Debug)]
pub struct InvertedIndex {
    postings: HashMap<FeatureId, Vec<UnitId>>,
    n_units: u32,
}

impl InvertedIndex {
    /// Units must be inserted in increasing id order so posting lists stay sorted.
    pub fn insert(&mut self, unit: UnitId, bag: &Bag) {
        for f in bag.distinct() {
            self.postings.entry(f).or_default().push(unit);
        }
        self.n_units = self.n_units.max(unit + 1);
    }

    /// Reference filter: keep units with overlap strictly greater than
    /// `min_ratio * (number of distinct query features known to the index)`, then the
    /// top `cap` by overlap. Paper: no ratio filter, cap eta1 = 1000. Reference: 0.4, 100.
    /// Ties break by ascending unit id (NumPy's argsort order is unspecified).
    pub fn search(&self, query: &Bag, min_ratio: f64, cap: usize) -> Vec<(UnitId, u32)> {
        let mut scores = vec![0u32; self.n_units as usize];
        let mut known = 0u32;
        for f in query.distinct() {
            if let Some(list) = self.postings.get(&f) {
                known += 1;
                for &u in list {
                    scores[u as usize] += 1;
                }
            }
        }
        let threshold = min_ratio * known as f64;
        let mut hits: Vec<(UnitId, u32)> = scores
            .iter()
            .enumerate()
            .filter(|&(_, &s)| s > 0 && s as f64 > threshold)
            .map(|(u, &s)| (u as UnitId, s))
            .collect();
        if hits.len() > cap {
            hits.select_nth_unstable_by(cap, |a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            hits.truncate(cap);
        }
        hits.sort_unstable_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        hits
    }
}
