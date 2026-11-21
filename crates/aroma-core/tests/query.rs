//! Query engine (D22/D23): eligibility with backfill, clone dedup before the cap,
//! stale units, deadlines with partial results, gated recommendations, determinism, and
//! the D30 split: no score floor on `similar`, `score > min_pruned` for clustering.
//! Uses `MemIndex` and a Vec-backed `UnitStore`.

use aroma_core::cluster::RecGate;
use aroma_core::features::{featurize, unit_bag, FeatureConfig};
use aroma_core::index::MemIndex;
use aroma_core::lang::Lang;
use aroma_core::lexical::TermRule;
use aroma_core::profile::Profile;
use aroma_core::query::{run_query, QueryRequest, QueryResult};
use aroma_core::spt::{Leaf, Raw, Spt};
use aroma_core::store::StoreError;
use aroma_core::unit::{UnitId, UnitKind, UnitRecord, UnitStore, MAX_PRUNE_LEAVES};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

struct VecStore {
    units: Vec<(UnitRecord, Spt)>,
    files: Vec<PathBuf>,
    /// Units whose stored SPT fails to load.
    broken: HashSet<UnitId>,
}

impl UnitStore for VecStore {
    fn n_units(&self) -> u32 {
        self.units.len() as u32
    }
    fn record(&self, id: UnitId) -> UnitRecord {
        self.units[id as usize].0.clone()
    }
    fn spt(&self, id: UnitId) -> Result<Spt, StoreError> {
        if self.broken.contains(&id) {
            return Err(StoreError::Corrupt {
                file: "spt.bin",
                reason: format!("blob {id} truncated"),
            });
        }
        Ok(self.units[id as usize].1.clone())
    }
    fn file_path(&self, file: u32) -> &Path {
        &self.files[file as usize]
    }
}

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

/// Query `x = y.open(z); x.read(z);` (7 leaves).
fn query_spt() -> Spt {
    block(vec![assign("x", "y", "open", "z"), call("x", "read", "z")])
}

/// The query's two statements plus `w = x.parse(z); w.close(y);` (14 leaves).
fn extended(i: usize) -> Spt {
    let n = |s: &str| format!("{s}{i}");
    let (x, y, z, w) = (n("x"), n("y"), n("z"), n("w"));
    block(vec![
        assign(&x, &y, "open", &z),
        call(&x, "read", &z),
        assign(&w, &x, "parse", &z),
        call(&w, "close", &y),
    ])
}

/// Only the query's first statement plus `w.close(y);`: fewer query features.
fn partial() -> Spt {
    block(vec![assign("x", "y", "open", "z"), call("w", "close", "y")])
}

fn unrelated() -> Spt {
    block(vec![
        call("total", "add", "one"),
        call("total", "add", "two"),
        call("acc", "flush", "total"),
    ])
}

/// Index and store over `spts`: unit i is a Java method with `body_hash` i + 1.
fn fixture(spts: Vec<Spt>) -> (MemIndex, VecStore) {
    let mut idx = MemIndex::default();
    let mut units = Vec::new();
    for (i, spt) in spts.into_iter().enumerate() {
        let id = i as UnitId;
        idx.insert(
            id,
            &unit_bag(Lang::Java, &featurize(&spt, &FeatureConfig::DEFAULT, None)),
        );
        let record = UnitRecord {
            id,
            lang: Lang::Java,
            kind: UnitKind::Method,
            file: 0,
            bytes: 0..0,
            lines: 1..2,
            n_leaves: spt.leaves.len() as u32,
            body_hash: id as u64 + 1,
            is_test: false,
        };
        units.push((record, spt));
    }
    let store = VecStore {
        units,
        files: vec![PathBuf::from("A.java")],
        broken: HashSet::new(),
    };
    (idx, store)
}

fn n_extended(n: usize) -> (MemIndex, VecStore) {
    fixture((0..n).map(extended).collect())
}

fn request<'a>(spt: &'a Spt, profile: &'a Profile) -> QueryRequest<'a> {
    QueryRequest {
        term_rule: TermRule::Identifier,
        lang: Lang::Java,
        spt,
        profile,
        top_similar: 10,
        experimental_recommend: false,
        gate: RecGate::DEFAULT,
        kind: None,
        include_tests: false,
        budget: None,
        lexical: None,
    }
}

fn capped(cap: usize) -> Profile {
    let mut p = Profile::REFERENCE;
    p.params.num_similars = cap;
    p
}

fn units(res: &QueryResult) -> Vec<UnitId> {
    res.similar.iter().map(|s| s.unit).collect()
}

fn query_feature_count(q: &Spt) -> u32 {
    unit_bag(Lang::Java, &featurize(q, &FeatureConfig::DEFAULT, None))
        .distinct()
        .count() as u32
}

