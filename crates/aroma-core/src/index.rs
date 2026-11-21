//! Phase I light-weight search (paper §3.3.1; `similar.py:769-782`).
//!
//! The paper's `D · v_q` over a binary unit-by-feature matrix is an inverted index:
//! for every distinct query feature, add 1 to each unit in its posting list. The score
//! is |S(F(m)) ∩ S(F(q))|; counts are ignored.
//!
//! Scores accumulate in a thread-local dense `Vec<u32>` sized to `n_units`, with a
//! `touched` list so only touched slots are reset afterwards (postings pattern, note 12).
//! Ranking is `(overlap desc, unit asc)`; top-k uses `select_nth_unstable_by` on
//! successive windows so ineligible units (D23b) never consume the cap.

use crate::multiset::{Bag, FeatureId};
use crate::unit::UnitId;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;

pub trait PostingSource: Sync {
    fn n_units(&self) -> u32;
    /// Ascending unit ids holding feature `f`; empty when unknown.
    fn postings(&self, f: FeatureId) -> &[UnitId];
}

#[derive(Default)]
pub struct MemIndex {
    postings: HashMap<FeatureId, Vec<UnitId>>,
    n_units: u32,
}

impl MemIndex {
    /// Units must be inserted in increasing id order so posting lists stay sorted.
    pub fn insert(&mut self, unit: UnitId, bag: &Bag) {
        for f in bag.distinct() {
            self.postings.entry(f).or_default().push(unit);
        }
        self.n_units = self.n_units.max(unit.saturating_add(1));
    }
}

impl PostingSource for MemIndex {
    fn n_units(&self) -> u32 {
        self.n_units
    }

    fn postings(&self, f: FeatureId) -> &[UnitId] {
        self.postings.get(&f).map_or(&[], Vec::as_slice)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub unit: UnitId,
    pub overlap: u32,
}

pub struct SearchOutcome {
    /// At most `cap` eligible hits, ranked `(overlap desc, unit asc)`.
    pub hits: Vec<Hit>,
    /// Distinct query features with a non-empty posting list (the ratio denominator).
    pub known_query_features: u32,
    /// Ineligible units met while filling the cap (each ranked above the last hit, or
    /// anywhere when the cap was not filled).
    pub ineligible_skipped: u32,
}

thread_local! {
    static SCORES: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

fn rank(a: &Hit, b: &Hit) -> Ordering {
    b.overlap.cmp(&a.overlap).then(a.unit.cmp(&b.unit))
}

/// Phase I. Keeps units with `overlap > min_ratio * known_query_features` (strict,
/// reference 0.4; paper 0.0) and returns the best `cap` eligible ones. `eligible` is
/// called in rank order, at most once per unit, and never after the cap is filled, so a
/// caller may keep state in it (e.g. clone dedup through a `RefCell`).
pub fn search(
    idx: &dyn PostingSource,
    query: &Bag,
    min_ratio: f64,
    cap: usize,
    eligible: &dyn Fn(UnitId) -> bool,
) -> SearchOutcome {
    let (mut passing, known) = accumulate(idx, query, min_ratio);
    let mut hits = Vec::with_capacity(cap.min(passing.len()));
    let mut ineligible_skipped = 0u32;
    let mut start = 0;
    while start < passing.len() && hits.len() < cap {
        let rest = &mut passing[start..];
        let window = (cap - hits.len()).min(rest.len());
        if window < rest.len() {
            rest.select_nth_unstable_by(window - 1, rank);
        }
        rest[..window].sort_unstable_by(rank);
        for h in &rest[..window] {
            if hits.len() == cap {
                break;
            }
            if eligible(h.unit) {
                hits.push(*h);
            } else {
                ineligible_skipped += 1;
            }
        }
        start += window;
    }
    SearchOutcome {
        hits,
        known_query_features: known,
        ineligible_skipped,
    }
}

/// Dense accumulation; returns the units above the threshold (unordered) and the
/// number of known query features. Every touched slot is reset before returning.
fn accumulate(idx: &dyn PostingSource, query: &Bag, min_ratio: f64) -> (Vec<Hit>, u32) {
    SCORES.with(|cell| {
        let mut scores = cell.borrow_mut();
        let n = idx.n_units() as usize;
        if scores.len() < n {
            scores.resize(n, 0);
        }
        let mut touched: Vec<UnitId> = Vec::new();
        let mut known = 0u32;
        for f in query.distinct() {
            let list = idx.postings(f);
            if list.is_empty() {
                continue;
            }
            known += 1;
            for &u in list {
                let Some(s) = scores.get_mut(u as usize) else {
                    continue;
                };
                if *s == 0 {
                    touched.push(u);
                }
                *s += 1;
            }
        }
        let threshold = min_ratio * f64::from(known);
        let mut passing = Vec::new();
        for &u in &touched {
            let s = &mut scores[u as usize];
            if f64::from(*s) > threshold {
                passing.push(Hit {
                    unit: u,
                    overlap: *s,
                });
            }
            *s = 0;
        }
        (passing, known)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulate_resets_every_touched_slot() {
        let mut idx = MemIndex::default();
        idx.insert(0, &Bag::from_ids([1, 2]));
        idx.insert(3, &Bag::from_ids([2]));
        let (mut passing, known) = accumulate(&idx, &Bag::from_ids([1, 2, 7]), 0.0);
        passing.sort_by_key(|h| h.unit);
        assert_eq!(known, 2);
        assert_eq!(
            passing,
            [
                Hit {
                    unit: 0,
                    overlap: 2
                },
                Hit {
                    unit: 3,
                    overlap: 1
                }
            ]
        );
        SCORES.with(|s| assert!(s.borrow().iter().all(|&x| x == 0)));
    }
}
