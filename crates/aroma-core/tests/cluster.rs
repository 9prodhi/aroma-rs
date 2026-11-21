//! Phase III clustering, intersection and the recommendation gate, pinned to the
//! reference `get_completions3` (similar.py:689-748; note 04 §3.7, note 08 cluster rows).

use aroma_core::cluster::cluster;
use aroma_core::deadline::{Cancelled, Deadline};
use aroma_core::multiset::Bag;
use aroma_core::prune::{Candidate, Pruned};
use aroma_core::spt::{Leaf, Raw, Spt};
use aroma_core::unit::UnitId;
use std::sync::atomic::Ordering;
use std::time::Duration;

// ---------------------------------------------------------------------------------
// Synthetic candidates: only the bags matter to `cluster`.

fn ids(r: impl IntoIterator<Item = u64>) -> Vec<u64> {
    r.into_iter().collect()
}

fn one_leaf_spt() -> Spt {
    Spt::build(Raw::List(vec![
        Raw::Keyword("{".into()),
        Raw::Leaf(Leaf {
            text: "x".into(),
            is_var: false,
            binding: None,
            bytes: 0..0,
            line: 1,
        }),
        Raw::Keyword("}".into()),
    ]))
    .unwrap()
}

/// Candidate with unpruned bag `full` and pruned bag `pruned` (feature ids).
fn cand(unit: UnitId, full: &[u64], pruned: &[u64]) -> Candidate {
    Candidate {
        unit,
        spt: one_leaf_spt(),
        bag: Bag::from_ids(full.iter().copied()),
        pruned: Pruned {
            kept: vec![true],
            bag: Bag::from_ids(pruned.iter().copied()),
        },
        score: 1.0,
    }
}

/// Candidate whose unpruned bag is `pruned` plus the shared block 100..150.
fn with_common(unit: UnitId, pruned: &[u64]) -> Candidate {
    let mut full = pruned.to_vec();
    full.extend(100..150);
    cand(unit, &full, pruned)
}

fn run(cands: &[Candidate], top_n: usize) -> Vec<Vec<usize>> {
    cluster(cands, 0.9, 1.5, top_n, &Deadline::none()).unwrap()
}

#[test]
fn containment_boundary_is_strict_at_0_9() {
    // qlen = |P0| = 10. P0 ∩ P1 = 9 -> 9/10 = 0.9, not > 0.9. P0 ∩ P2 = 10 -> 1.0.
    let p0 = ids(1..=10);
    let mut p1 = ids(1..=9);
    p1.push(99);
    let cands = vec![
        with_common(0, &p0),
        with_common(1, &p1),
        with_common(2, &p0),
    ];
    assert_eq!(run(&cands, 5), vec![vec![0, 2]]);
}

#[test]
fn extension_boundary_is_strict_at_1_5() {
    // iscore = 10 for every pair; tau2 * iscore = 15.
    // F0 ∩ F1 = 15 (not > 15), F0 ∩ F2 = 16, F1 ∩ F2 = 15.
    let p = ids(1..=10);
    let cands = vec![
        cand(0, &ids(1..=16), &p),
        cand(1, &ids(1..=15), &p),
        cand(2, &ids(1..=16), &p),
    ];
    assert_eq!(run(&cands, 5), vec![vec![0, 2]]);
}

#[test]
fn containment_denominator_is_the_first_pruned_bag() {
    // Against the first UNPRUNED bag (110 features) 10/110 would fail tau3.
    let p = ids(1..=10);
    let mut full = p.clone();
    full.extend(200..300);
    let cands = vec![cand(0, &full, &p), cand(1, &full, &p)];
    assert_eq!(run(&cands, 5), vec![vec![0, 1]]);
}

#[test]
fn empty_first_pruned_bag_never_seeds() {
    let p = ids(1..=10);
    let cands = vec![with_common(0, &[]), with_common(1, &p), with_common(2, &p)];
    // Seed 0 has qlen 0 and is skipped; candidate 0 is never a later member either
    // because members only extend to higher indices.
    assert_eq!(run(&cands, 5), vec![vec![1, 2]]);
}

