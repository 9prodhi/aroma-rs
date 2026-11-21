//! T23: the BM25 identifier channel inside `run_query` — RRF fusion of the Phase I and
//! BM25 rankings, final RRF of (rerank rank, BM25 rank), clone dedup across channels,
//! recommendations from structurally-qualified candidates only, and the unchanged
//! structural pipeline when the channel is off. Uses `MemIndex` + `MemLexical`.

use aroma_core::cluster::RecGate;
use aroma_core::features::{featurize, unit_bag, FeatureConfig};
use aroma_core::index::MemIndex;
use aroma_core::lang::Lang;
use aroma_core::lexical::{rrf, unit_terms, MemLexical, TermRule};
use aroma_core::profile::Profile;
use aroma_core::query::{run_query, QueryRequest, QueryResult};
use aroma_core::spt::{Leaf, Raw, Spt};
use aroma_core::store::StoreError;
use aroma_core::unit::{UnitId, UnitKind, UnitRecord, UnitStore};
use std::path::{Path, PathBuf};
use std::time::Duration;

struct VecStore {
    units: Vec<(UnitRecord, Spt)>,
    files: Vec<PathBuf>,
}

impl UnitStore for VecStore {
    fn n_units(&self) -> u32 {
        self.units.len() as u32
    }
    fn record(&self, id: UnitId) -> UnitRecord {
        self.units[id as usize].0.clone()
    }
    fn spt(&self, id: UnitId) -> Result<Spt, StoreError> {
        Ok(self.units[id as usize].1.clone())
    }
    fn file_path(&self, file: u32) -> &Path {
        &self.files[file as usize]
    }
}

fn kw(s: &str) -> Raw {
    Raw::Keyword(s.to_string())
}

fn leaf(s: &str, is_var: bool) -> Raw {
    Raw::Leaf(Leaf {
        text: s.into(),
        is_var,
        binding: None,
        bytes: 0..0,
        line: 1,
    })
}

/// `lhs = recv.m(arg);`
fn assign(lhs: &str, recv: &str, m: &str, arg: &str) -> Raw {
    Raw::List(vec![
        Raw::List(vec![
            leaf(lhs, true),
            kw("="),
            Raw::List(vec![
                leaf(recv, true),
                kw("."),
                Raw::List(vec![leaf(m, false), kw("("), leaf(arg, true), kw(")")]),
            ]),
        ]),
        kw(";"),
    ])
}

