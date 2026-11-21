//! Greedy prune (Algorithm 1) and rerank, checked against a naive eager greedy written
//! with `Bag` operations only.

use aroma_core::deadline::{Cancelled, Deadline};
use aroma_core::multiset::{jaccard, Bag};
use aroma_core::prune::{select_leaves, Objective};
use proptest::prelude::*;
use std::sync::atomic::Ordering;

fn arb_bag() -> impl Strategy<Value = Bag> {
    prop::collection::vec(0u64..12, 0..8).prop_map(Bag::from_ids)
}

fn objective(obj: Objective, target: &Bag, acc: &Bag) -> f64 {
    match obj {
        Objective::Jaccard => jaccard(target, acc),
        Objective::IntersectionSize => target.intersection_size(acc) as f64,
    }
}

/// Eager greedy straight from `similar.py:prune_last_jd`: rebuild the accumulator for
/// every trial, strict `>`, earliest leaf wins ties. Returns the mask and, per accepted
/// step, the objective before and after.
fn naive_select(
    targets: &[&Bag],
    leaf_bags: &[Bag],
    candidates: &[bool],
    obj: Objective,
) -> (Vec<bool>, Vec<(f64, f64)>) {
    let mut kept = vec![false; leaf_bags.len()];
    let mut acc = Bag::default();
    let mut steps = Vec::new();
    for target in targets {
        let mut current = objective(obj, target, &acc);
        loop {
            let mut best: Option<(usize, f64)> = None;
            for (i, lb) in leaf_bags.iter().enumerate() {
                if kept[i] || !candidates.get(i).copied().unwrap_or(false) {
                    continue;
                }
                let s = objective(obj, target, &acc.sum(lb));
                if s > best.map_or(current, |b| b.1) {
                    best = Some((i, s));
                }
            }
            let Some((i, s)) = best else { break };
            steps.push((current, s));
            kept[i] = true;
            acc = acc.sum(&leaf_bags[i]);
            current = s;
        }
    }
    (kept, steps)
}

fn cancelled_deadline() -> Deadline {
    let d = Deadline::none();
    d.cancel_handle().store(true, Ordering::SeqCst);
    d
}

fn bag(ids: &[u64]) -> Bag {
    Bag::from_ids(ids.iter().copied())
}

proptest! {
    #[test]
    fn select_leaves_equals_naive_eager(
        leaf_bags in prop::collection::vec(arb_bag(), 0..12),
        targets in prop::collection::vec(arb_bag(), 1..3),
        candidates in prop::collection::vec(any::<bool>(), 0..14),
    ) {
        let refs: Vec<&Bag> = targets.iter().collect();
        for obj in [Objective::Jaccard, Objective::IntersectionSize] {
            let got = select_leaves(&refs, &leaf_bags, &candidates, obj, &Deadline::none()).unwrap();
            let (want, steps) = naive_select(&refs, &leaf_bags, &candidates, obj);
            // lazy greedy (IntersectionSize) and eager scan (Jaccard) both equal the naive eager
            prop_assert_eq!(&got, &want, "{:?}", obj);
            // kept ⊆ candidates
            for (i, &k) in got.iter().enumerate() {
                prop_assert!(!k || candidates.get(i).copied().unwrap_or(false));
            }
            // every accepted step strictly increases the objective
            prop_assert!(steps.iter().all(|(before, after)| after > before), "{:?}", steps);
        }
    }

    #[test]
    fn single_target_result_is_locally_optimal(
        leaf_bags in prop::collection::vec(arb_bag(), 0..12),
        target in arb_bag(),
    ) {
        let all = vec![true; leaf_bags.len()];
        for obj in [Objective::Jaccard, Objective::IntersectionSize] {
            let kept = select_leaves(&[&target], &leaf_bags, &all, obj, &Deadline::none()).unwrap();
            let acc = leaf_bags
                .iter()
                .zip(&kept)
                .filter(|(_, &k)| k)
                .fold(Bag::default(), |a, (b, _)| a.sum(b));
            let current = objective(obj, &target, &acc);
            for (i, lb) in leaf_bags.iter().enumerate() {
                if !kept[i] {
                    prop_assert!(objective(obj, &target, &acc.sum(lb)) <= current);
                }
            }
        }
    }
}

#[test]
fn ties_go_to_the_earliest_leaf() {
    let leaves = [bag(&[1]), bag(&[2]), bag(&[1])];
    for obj in [Objective::Jaccard, Objective::IntersectionSize] {
        let kept =
            select_leaves(&[&bag(&[1])], &leaves, &[true; 3], obj, &Deadline::none()).unwrap();
        assert_eq!(kept, [true, false, false], "{obj:?}");
    }
}