#[test]
fn growth_maximises_unpruned_intersection_earliest_index_wins_ties() {
    // tmp with seed 0: c1 = 26, c2 = 30, c3 = 30 -> c2 (earliest of the tie).
    let p = ids(1..=10);
    let full = |hi: u64| {
        let mut f = p.clone();
        f.extend(100..=hi);
        f
    };
    let cands = vec![
        cand(0, &full(119), &p),
        cand(1, &full(115), &p),
        cand(2, &full(119), &p),
        cand(3, &full(119), &p),
    ];
    // Rounds: (0,2) (1,2) (2,3) | (0,2,3) (1,2,3).
    // Sorted: (0,2,3) (0,2) (1,2,3) (1,2) (2,3).
    // (0,2) J=2/3 vs (0,2,3): dropped. (1,2,3) J=2/4=0.5 vs (0,2,3): equality is kept.
    // (1,2) J=2/3 vs (1,2,3): dropped. (2,3) J=2/3 vs (0,2,3): dropped.
    assert_eq!(run(&cands, 5), vec![vec![0, 2, 3], vec![1, 2, 3]]);
}

#[test]
fn dedup_compares_against_dropped_tuples_too() {
    // P0 = P3 = 1..10, P1 = P2 = 1..20, every unpruned bag adds 100..150.
    // Seed 0 grows 0-1-2-3; seed 1 only reaches (1,2) because (1,2)+3 has
    // containment 10/20 = 0.5; seeds 2 and 3 never grow.
    // Sorted: (0,1,2,3) (0,1,2) (0,1) (1,2).
    // (1,2) vs kept (0,1,2,3): J = 2/4 = 0.5 (kept), but vs DROPPED (0,1,2): J = 2/3.
    let short = ids(1..=10);
    let long = ids(1..=20);
    let cands = vec![
        with_common(0, &short),
        with_common(1, &long),
        with_common(2, &long),
        with_common(3, &short),
    ];
    assert_eq!(run(&cands, 5), vec![vec![0, 1, 2, 3]]);
}

#[test]
fn singletons_are_never_emitted() {
    let p = ids(1..=10);
    // Alone: nothing to extend with.
    assert!(run(&[with_common(0, &p)], 5).is_empty());
    // Identical unpruned and pruned bags: tmp = 10 is not > 15.
    let flat = vec![cand(0, &p, &p), cand(1, &p, &p), cand(2, &p, &p)];
    assert!(run(&flat, 5).is_empty());
    // No candidates at all.
    assert!(run(&[], 5).is_empty());
}

#[test]
fn output_is_capped_at_top_n_in_sorted_order() {
    // Three disjoint pairs (0,1) (2,3) (4,5): cross-pair containment is 0.
    let a = ids(1..=10);
    let b = ids(11..=20);
    let c = ids(21..=30);
    let cands = vec![
        with_common(0, &a),
        with_common(1, &a),
        with_common(2, &b),
        with_common(3, &b),
        with_common(4, &c),
        with_common(5, &c),
    ];
    assert_eq!(run(&cands, 5), vec![vec![0, 1], vec![2, 3], vec![4, 5]]);
    assert_eq!(run(&cands, 2), vec![vec![0, 1], vec![2, 3]]);
    assert_eq!(run(&cands, 1), vec![vec![0, 1]]);
    assert!(run(&cands, 0).is_empty());
}

#[test]
fn zero_top_n_does_no_work_even_after_the_deadline() {
    let p = ids(1..=10);
    let cands = vec![with_common(0, &p), with_common(1, &p)];
    assert_eq!(
        cluster(&cands, 0.9, 1.5, 0, &Deadline::after(Duration::ZERO)),
        Ok(Vec::new())
    );
}

#[test]
fn deadline_and_cancellation_stop_clustering() {
    let p = ids(1..=10);
    let cands = vec![with_common(0, &p), with_common(1, &p)];
    assert_eq!(
        cluster(&cands, 0.9, 1.5, 5, &Deadline::after(Duration::ZERO)),
        Err(Cancelled { stage: "cluster" })
    );
    let d = Deadline::none();
    d.cancel_handle().store(true, Ordering::Relaxed);
    assert_eq!(
        cluster(&cands, 0.9, 1.5, 5, &d),
        Err(Cancelled { stage: "cluster" })
    );
}

// ---------------------------------------------------------------------------------
// Real trees for `intersect`, and the gate.

use aroma_core::cluster::{independent_support, intersect, passes_gate, RecGate};
use aroma_core::features::{featurize, unit_bag, FeatureConfig};
use aroma_core::lang::Lang;
use aroma_core::prune::{rerank, Objective};