/// `recv.m(arg);`
fn call(recv: &str, m: &str, arg: &str) -> Raw {
    Raw::List(vec![
        Raw::List(vec![
            leaf(recv, true),
            kw("."),
            Raw::List(vec![leaf(m, false), kw("("), leaf(arg, true), kw(")")]),
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

/// `x = y.open(z); x.read(z); x.commitBatch(z);`: the start of an idiom whose last call
/// names a rare API.
fn query_spt() -> Spt {
    block(vec![
        assign("x", "y", "open", "z"),
        call("x", "read", "z"),
        call("x", "commitBatch", "z"),
    ])
}

/// The query's first two statements plus `w = x.parse(z); w.close(y);`.
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

fn unrelated() -> Spt {
    block(vec![
        call("total", "add", "one"),
        call("total", "add", "two"),
        call("acc", "flush", "total"),
    ])
}

/// `return commitBatch;` among unrelated statements: shares only the rare identifier
/// with the query and is structurally a different method (below the Phase I ratio).
fn rare() -> Spt {
    let ret = Raw::List(vec![kw("return"), leaf("commitBatch", false), kw(";")]);
    let inc = |v: &str| Raw::List(vec![leaf(v, true), kw("++"), kw(";")]);
    block(vec![inc("count"), inc("total"), ret])
}

/// Units 0-2 extended, 3 unrelated, 4 rare.
fn fixture() -> (MemIndex, MemLexical, VecStore) {
    build(vec![
        extended(0),
        extended(1),
        extended(2),
        unrelated(),
        rare(),
    ])
}

fn build(spts: Vec<Spt>) -> (MemIndex, MemLexical, VecStore) {
    let mut idx = MemIndex::default();
    let mut lex = MemLexical::default();
    let mut units = Vec::new();
    for (i, spt) in spts.into_iter().enumerate() {
        let id = i as UnitId;
        idx.insert(
            id,
            &unit_bag(Lang::Java, &featurize(&spt, &FeatureConfig::DEFAULT, None)),
        );
        lex.insert(id, &unit_terms(&spt, TermRule::Identifier));
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
    };
    (idx, lex, store)
}

fn request<'a>(
    spt: &'a Spt,
    profile: &'a Profile,
    lexical: Option<&'a MemLexical>,
) -> QueryRequest<'a> {
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
        lexical: lexical.map(|l| l as _),
    }
}

fn units(res: &QueryResult) -> Vec<UnitId> {
    res.similar.iter().map(|s| s.unit).collect()
}

#[test]
fn channel_off_reports_structural_ranks_only() {
    let (idx, _lex, store) = fixture();
    let q = query_spt();
    let res = run_query(&idx, &store, &request(&q, &Profile::REFERENCE, None));
    assert!(!units(&res).contains(&4), "precondition: {:?}", units(&res));
    for (i, s) in res.similar.iter().enumerate() {
        assert_eq!(s.structural_rank, i as u32 + 1);
        assert_eq!((s.lexical_rank, s.bm25), (None, None));
        assert!(s.matched_terms.is_empty());
    }
    let d = &res.diagnostics;
    assert!(!d.lexical);
    assert_eq!((d.query_terms, d.known_query_terms), (0, 0));
}

#[test]
fn rare_identifiers_retrieve_a_unit_structural_search_misses() {
    let (idx, lex, store) = fixture();
    let q = query_spt();
    let res = run_query(&idx, &store, &request(&q, &Profile::REFERENCE, Some(&lex)));
    let rare = res
        .similar
        .iter()
        .find(|s| s.unit == 4)
        .unwrap_or_else(|| panic!("unit 4 missing from {:?}", units(&res)));
    assert_eq!(rare.lexical_rank, Some(1));
    assert_eq!(rare.matched_terms, ["batch", "commit", "commitbatch"]);
    assert!(rare.bm25.unwrap() > 0.0);
    // The extended units match `open` and `read` only.
    for s in res.similar.iter().filter(|s| s.unit < 3) {
        assert_eq!(s.matched_terms, ["open", "read"]);
        assert!(s.bm25.unwrap() < rare.bm25.unwrap());
    }
    let d = &res.diagnostics;
    assert!(d.lexical);
    // open, read, commitbatch, commit, batch (x, y, z are shorter than 2).
    assert_eq!((d.query_terms, d.known_query_terms), (5, 5));
}

#[test]
fn similar_order_is_rrf_of_rerank_rank_and_bm25_rank() {
    let (idx, lex, store) = fixture();
    let q = query_spt();
    let res = run_query(&idx, &store, &request(&q, &Profile::REFERENCE, Some(&lex)));
    assert!(res.similar.len() >= 4);
    let fused = |s: &aroma_core::query::Similar| {
        rrf(s.structural_rank as usize) + s.lexical_rank.map_or(0.0, |r| rrf(r as usize))
    };
    for w in res.similar.windows(2) {
        let (a, b) = (fused(&w[0]), fused(&w[1]));
        assert!(a > b || (a == b && w[0].unit < w[1].unit), "{w:?}");
    }
    // Structural ranks follow the rerank score; BM25 ranks follow bm25.
    let mut by_rank: Vec<_> = res.similar.iter().collect();
    by_rank.sort_by_key(|s| s.structural_rank);
    assert!(by_rank.windows(2).all(|w| w[0].score >= w[1].score));
    let mut by_lex: Vec<_> = res
        .similar
        .iter()
        .filter(|s| s.lexical_rank.is_some())
        .collect();
    by_lex.sort_by_key(|s| s.lexical_rank);
    assert!(by_lex.windows(2).all(|w| w[0].bm25 >= w[1].bm25));
    // The best structural match (also second by BM25) leads; the rare unit, first by
    // BM25 and last by rerank score, still makes the list.
    assert_eq!(units(&res)[0], 0);
    assert!(units(&res).contains(&4));
}

#[test]
fn recommendations_use_only_structurally_qualified_candidates() {
    let (idx, lex, store) = fixture();
    let q = query_spt();
    let profile = Profile::REFERENCE;
    let mut req = request(&q, &profile, Some(&lex));
    req.experimental_recommend = true;
    req.gate = RecGate {
        min_independent: 3,
        max_extension_ratio: 100.0,
    };
    let on = run_query(&idx, &store, &req);
    assert!(units(&on).contains(&4));
    assert!(!on.recommendations.is_empty());
    for r in &on.recommendations {
        for m in &r.members {
            let s = on.similar.iter().find(|s| s.unit == *m).unwrap();
            assert!(s.score > profile.params.min_pruned, "{m}: {}", s.score);
        }
        assert!(!r.members.contains(&4));
    }
    req.lexical = None;
    let off = run_query(&idx, &store, &req);
    let key = |r: &QueryResult| {
        r.recommendations
            .iter()
            .map(|x| (x.members.clone(), x.independent_support, x.kept.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(key(&on), key(&off));
}

#[test]
fn a_clone_found_by_both_channels_is_listed_once() {
    let (idx, lex, mut store) = fixture();
    // Unit 4 becomes an exact clone of unit 0 (same body hash): unit 0 fuses higher.
    store.units[4].0.body_hash = store.units[0].0.body_hash;
    let q = query_spt();
    let res = run_query(&idx, &store, &request(&q, &Profile::REFERENCE, Some(&lex)));
    let u = units(&res);
    assert!(u.contains(&0) && !u.contains(&4), "{u:?}");
    assert!(res.diagnostics.ineligible_skipped >= 1);
}

#[test]
fn lexical_channel_obeys_eligibility() {
    let (idx, lex, mut store) = fixture();
    store.units[4].0.is_test = true;
    let q = query_spt();
    let mut req = request(&q, &Profile::REFERENCE, Some(&lex));
    assert!(!units(&run_query(&idx, &store, &req)).contains(&4));
    req.include_tests = true;
    assert!(units(&run_query(&idx, &store, &req)).contains(&4));
    req.kind = Some(UnitKind::Trigger);
    assert!(run_query(&idx, &store, &req).similar.is_empty());
}

#[test]
fn deterministic_and_budgeted() {
    let (idx, lex, store) = fixture();
    let q = query_spt();
    let mut req = request(&q, &Profile::REFERENCE, Some(&lex));
    let key = |r: &QueryResult| {
        r.similar
            .iter()
            .map(|s| {
                (
                    s.unit,
                    s.score.to_bits(),
                    s.structural_rank,
                    s.lexical_rank,
                    s.bm25.map(f64::to_bits),
                    s.matched_terms.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    let a = run_query(&idx, &store, &req);
    let b = run_query(&idx, &store, &req);
    assert_eq!(key(&a), key(&b));
    assert_eq!(a.diagnostics, b.diagnostics);
    req.budget = Some(Duration::ZERO);
    let z = run_query(&idx, &store, &req);
    assert!(z.similar.is_empty());
    assert_eq!(z.diagnostics.budget_exhausted, Some("search"));
}

#[test]
fn snippet_without_identifiers_falls_back_to_structure() {
    let (idx, lex, store) = fixture();
    // Every leaf is one character: no lexical term.
    let q = block(vec![assign("x", "y", "o", "z"), call("x", "r", "z")]);
    let on = run_query(&idx, &store, &request(&q, &Profile::REFERENCE, Some(&lex)));
    let off = run_query(&idx, &store, &request(&q, &Profile::REFERENCE, None));
    assert_eq!(on.diagnostics.query_terms, 0);
    assert_eq!(units(&on), units(&off));
}