#[test]
fn jaccard_and_intersection_disagree_on_noisy_leaves() {
    // Leaf 0 covers the target but drags in noise; Jaccard refuses it, SimScore takes it.
    let target = bag(&[1, 2]);
    let leaves = [bag(&[1, 2, 3, 4, 5, 6]), bag(&[1])];
    let d = Deadline::none();
    let j = select_leaves(&[&target], &leaves, &[true, true], Objective::Jaccard, &d).unwrap();
    let s = select_leaves(
        &[&target],
        &leaves,
        &[true, true],
        Objective::IntersectionSize,
        &d,
    )
    .unwrap();
    assert_eq!(j, [false, true]);
    assert_eq!(s, [true, false]);
}

#[test]
fn zero_gain_leaves_are_never_selected() {
    let leaves = [bag(&[7]), bag(&[8, 8]), bag(&[])];
    for obj in [Objective::Jaccard, Objective::IntersectionSize] {
        let kept = select_leaves(
            &[&bag(&[1, 2])],
            &leaves,
            &[true; 3],
            obj,
            &Deadline::none(),
        )
        .unwrap();
        assert_eq!(kept, [false, false, false], "{obj:?}");
    }
}

#[test]
fn sequential_targets_share_the_accumulator() {
    // The second target only gains from leaf 2; leaf 0 stays selected from the first pass.
    let leaves = [bag(&[1]), bag(&[2]), bag(&[3])];
    let kept = select_leaves(
        &[&bag(&[1]), &bag(&[1, 3])],
        &leaves,
        &[true; 3],
        Objective::IntersectionSize,
        &Deadline::none(),
    )
    .unwrap();
    assert_eq!(kept, [true, false, true]);
}

#[test]
fn cancelled_deadline_stops_selection() {
    let d = cancelled_deadline();
    for obj in [Objective::Jaccard, Objective::IntersectionSize] {
        assert_eq!(
            select_leaves(&[&bag(&[1])], &[bag(&[1])], &[true], obj, &d),
            Err(Cancelled { stage: "prune" })
        );
    }
}

#[test]
fn expired_wall_clock_deadline_cancels() {
    let d = Deadline::after(std::time::Duration::ZERO);
    std::thread::sleep(std::time::Duration::from_millis(2));
    let r = select_leaves(&[&bag(&[1])], &[bag(&[1])], &[true], Objective::Jaccard, &d);
    assert_eq!(r, Err(Cancelled { stage: "prune" }));
}

// ---- prune and rerank (Step group 2) ----

use aroma_core::features::{featurize, unit_bag, FeatureConfig};
use aroma_core::lang::Lang;
use aroma_core::prune::{prune, rerank};
use aroma_core::spt::{Leaf, Raw, Spt, SymbolId};
use aroma_core::unit::UnitId;

fn arb_leaf() -> impl Strategy<Value = Raw> {
    (
        prop::sample::select(vec!["a", "b", "c", "Foo", "bar"]),
        any::<bool>(),
        prop::option::of(0u32..3),
    )
        .prop_map(|(text, is_var, binding)| {
            Raw::Leaf(Leaf {
                text: text.into(),
                is_var,
                binding: binding.map(SymbolId),
                bytes: 0..1,
                line: 1,
            })
        })
}

fn arb_spt() -> impl Strategy<Value = Spt> {
    let atom = prop_oneof![
        3 => arb_leaf(),
        2 => prop::sample::select(vec![".", "(", ")", "=", ";", "if"]).prop_map(|k| Raw::Keyword(k.into())),
    ];
    let tree = atom.prop_recursive(4, 48, 5, |inner| {
        prop::collection::vec(inner, 0..5).prop_map(Raw::List)
    });
    prop::collection::vec(tree, 1..6)
        .prop_filter_map("tree without tokens", |v| Spt::build(Raw::List(v)))
}

fn bag_of(spt: &Spt, cfg: &FeatureConfig, mask: Option<&[bool]>) -> Bag {
    unit_bag(Lang::Java, &featurize(spt, cfg, mask))
}

/// Rerank score as specified: Jaccard, or SimScore / |F(q)| (0 for an empty query).
fn expected_score(q: &Bag, pruned: &Bag, obj: Objective) -> f64 {
    match obj {
        Objective::Jaccard => jaccard(q, pruned),
        Objective::IntersectionSize if q.is_empty() => 0.0,
        Objective::IntersectionSize => q.intersection_size(pruned) as f64 / q.len() as f64,
    }
}