fn kw(s: &str) -> Raw {
    Raw::Keyword(s.to_string())
}

fn var(s: &str) -> Raw {
    Raw::Leaf(Leaf {
        text: s.into(),
        is_var: true,
        binding: None,
        bytes: 0..0,
        line: 1,
    })
}

fn name(s: &str) -> Raw {
    Raw::Leaf(Leaf {
        text: s.into(),
        is_var: false,
        binding: None,
        bytes: 0..0,
        line: 1,
    })
}

/// `lhs = recv.m(arg);` (4 leaves)
fn assign(lhs: &str, recv: &str, m: &str, arg: &str) -> Raw {
    Raw::List(vec![
        Raw::List(vec![
            var(lhs),
            kw("="),
            Raw::List(vec![
                var(recv),
                kw("."),
                Raw::List(vec![name(m), kw("("), var(arg), kw(")")]),
            ]),
        ]),
        kw(";"),
    ])
}

/// `recv.m(arg);` (3 leaves)
fn call(recv: &str, m: &str, arg: &str) -> Raw {
    Raw::List(vec![
        Raw::List(vec![
            var(recv),
            kw("."),
            Raw::List(vec![name(m), kw("("), var(arg), kw(")")]),
        ]),
        kw(";"),
    ])
}

fn block(stmts: Vec<Raw>) -> Spt {
    let mut v = vec![kw("{")];
    v.extend(stmts);
    v.push(kw("}"));
    Spt::build(Raw::List(v)).unwrap()
}

fn bag_of(spt: &Spt) -> Bag {
    unit_bag(Lang::Java, &featurize(spt, &FeatureConfig::DEFAULT, None))
}

/// Query: `x = y.open(z); x.read(z);` (7 leaves).
fn query() -> Spt {
    block(vec![assign("x", "y", "open", "z"), call("x", "read", "z")])
}

/// Query statements plus `w = x.parse(z);` (11 leaves).
fn plus_parse(x: &str, y: &str, z: &str, w: &str) -> Spt {
    block(vec![
        assign(x, y, "open", z),
        call(x, "read", z),
        assign(w, x, "parse", z),
    ])
}

/// Query statements plus `w = x.parse(z); w.close(y);` (14 leaves).
fn plus_parse_close(x: &str, y: &str, z: &str, w: &str) -> Spt {
    block(vec![
        assign(x, y, "open", z),
        call(x, "read", z),
        assign(w, x, "parse", z),
        call(w, "close", y),
    ])
}

/// Rerank `spts` (as units 0, 1, ...) against the query, as the engine does.
fn reranked(spts: Vec<Spt>) -> (Bag, Vec<Candidate>) {
    let q = bag_of(&query());
    let hits: Vec<(UnitId, Spt, Bag)> = spts
        .into_iter()
        .enumerate()
        .map(|(i, s)| {
            let b = bag_of(&s);
            (i as UnitId, s, b)
        })
        .collect();
    let (cands, cancelled) = rerank(
        &q,
        hits,
        Lang::Java,
        &FeatureConfig::DEFAULT,
        Objective::Jaccard,
        0.65,
        &Deadline::none(),
    );
    assert!(cancelled.is_none());
    (q, cands)
}

#[test]
fn intersect_keeps_the_extension_shared_by_every_member() {
    let (q, cands) = reranked(vec![
        plus_parse_close("x", "y", "z", "w"),
        plus_parse_close("a", "b", "c", "d"),
        plus_parse("p", "q", "r", "s"),
    ]);
    assert_eq!(cands.iter().map(|c| c.unit).collect::<Vec<_>>(), [0, 1, 2]);
    for c in &cands {
        assert_eq!(c.pruned.kept.iter().filter(|&&k| k).count(), 7);
    }
    let d = Deadline::none();
    let cfg = FeatureConfig::DEFAULT;
    // Both members carry parse and close: the whole first member survives.
    let both = intersect(&q, &[0, 1], &cands, Lang::Java, &cfg, &d).unwrap();
    assert_eq!(both, vec![true; 14]);
    // The third member has no close statement: its 3 leaves are dropped.
    let shared = intersect(&q, &[0, 2], &cands, Lang::Java, &cfg, &d).unwrap();
    let mut expected = vec![true; 11];
    expected.extend([false; 3]);
    assert_eq!(shared, expected);
    // Sequential: the mask after member 1 is pruned again towards member 2.
    assert_eq!(
        intersect(&q, &[0, 1, 2], &cands, Lang::Java, &cfg, &d).unwrap(),
        expected
    );
}

