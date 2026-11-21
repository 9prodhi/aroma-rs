//! Multiset laws and Phase I search, checked against naive reference implementations.

use aroma_core::multiset::{jaccard, Bag, FeatureId};
use proptest::prelude::*;
use std::collections::BTreeSet;

fn arb_bag() -> impl Strategy<Value = Bag> {
    prop::collection::vec(0u64..24, 0..40).prop_map(Bag::from_ids)
}

fn keys(a: &Bag, b: &Bag) -> BTreeSet<FeatureId> {
    a.distinct().chain(b.distinct()).collect()
}

fn bag(ids: &[u64]) -> Bag {
    Bag::from_ids(ids.iter().copied())
}

proptest! {
    #[test]
    fn from_ids_is_sorted_with_positive_counts(ids in prop::collection::vec(any::<u64>(), 0..50)) {
        let bag = Bag::from_ids(ids.iter().copied());
        let pairs: Vec<(FeatureId, u32)> = bag.iter().collect();
        prop_assert!(pairs.windows(2).all(|w| w[0].0 < w[1].0));
        prop_assert!(pairs.iter().all(|&(_, c)| c > 0));
        prop_assert_eq!(bag.len(), ids.len() as u64);
        prop_assert_eq!(bag.is_empty(), ids.is_empty());
        prop_assert_eq!(bag.distinct().collect::<Vec<_>>(), pairs.iter().map(|p| p.0).collect::<Vec<_>>());
        for &id in &ids {
            prop_assert_eq!(bag.get(id), ids.iter().filter(|&&x| x == id).count() as u32);
        }
    }

    #[test]
    fn min_max_sum_laws(a in arb_bag(), b in arb_bag(), c in arb_bag()) {
        let (i, u, s) = (a.intersect(&b), a.union(&b), a.sum(&b));
        for f in keys(&a, &b) {
            prop_assert_eq!(i.get(f), a.get(f).min(b.get(f)));
            prop_assert_eq!(u.get(f), a.get(f).max(b.get(f)));
            prop_assert_eq!(s.get(f), a.get(f) + b.get(f));
        }
        prop_assert!(i.iter().all(|(_, n)| n > 0) && u.iter().all(|(_, n)| n > 0));
        prop_assert_eq!(&i, &b.intersect(&a));
        prop_assert_eq!(&u, &b.union(&a));
        prop_assert_eq!(&s, &b.sum(&a));
        prop_assert_eq!(a.intersect(&b).intersect(&c), a.intersect(&b.intersect(&c)));
        prop_assert_eq!(a.union(&b).union(&c), a.union(&b.union(&c)));
        prop_assert_eq!(a.sum(&b).sum(&c), a.sum(&b.sum(&c)));
        prop_assert_eq!(a.intersect(&a.union(&b)), a.clone());
        prop_assert_eq!(a.union(&a.intersect(&b)), a.clone());
        prop_assert_eq!(a.intersect(&Bag::default()), Bag::default());
        prop_assert_eq!(a.union(&Bag::default()), a.clone());
        prop_assert_eq!(a.sum(&Bag::default()), a.clone());
        prop_assert_eq!(a.intersection_size(&b), i.len());
        prop_assert_eq!(a.union_size(&b), u.len());
        prop_assert_eq!(s.len(), a.len() + b.len());
        prop_assert_eq!(a.intersection_size(&b) + a.union_size(&b), a.len() + b.len());
    }

    #[test]
    fn jaccard_is_symmetric_and_bounded(a in arb_bag(), b in arb_bag()) {
        let j = jaccard(&a, &b);
        prop_assert_eq!(j, jaccard(&b, &a));
        prop_assert!((0.0..=1.0).contains(&j));
        if a.is_empty() && b.is_empty() {
            prop_assert_eq!(j, 0.0);
        } else {
            prop_assert_eq!(j, a.intersection_size(&b) as f64 / a.union_size(&b) as f64);
        }
        if !a.is_empty() {
            prop_assert_eq!(jaccard(&a, &a), 1.0);
        }
    }
}

#[test]
fn jaccard_of_empty_bags_is_zero() {
    assert_eq!(jaccard(&Bag::default(), &Bag::default()), 0.0);
    assert_eq!(jaccard(&bag(&[1]), &Bag::default()), 0.0);
    assert_eq!(jaccard(&bag(&[1, 1, 2]), &bag(&[1, 2, 2])), 0.5);
}