#[test]
fn similar_code_end_to_end() {
    let (idx, store) = fixture(vec![extended(0), extended(1), extended(2), unrelated()]);
    let q = query_spt();
    let res = run_query(&idx, &store, &request(&q, &Profile::REFERENCE));
    assert_eq!(units(&res), [0, 1, 2]);
    for s in &res.similar {
        assert_eq!(s.score, 1.0);
        assert_eq!(s.kept.len(), 14);
        assert_eq!(s.kept.iter().filter(|&&k| k).count(), 7);
    }
    assert!(res.recommendations.is_empty());
    let qf = query_feature_count(&q);
    let d = &res.diagnostics;
    assert_eq!(d.query_features, qf);
    assert_eq!(d.known_query_features, qf);
    assert_eq!(d.ineligible_skipped, 0);
    assert_eq!(d.stale_units, 0);
    assert_eq!(d.budget_exhausted, None);
}

#[test]
fn ineligible_units_do_not_use_the_cap() {
    let (idx, mut store) = n_extended(5);
    store.units[0].0.is_test = true;
    store.units[1].0.lang = Lang::Apex;
    store.units[2].0.n_leaves = MAX_PRUNE_LEAVES + 1;
    let q = query_spt();
    let profile = capped(2);
    let res = run_query(&idx, &store, &request(&q, &profile));
    assert_eq!(units(&res), [3, 4]);
    assert_eq!(res.diagnostics.ineligible_skipped, 3);
    assert_eq!(res.diagnostics.stale_units, 0);
    // The size limit is inclusive.
    store.units[2].0.n_leaves = MAX_PRUNE_LEAVES;
    let res = run_query(&idx, &store, &request(&q, &profile));
    assert_eq!(units(&res), [2, 3]);
    assert_eq!(res.diagnostics.ineligible_skipped, 2);
}

#[test]
fn test_units_need_include_tests() {
    let (idx, mut store) = n_extended(3);
    store.units[0].0.is_test = true;
    let q = query_spt();
    let mut req = request(&q, &Profile::REFERENCE);
    let res = run_query(&idx, &store, &req);
    assert_eq!(units(&res), [1, 2]);
    assert_eq!(res.diagnostics.ineligible_skipped, 1);
    req.include_tests = true;
    let res = run_query(&idx, &store, &req);
    assert_eq!(units(&res), [0, 1, 2]);
    assert_eq!(res.diagnostics.ineligible_skipped, 0);
}

#[test]
fn kind_filter() {
    let (idx, mut store) = n_extended(3);
    store.units[1].0.kind = UnitKind::Constructor;
    let q = query_spt();
    let mut req = request(&q, &Profile::REFERENCE);
    req.kind = Some(UnitKind::Constructor);
    let res = run_query(&idx, &store, &req);
    assert_eq!(units(&res), [1]);
    assert_eq!(res.diagnostics.ineligible_skipped, 2);
    req.kind = Some(UnitKind::Trigger);
    let res = run_query(&idx, &store, &req);
    assert!(res.similar.is_empty());
    assert_eq!(res.diagnostics.ineligible_skipped, 3);
    req.kind = None;
    assert_eq!(units(&run_query(&idx, &store, &req)), [0, 1, 2]);
}

#[test]
fn clones_are_dropped_before_the_cap() {
    let (idx, mut store) = n_extended(4);
    for (i, h) in [7u64, 7, 9, 7].into_iter().enumerate() {
        store.units[i].0.body_hash = h;
    }
    let q = query_spt();
    let res = run_query(&idx, &store, &request(&q, &capped(2)));
    // Without dedup before the cap the second slot would go to clone 1.
    assert_eq!(units(&res), [0, 2]);
    assert_eq!(res.diagnostics.ineligible_skipped, 1);
    let res = run_query(&idx, &store, &request(&q, &Profile::REFERENCE));
    assert_eq!(units(&res), [0, 2]);
    assert_eq!(res.diagnostics.ineligible_skipped, 2);
}

#[test]
fn clone_dedup_keeps_the_best_ranked_representative() {
    // Unit 0 shares fewer query features, so unit 1 ranks first in Phase I.
    let (idx, mut store) = fixture(vec![partial(), extended(1)]);
    store.units[0].0.body_hash = 42;
    store.units[1].0.body_hash = 42;
    let q = query_spt();
    let res = run_query(&idx, &store, &request(&q, &Profile::REFERENCE));
    assert_eq!(units(&res), [1]);
    assert_eq!(res.diagnostics.ineligible_skipped, 1);
}