#[test]
fn intersect_of_short_tuples() {
    let (q, cands) = reranked(vec![
        plus_parse_close("x", "y", "z", "w"),
        plus_parse_close("a", "b", "c", "d"),
    ]);
    let d = Deadline::none();
    let cfg = FeatureConfig::DEFAULT;
    assert_eq!(
        intersect(&q, &[1], &cands, Lang::Java, &cfg, &d).unwrap(),
        cands[1].pruned.kept
    );
    assert_eq!(
        intersect(&q, &[], &cands, Lang::Java, &cfg, &d).unwrap(),
        Vec::<bool>::new()
    );
}

#[test]
fn intersect_honours_the_deadline() {
    let (q, cands) = reranked(vec![
        plus_parse_close("x", "y", "z", "w"),
        plus_parse_close("a", "b", "c", "d"),
    ]);
    assert_eq!(
        intersect(
            &q,
            &[0, 1],
            &cands,
            Lang::Java,
            &FeatureConfig::DEFAULT,
            &Deadline::after(Duration::ZERO)
        ),
        Err(Cancelled { stage: "intersect" })
    );
}

#[test]
fn clustered_real_candidates_pass_the_gate() {
    let (q, cands) = reranked(vec![
        plus_parse_close("x", "y", "z", "w"),
        plus_parse_close("a", "b", "c", "d"),
        plus_parse_close("p", "q", "r", "s"),
    ]);
    let tuples = run(&cands, 5);
    assert_eq!(tuples, vec![vec![0, 1, 2]]);
    let kept = intersect(
        &q,
        &tuples[0],
        &cands,
        Lang::Java,
        &FeatureConfig::DEFAULT,
        &Deadline::none(),
    )
    .unwrap();
    let support = independent_support(&tuples[0], &cands, &|u| u64::from(u) + 1);
    assert_eq!(support, 3);
    // 14 kept leaves - 7 query leaves = 7 added <= 2 * 7.
    assert!(passes_gate(&RecGate::DEFAULT, support, &kept, 7));
}

// ---------------------------------------------------------------------------------
// Gate.

#[test]
fn independent_support_counts_distinct_body_hashes() {
    let p = ids(1..=3);
    let cands: Vec<Candidate> = (0..4).map(|u| with_common(u, &p)).collect();
    let all = [0, 1, 2, 3];
    assert_eq!(independent_support(&all, &cands, &|u| u64::from(u)), 4);
    assert_eq!(independent_support(&all, &cands, &|u| u64::from(u / 2)), 2);
    assert_eq!(independent_support(&all, &cands, &|_| 7), 1);
    assert_eq!(independent_support(&[], &cands, &|_| 7), 0);
}

#[test]
fn gate_boundaries() {
    let g = RecGate::DEFAULT;
    let kept = |n: usize| vec![true; n];
    // Support: exactly min_independent passes.
    assert!(passes_gate(&g, 3, &kept(8), 4));
    assert!(!passes_gate(&g, 2, &kept(8), 4));
    // Extension: 1 ..= 2 * query_leaves added leaves.
    assert!(passes_gate(&g, 3, &kept(5), 4)); // added 1
    assert!(passes_gate(&g, 3, &kept(12), 4)); // added 8 == 2 * 4
    assert!(!passes_gate(&g, 3, &kept(13), 4)); // added 9
    assert!(!passes_gate(&g, 3, &kept(4), 4)); // added 0
    assert!(!passes_gate(&g, 3, &kept(2), 4)); // fewer kept than the query

    // Only kept leaves count: 3 kept, added 1.
    assert!(passes_gate(&g, 3, &[true, false, true, false, true], 2));

    // A query without leaves never passes.
    assert!(!passes_gate(&g, 3, &kept(3), 0));
    // A custom gate.
    let strict = RecGate {
        min_independent: 1,
        max_extension_ratio: 0.5,
    };
    assert!(passes_gate(&strict, 1, &kept(6), 4)); // added 2 == 0.5 * 4
    assert!(!passes_gate(&strict, 1, &kept(7), 4)); // added 3
    assert!(!passes_gate(&strict, 0, &kept(6), 4));
}