#[test]
fn repeated_sums_double_counts() {
    let big = Bag::from_ids(std::iter::repeat_n(5, 3));
    let mut acc = big.clone();
    for _ in 0..4 {
        acc = acc.sum(&acc);
    }
    assert_eq!(acc.get(5), 48);
    assert_eq!(acc.union(&big).get(5), 48);
    assert_eq!(acc.intersect(&big).get(5), 3);
}

// ---- Phase I search (Step group 2) ----

use aroma_core::index::{search, Hit, MemIndex, PostingSource};
use aroma_core::unit::UnitId;
use std::cell::RefCell;

/// Naive Phase I: per-unit set overlap, strict ratio filter, rank, then fill the cap
/// in rank order skipping ineligible units.
fn naive_search(
    units: &[Bag],
    query: &Bag,
    min_ratio: f64,
    cap: usize,
    eligible: &dyn Fn(UnitId) -> bool,
) -> (Vec<Hit>, u32, u32) {
    let indexed: BTreeSet<FeatureId> = units.iter().flat_map(|u| u.distinct()).collect();
    let q: BTreeSet<FeatureId> = query.distinct().collect();
    let known = q.intersection(&indexed).count() as u32;
    let mut ranked: Vec<Hit> = units
        .iter()
        .enumerate()
        .map(|(u, b)| Hit {
            unit: u as UnitId,
            overlap: b.distinct().filter(|f| q.contains(f)).count() as u32,
        })
        .filter(|h| h.overlap > 0 && f64::from(h.overlap) > min_ratio * f64::from(known))
        .collect();
    ranked.sort_by(|a, b| b.overlap.cmp(&a.overlap).then(a.unit.cmp(&b.unit)));
    let (mut hits, mut skipped) = (Vec::new(), 0);
    for h in ranked {
        if hits.len() == cap {
            break;
        }
        if eligible(h.unit) {
            hits.push(h);
        } else {
            skipped += 1;
        }
    }
    (hits, known, skipped)
}

fn mem_index(units: &[Bag]) -> MemIndex {
    let mut idx = MemIndex::default();
    for (u, b) in units.iter().enumerate() {
        idx.insert(u as UnitId, b);
    }
    idx
}

proptest! {
    #[test]
    fn search_matches_naive_reference(
        units in prop::collection::vec(arb_bag(), 0..30),
        query in arb_bag(),
        ratio_tenths in 0u32..10,
        cap in 0usize..12,
        banned in prop::collection::btree_set(0u32..30, 0..8),
    ) {
        let idx = mem_index(&units);
        let min_ratio = f64::from(ratio_tenths) / 10.0;
        let eligible = |u: UnitId| !banned.contains(&u);
        let calls = RefCell::new(Vec::new());
        let recording = |u: UnitId| {
            calls.borrow_mut().push(u);
            eligible(u)
        };
        let out = search(&idx, &query, min_ratio, cap, &recording);
        let (hits, known, skipped) = naive_search(&units, &query, min_ratio, cap, &eligible);
        prop_assert_eq!(&out.hits, &hits);
        prop_assert_eq!(out.known_query_features, known);
        prop_assert_eq!(out.ineligible_skipped, skipped);
        prop_assert!(out.hits.len() <= cap);
        // eligibility is asked in rank order, once per unit, only until the cap fills
        let calls = calls.into_inner();
        prop_assert_eq!(calls.len(), hits.len() + skipped as usize);
        let mut seen = BTreeSet::new();
        prop_assert!(calls.iter().all(|u| seen.insert(*u)));
        // a second query on the same thread sees a clean score buffer
        let again = search(&idx, &query, min_ratio, cap, &eligible);
        prop_assert_eq!(again.hits, hits);
    }

    #[test]
    fn overlap_ignores_duplicate_counts(units in prop::collection::vec(arb_bag(), 1..10), query in arb_bag()) {
        let idx = mem_index(&units);
        let doubled = query.sum(&query);
        let all = |_: UnitId| true;
        let a = search(&idx, &query, 0.0, 100, &all);
        let b = search(&idx, &doubled, 0.0, 100, &all);
        prop_assert_eq!(a.hits, b.hits);
        prop_assert_eq!(a.known_query_features, b.known_query_features);
    }
}