#[test]
fn stale_units_are_counted_skipped_and_backfilled() {
    let (idx, mut store) = n_extended(4);
    store.broken.insert(0);
    let q = query_spt();
    let res = run_query(&idx, &store, &request(&q, &capped(2)));
    assert_eq!(units(&res), [1, 2]);
    assert_eq!(res.diagnostics.stale_units, 1);
    assert_eq!(res.diagnostics.ineligible_skipped, 0);
    // A clone of an unreadable unit becomes the representative.
    store.units[1].0.body_hash = store.units[0].0.body_hash;
    let res = run_query(&idx, &store, &request(&q, &capped(2)));
    assert_eq!(units(&res), [1, 2]);
    assert_eq!(res.diagnostics.stale_units, 1);
    assert_eq!(res.diagnostics.ineligible_skipped, 0);
}

#[test]
fn postings_beyond_the_store_count_as_stale() {
    let (idx, mut store) = n_extended(4);
    store.units.truncate(3);
    let q = query_spt();
    let res = run_query(&idx, &store, &request(&q, &Profile::REFERENCE));
    assert_eq!(units(&res), [0, 1, 2]);
    assert_eq!(res.diagnostics.stale_units, 1);
    assert_eq!(res.diagnostics.ineligible_skipped, 0);
}

// ---------------------------------------------------------------------------------
// Deadlines, gated recommendations, determinism.

use std::time::Duration;

#[test]
fn zero_budget_returns_a_valid_partial_result() {
    let (idx, store) = n_extended(3);
    let q = query_spt();
    let mut req = request(&q, &Profile::REFERENCE);
    req.experimental_recommend = true;
    req.budget = Some(Duration::ZERO);
    let res = run_query(&idx, &store, &req);
    assert!(res.similar.is_empty());
    assert!(res.recommendations.is_empty());
    let qf = query_feature_count(&q);
    let d = &res.diagnostics;
    assert_eq!(d.budget_exhausted, Some("search"));
    // Search itself finished: its diagnostics are reported.
    assert_eq!(d.query_features, qf);
    assert_eq!(d.known_query_features, qf);
    assert_eq!(d.ineligible_skipped, 0);
    assert_eq!(d.stale_units, 0);
}

#[test]
fn generous_budget_completes_every_stage() {
    let (idx, store) = n_extended(3);
    let q = query_spt();
    let mut req = request(&q, &Profile::REFERENCE);
    req.experimental_recommend = true;
    req.budget = Some(Duration::from_secs(3600));
    let res = run_query(&idx, &store, &req);
    assert_eq!(res.diagnostics.budget_exhausted, None);
    assert_eq!(units(&res), [0, 1, 2]);
    assert_eq!(res.recommendations.len(), 1);
}

#[test]
fn recommendations_require_the_flag_and_the_gate() {
    let (idx, store) = fixture(vec![extended(0), extended(1), extended(2), unrelated()]);
    let q = query_spt();
    let mut req = request(&q, &Profile::REFERENCE);
    assert!(run_query(&idx, &store, &req).recommendations.is_empty());

    req.experimental_recommend = true;
    let res = run_query(&idx, &store, &req);
    assert_eq!(res.recommendations.len(), 1);
    let r = &res.recommendations[0];
    assert_eq!(r.members, [0, 1, 2]);
    assert_eq!(r.independent_support, 3);
    assert_eq!(r.kept, vec![true; 14]);

    // 14 kept - 7 query leaves = 7 added: passes ratio 1.0, fails 0.5.
    req.gate = RecGate {
        min_independent: 3,
        max_extension_ratio: 1.0,
    };
    assert_eq!(run_query(&idx, &store, &req).recommendations.len(), 1);
    req.gate = RecGate {
        min_independent: 3,
        max_extension_ratio: 0.5,
    };
    assert!(run_query(&idx, &store, &req).recommendations.is_empty());
    req.gate = RecGate {
        min_independent: 4,
        max_extension_ratio: 2.0,
    };
    assert!(run_query(&idx, &store, &req).recommendations.is_empty());
}

#[test]
fn clones_never_reach_the_cluster() {
    let (idx, mut store) = fixture(vec![extended(0), extended(1), extended(2), unrelated()]);
    store.units[1].0.body_hash = store.units[0].0.body_hash;
    let q = query_spt();
    let mut req = request(&q, &Profile::REFERENCE);
    req.experimental_recommend = true;
    let res = run_query(&idx, &store, &req);
    assert_eq!(units(&res), [0, 2]);
    // Only two independent members remain: below the default gate.
    assert!(res.recommendations.is_empty());
    req.gate = RecGate {
        min_independent: 2,
        max_extension_ratio: 2.0,
    };
    let res = run_query(&idx, &store, &req);
    assert_eq!(res.recommendations.len(), 1);
    assert_eq!(res.recommendations[0].members, [0, 2]);
    assert_eq!(res.recommendations[0].independent_support, 2);
}