/// `a;` with `a` a non-variable leaf: features `a` and `#;1>a`.
fn one_leaf_spt() -> Spt {
    Spt::build(Raw::List(vec![
        Raw::Leaf(Leaf {
            text: "a".into(),
            is_var: false,
            binding: None,
            bytes: 0..1,
            line: 1,
        }),
        Raw::Keyword(";".into()),
    ]))
    .expect("has tokens")
}

proptest! {
    #[test]
    fn self_prune_keeps_every_leaf(spt in arb_spt()) {
        for cfg in [FeatureConfig::DEFAULT, FeatureConfig::REFERENCE] {
            let q = bag_of(&spt, &cfg, None);
            for obj in [Objective::Jaccard, Objective::IntersectionSize] {
                let p = prune(&[&q], &spt, None, Lang::Java, &cfg, obj, &Deadline::none()).unwrap();
                prop_assert!(p.kept.iter().all(|&k| k), "{:?}", obj);
                prop_assert_eq!(&p.bag, &q);
                if !q.is_empty() {
                    prop_assert_eq!(jaccard(&q, &p.bag), 1.0);
                }
            }
        }
    }

    #[test]
    fn prune_respects_mask_and_bounds(
        query in arb_spt(),
        cand in arb_spt(),
        mask in prop::collection::vec(any::<bool>(), 0..40),
    ) {
        let cfg = FeatureConfig::DEFAULT;
        let q = bag_of(&query, &cfg, None);
        for obj in [Objective::Jaccard, Objective::IntersectionSize] {
            let p = prune(&[&q], &cand, Some(&mask), Lang::Java, &cfg, obj, &Deadline::none()).unwrap();
            prop_assert_eq!(p.kept.len(), cand.leaves.len());
            for (i, &k) in p.kept.iter().enumerate() {
                prop_assert!(!k || mask.get(i).copied().unwrap_or(false));
            }
            prop_assert!(q.intersection_size(&p.bag) <= q.len());
            // the returned bag is re-extracted from the pruned tree
            prop_assert_eq!(&p.bag, &bag_of(&cand, &cfg, Some(&p.kept)));
        }
    }

    #[test]
    fn rerank_is_filtered_descending_and_stable(
        query in arb_spt(),
        cands in prop::collection::vec(arb_spt(), 0..8),
        min_tenths in 0u32..8,
    ) {
        let cfg = FeatureConfig::DEFAULT;
        let q = bag_of(&query, &cfg, None);
        let min_pruned = f64::from(min_tenths) / 10.0;
        for obj in [Objective::Jaccard, Objective::IntersectionSize] {
            let hits: Vec<(UnitId, Spt, Bag)> = cands
                .iter()
                .enumerate()
                .map(|(i, s)| (i as UnitId, s.clone(), bag_of(s, &cfg, None)))
                .collect();
            let (out, cancelled) = rerank(&q, hits, Lang::Java, &cfg, obj, min_pruned, &Deadline::none());
            prop_assert!(cancelled.is_none());
            for c in &out {
                prop_assert!(c.score > min_pruned);
                prop_assert_eq!(c.score, expected_score(&q, &c.pruned.bag, obj));
                prop_assert_eq!(&c.bag, &bag_of(&c.spt, &cfg, None));
            }
            for w in out.windows(2) {
                prop_assert!(w[0].score >= w[1].score);
                if w[0].score == w[1].score {
                    prop_assert!(w[0].unit < w[1].unit, "equal scores keep Phase I order");
                }
            }
            // every candidate scoring above the bar is present
            let expected = cands
                .iter()
                .filter(|s| {
                    let p = prune(&[&q], s, None, Lang::Java, &cfg, obj, &Deadline::none()).unwrap();
                    expected_score(&q, &p.bag, obj) > min_pruned
                })
                .count();
            prop_assert_eq!(out.len(), expected);
        }
    }
}

#[test]
fn cancelled_deadline_stops_prune() {
    let spt = one_leaf_spt();
    let cfg = FeatureConfig::DEFAULT;
    let q = bag_of(&spt, &cfg, None);
    let r = prune(
        &[&q],
        &spt,
        None,
        Lang::Java,
        &cfg,
        Objective::Jaccard,
        &cancelled_deadline(),
    );
    assert_eq!(r.unwrap_err(), Cancelled { stage: "prune" });
}