#[test]
fn ratio_boundary_is_strict() {
    // 5 known query features: threshold 0.4 * 5 = 2.0, so overlap 2 is out, 3 is in.
    let units = [bag(&[1, 2]), bag(&[1, 2, 3]), bag(&[4, 5])];
    let idx = mem_index(&units);
    let out = search(&idx, &bag(&[1, 2, 3, 4, 5, 99]), 0.4, 100, &|_| true);
    assert_eq!(
        out.known_query_features, 5,
        "feature 99 is unknown to the index"
    );
    assert_eq!(
        out.hits,
        [Hit {
            unit: 1,
            overlap: 3
        }]
    );
    assert_eq!(out.ineligible_skipped, 0);
}

#[test]
fn ties_break_by_ascending_unit_and_cap_applies() {
    let units = [bag(&[1]), bag(&[1, 2]), bag(&[2]), bag(&[1, 2]), bag(&[1])];
    let idx = mem_index(&units);
    let out = search(&idx, &bag(&[1, 2]), 0.0, 3, &|_| true);
    assert_eq!(
        out.hits,
        [
            Hit {
                unit: 1,
                overlap: 2
            },
            Hit {
                unit: 3,
                overlap: 2
            },
            Hit {
                unit: 0,
                overlap: 1
            }
        ]
    );
}

#[test]
fn ineligible_units_never_consume_the_cap_and_are_counted() {
    let units = [
        bag(&[1, 2, 3]),
        bag(&[1, 2, 3]),
        bag(&[1, 2]),
        bag(&[1]),
        bag(&[1]),
    ];
    let idx = mem_index(&units);
    let calls = RefCell::new(Vec::new());
    let eligible = |u: UnitId| {
        calls.borrow_mut().push(u);
        u != 0 && u != 2
    };
    let out = search(&idx, &bag(&[1, 2, 3]), 0.0, 2, &eligible);
    assert_eq!(
        out.hits,
        [
            Hit {
                unit: 1,
                overlap: 3
            },
            Hit {
                unit: 3,
                overlap: 1
            }
        ]
    );
    assert_eq!(out.ineligible_skipped, 2);
    assert_eq!(
        calls.into_inner(),
        [0, 1, 2, 3],
        "unit 4 is never asked: the cap is full"
    );
}

#[test]
fn backfill_reaches_past_the_first_window() {
    // cap 2, the four best units are ineligible: hits come from ranks 5 and 6.
    let units: Vec<Bag> = (0..8u64)
        .map(|u| bag(&(0..8 - u).collect::<Vec<_>>()))
        .collect();
    let idx = mem_index(&units);
    let out = search(&idx, &bag(&(0..8).collect::<Vec<_>>()), 0.0, 2, &|u| u >= 4);
    assert_eq!(
        out.hits,
        [
            Hit {
                unit: 4,
                overlap: 4
            },
            Hit {
                unit: 5,
                overlap: 3
            }
        ]
    );
    assert_eq!(out.ineligible_skipped, 4);
}

#[test]
fn zero_cap_asks_nobody() {
    let idx = mem_index(&[bag(&[1])]);
    let out = search(&idx, &bag(&[1]), 0.0, 0, &|_| {
        panic!("eligible must not be called")
    });
    assert!(out.hits.is_empty());
    assert_eq!(out.known_query_features, 1);
    assert_eq!(out.ineligible_skipped, 0);
}

#[test]
fn empty_query_and_empty_index() {
    let idx = MemIndex::default();
    assert_eq!(idx.n_units(), 0);
    assert!(idx.postings(7).is_empty());
    let out = search(&idx, &bag(&[1, 2]), 0.4, 100, &|_| true);
    assert!(out.hits.is_empty());
    assert_eq!(out.known_query_features, 0);
    let idx = mem_index(&[bag(&[1])]);
    let out = search(&idx, &Bag::default(), 0.0, 100, &|_| true);
    assert!(out.hits.is_empty());
    assert_eq!(out.known_query_features, 0);
}

#[test]
fn mem_index_postings_are_sorted_and_distinct() {
    let idx = mem_index(&[bag(&[5, 5, 6]), bag(&[5]), bag(&[]), bag(&[6, 6])]);
    assert_eq!(idx.n_units(), 4);
    assert_eq!(idx.postings(5), [0, 1]);
    assert_eq!(idx.postings(6), [0, 3]);
}