#[test]
fn ordering_is_deterministic_and_top_similar_only_truncates_similar() {
    let (idx, store) = n_extended(5);
    let q = query_spt();
    let mut req = request(&q, &Profile::REFERENCE);
    req.experimental_recommend = true;
    let a = run_query(&idx, &store, &req);
    let b = run_query(&idx, &store, &req);
    // Equal scores keep Phase I order, which breaks overlap ties by unit id.
    assert_eq!(units(&a), [0, 1, 2, 3, 4]);
    let key = |r: &QueryResult| {
        (
            r.similar
                .iter()
                .map(|s| (s.unit, s.score.to_bits(), s.kept.clone()))
                .collect::<Vec<_>>(),
            r.recommendations
                .iter()
                .map(|x| (x.members.clone(), x.independent_support, x.kept.clone()))
                .collect::<Vec<_>>(),
            r.diagnostics.clone(),
        )
    };
    assert_eq!(key(&a), key(&b));
    assert_eq!(a.recommendations.len(), 1);
    assert_eq!(a.recommendations[0].members, [0, 1, 2, 3, 4]);

    req.top_similar = 2;
    let c = run_query(&idx, &store, &req);
    assert_eq!(units(&c), [0, 1]);
    assert_eq!(c.recommendations[0].members, [0, 1, 2, 3, 4]);
    req.top_similar = 0;
    let d = run_query(&idx, &store, &req);
    assert!(d.similar.is_empty());
    assert_eq!(d.recommendations.len(), 1);
}

// ---------------------------------------------------------------------------------
// D30: `similar` has no score floor; only clustering keeps `score > min_pruned`.

/// `x = y.open(z); x.close(z); w = x.parse(z); w.flush(y);` (14 leaves): passes Phase I
/// but its rerank score (weighted Jaccard 0.625) is below the reference `min_pruned`
/// (0.65).
fn weak(i: usize) -> Spt {
    let n = |s: &str| format!("{s}{i}");
    let (x, y, z, w) = (n("x"), n("y"), n("z"), n("w"));
    block(vec![
        assign(&x, &y, "open", &z),
        call(&x, "close", &z),
        assign(&w, &x, "parse", &z),
        call(&w, "flush", &y),
    ])
}

fn scores(res: &QueryResult) -> Vec<f64> {
    res.similar.iter().map(|s| s.score).collect()
}

#[test]
fn similar_has_no_score_floor_and_is_ordered_by_score() {
    // Phase I ranks the weak unit 0 first; the rerank score puts unit 1 first.
    let (idx, store) = fixture(vec![weak(0), partial()]);
    let q = query_spt();
    let profile = Profile::REFERENCE;
    let min = profile.params.min_pruned;
    let mut req = request(&q, &profile);
    let res = run_query(&idx, &store, &req);
    assert_eq!(units(&res), [1, 0]);
    let s = scores(&res);
    assert!(s[0] > min, "{s:?}");
    assert!(s[1] <= min && s[1] > 0.0, "{s:?}");
    assert_eq!(res.similar[1].kept.len(), 14);
    assert!(res.similar[1].kept.iter().any(|&k| k));
    assert!(res.recommendations.is_empty());
    assert_eq!(res.diagnostics.budget_exhausted, None);

    // `top_similar` truncates the score-sorted list.
    req.top_similar = 1;
    assert_eq!(units(&run_query(&idx, &store, &req)), [1]);
}

#[test]
fn low_scores_are_listed_but_never_recommended() {
    let (idx, store) = fixture(vec![
        extended(0),
        extended(1),
        extended(2),
        weak(3),
        weak(4),
        weak(5),
    ]);
    let q = query_spt();
    let profile = Profile::REFERENCE;
    let min = profile.params.min_pruned;
    let mut req = request(&q, &profile);
    req.experimental_recommend = true;
    let res = run_query(&idx, &store, &req);
    assert_eq!(units(&res), [0, 1, 2, 3, 4, 5]);
    let s = scores(&res);
    assert!(s[..3].iter().all(|&x| x == 1.0), "{s:?}");
    assert!(s[3..].iter().all(|&x| x <= min && x > 0.0), "{s:?}");
    assert_eq!(res.recommendations.len(), 1);
    assert_eq!(res.recommendations[0].members, [0, 1, 2]);

    // Weak units alone would cluster with each other if they reached the clusterer.
    let (idx, store) = fixture(vec![weak(0), weak(1), weak(2), weak(3)]);
    for min_independent in [1, 2, 3] {
        req.gate = RecGate {
            min_independent,
            max_extension_ratio: 100.0,
        };
        let res = run_query(&idx, &store, &req);
        assert_eq!(units(&res), [0, 1, 2, 3]);
        assert!(scores(&res).iter().all(|&x| x <= min));
        assert!(res.recommendations.is_empty());
    }
}