#[test]
fn cancelled_deadline_returns_partial_rerank() {
    let spt = one_leaf_spt();
    let cfg = FeatureConfig::DEFAULT;
    let q = bag_of(&spt, &cfg, None);
    let hits = vec![(0, spt.clone(), q.clone()), (1, spt.clone(), q.clone())];
    let (out, cancelled) = rerank(
        &q,
        hits.clone(),
        Lang::Java,
        &cfg,
        Objective::Jaccard,
        0.65,
        &cancelled_deadline(),
    );
    assert!(out.is_empty());
    assert_eq!(cancelled, Some(Cancelled { stage: "rerank" }));
    let (out, cancelled) = rerank(
        &q,
        hits,
        Lang::Java,
        &cfg,
        Objective::Jaccard,
        0.65,
        &Deadline::none(),
    );
    assert_eq!(cancelled, None);
    assert_eq!(
        out.iter().map(|c| (c.unit, c.score)).collect::<Vec<_>>(),
        [(0, 1.0), (1, 1.0)]
    );
}

#[test]
fn rerank_threshold_is_strict() {
    // The candidate has 2 distinct features; the query has them plus 2 unknown ones,
    // so the best pruned Jaccard is exactly 2 / 4 = 0.5.
    let spt = one_leaf_spt();
    let cfg = FeatureConfig::DEFAULT;
    let cand_bag = bag_of(&spt, &cfg, None);
    assert_eq!(cand_bag.len(), 2);
    let mut q_ids: Vec<u64> = cand_bag.distinct().collect();
    q_ids.extend([u64::MAX, u64::MAX - 1]);
    let q = Bag::from_ids(q_ids);
    let hits = vec![(7, spt.clone(), cand_bag.clone())];
    let d = Deadline::none();
    let (out, _) = rerank(
        &q,
        hits.clone(),
        Lang::Java,
        &cfg,
        Objective::Jaccard,
        0.5,
        &d,
    );
    assert!(out.is_empty(), "0.5 is not > 0.5");
    let (out, _) = rerank(&q, hits, Lang::Java, &cfg, Objective::Jaccard, 0.49, &d);
    assert_eq!(out.len(), 1);
    assert_eq!((out[0].unit, out[0].score), (7, 0.5));
    assert!(out[0].pruned.kept.iter().all(|&k| k));
}

#[test]
fn paper_rerank_normalises_by_query_size() {
    // Same setup, IntersectionSize objective: SimScore 2 over |F(q)| 4 = 0.5.
    let spt = one_leaf_spt();
    let cfg = FeatureConfig::DEFAULT;
    let cand_bag = bag_of(&spt, &cfg, None);
    let mut q_ids: Vec<u64> = cand_bag.distinct().collect();
    q_ids.extend([u64::MAX, u64::MAX - 1]);
    let q = Bag::from_ids(q_ids);
    let hits = vec![(3, spt, cand_bag)];
    let (out, _) = rerank(
        &q,
        hits,
        Lang::Java,
        &cfg,
        Objective::IntersectionSize,
        0.4,
        &Deadline::none(),
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].score, 0.5);
}

#[test]
fn prune_keeps_the_part_matching_the_query() {
    // T00's in-module case (prune.rs is replaced in Step group 1): query `{ a.open(b); }`
    // against `{ a.open(b); c.flush(d); }` keeps exactly the matching statement.
    let kw = |k: &str| Raw::Keyword(k.into());
    let leaf = |t: &str, is_var: bool| {
        Raw::Leaf(Leaf {
            text: t.into(),
            is_var,
            binding: None,
            bytes: 0..1,
            line: 1,
        })
    };
    let call = |recv: &str, m: &str, arg: &str| {
        Raw::List(vec![
            Raw::List(vec![
                leaf(recv, true),
                kw("."),
                Raw::List(vec![leaf(m, false), kw("("), leaf(arg, true), kw(")")]),
            ]),
            kw(";"),
        ])
    };
    let block = |stmts: Vec<Raw>| {
        let mut v = vec![kw("{")];
        v.extend(stmts);
        v.push(kw("}"));
        Spt::build(Raw::List(v)).expect("has tokens")
    };
    let cfg = FeatureConfig::DEFAULT;
    let q = bag_of(&block(vec![call("a", "open", "b")]), &cfg, None);
    let cand = block(vec![call("a", "open", "b"), call("c", "flush", "d")]);
    let p = prune(
        &[&q],
        &cand,
        None,
        Lang::Java,
        &cfg,
        Objective::Jaccard,
        &Deadline::none(),
    )
    .unwrap();
    assert_eq!(p.kept, [true, true, true, false, false, false]);
    assert_eq!(jaccard(&q, &p.bag), 1.0);
}
