//! Differential test against the Python reference (D13, task T16).
//!
//! `cargo run -p xtask -- gen-goldens` (T14) ran similar.py's own functions on 40
//! hand-written Java methods (the corpus) and 520 mutated queries, each converted by the Java
//! adapter in Compat mode, and checked the results in under
//! `crates/aroma-core/tests/fixtures/goldens/`. CI never runs Python: these tests replay the
//! Rust Compat pipeline (`Profile::REFERENCE.compat()`) on the same trees and compare FINAL
//! outputs only (D24):
//!
//! ```text
//!  stage      golden field                    Rust
//!  features   features (sorted strings)       featurize(spt, FeatureConfig::REFERENCE, None), flattened
//!  Phase I    known_query_features, hits      search(index, q, 0.4, 100, every unit eligible)
//!  prune      prunes[].kept, prunes[].score   prune(&[q], unit, None, ..), jaccard(q, pruned.bag)
//!  rerank     candidates                      rerank(q, hits, .., 0.65)    scores within 1e-9
//!  clusters   clusters[].members, .kept       cluster(cands, 0.9, 1.5, 5) + intersect
//! ```
//!
//! The goldens bake in two documented deviations (meta.json `deviations`): Phase I ties are
//! ordered by (overlap desc, unit asc), and features unknown to the corpus keep distinct
//! identities (D7 hashing) instead of collapsing to vocabulary id 10. A separate test
//! re-runs the Java Compat adapter on every golden source and requires the exact tree
//! similar.py consumed, so adapter drift is reported apart from algorithm drift.
//! Regenerate the goldens (never hand-edit them) with `cargo run -p xtask -- gen-goldens`.

use aroma_core::cluster::{cluster, intersect};
use aroma_core::deadline::Deadline;
use aroma_core::features::{featurize, unit_bag, FeatureConfig};
use aroma_core::index::{search, MemIndex};
use aroma_core::lang::Lang;
use aroma_core::multiset::{jaccard, Bag};
use aroma_core::profile::{Profile, ProfileName};
use aroma_core::prune::{prune, rerank};
use aroma_core::spt::Spt;
use aroma_lang::classify::{unit_spt, VarMode};
use aroma_lang::globals::Globals;
use aroma_lang::spec::spec_for;
use aroma_lang::table::{declared_types, extract_units, parse, Limits};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt::Write;
use std::path::PathBuf;

const REFERENCE_COMMIT: &str = "e8d8beeb99255d8929874c156fd65aac74d21508";
const SIMILAR_PY_SHA256: &str = "7203c86b1fca6cf21373cca2d9fc2f0a23a1685cc559887b02dbc52b9f411065";
/// 40 inputs x 13 mutations (T14).
const EXPECTED_CASES: usize = 520;

#[derive(Deserialize)]
struct Meta {
    reference_commit: String,
    similar_py_sha256: String,
    config: RefConfig,
    n_corpus: usize,
    n_cases: usize,
    n_cases_with_clusters: usize,
    skipped: Vec<Value>,
}

/// similar.py `Config` values the driver ran with.
#[derive(Deserialize)]
struct RefConfig {
    #[serde(rename = "N_PARENTS")]
    n_parents: usize,
    #[serde(rename = "N_SIBLINGS")]
    n_siblings: usize,
    #[serde(rename = "N_VAR_SIBLINGS")]
    n_var_siblings: usize,
    #[serde(rename = "NUM_SIMILARS")]
    num_similars: usize,
    #[serde(rename = "MIN_SIMILARITY_SCORE")]
    min_similarity_score: f64,
    #[serde(rename = "MIN_PRUNED_SCORE")]
    min_pruned_score: f64,
    #[serde(rename = "THRESHOLD1")]
    threshold1: f64,
    #[serde(rename = "THRESHOLD2")]
    threshold2: f64,
    #[serde(rename = "TOP_N")]
    top_n: usize,
}

#[derive(Deserialize)]
struct CorpusUnit {
    unit: u32,
    path: String,
    source: String,
    ast: Value,
    features: Vec<String>,
    n_leaves: usize,
}

#[derive(Clone, Deserialize)]
struct GoldPrune {
    unit: u32,
    kept: String,
    score: f64,
}

#[derive(Clone, Deserialize)]
struct GoldCluster {
    members: Vec<u32>,
    kept: String,
}

#[derive(Clone, Deserialize)]
struct Case {
    id: String,
    source: String,
    ast: Value,
    features: Vec<String>,
    known_query_features: u32,
    hits: Vec<(u32, u32)>,
    prunes: Vec<GoldPrune>,
    candidates: Vec<(u32, f64)>,
    clusters: Vec<GoldCluster>,
}

fn goldens_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../aroma-core/tests/fixtures/goldens")
}

fn read_json<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let path = goldens_dir().join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e} (checked in by T14; regenerate with `cargo run -p xtask -- gen-goldens`)",
            path.display()
        )
    });
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn profile() -> Profile {
    Profile::REFERENCE.compat()
}

#[test]
fn goldens_are_pinned_and_well_formed() {
    let meta: Meta = read_json("meta.json");
    assert_eq!(meta.reference_commit, REFERENCE_COMMIT);
    assert_eq!(meta.similar_py_sha256, SIMILAR_PY_SHA256);

    let p = profile();
    assert_eq!(p.name, ProfileName::Reference);
    assert_eq!(p.features, FeatureConfig::REFERENCE);
    let c = &meta.config;
    assert_eq!(
        (c.n_parents, c.n_siblings, c.n_var_siblings),
        (
            p.features.n_parents,
            p.features.n_siblings,
            p.features.n_var_siblings
        )
    );
    assert_eq!(c.num_similars, p.params.num_similars);
    assert_eq!(c.min_similarity_score, p.params.min_similarity);
    assert_eq!(c.min_pruned_score, p.params.min_pruned);
    assert_eq!(c.threshold1, p.params.tau3);
    assert_eq!(c.threshold2, p.params.tau2);
    assert_eq!(c.top_n, p.params.top_n);

    let corpus: Vec<CorpusUnit> = read_json("corpus.json");
    let cases: Vec<Case> = read_json("cases.json");
    assert_eq!(corpus.len(), meta.n_corpus);
    assert_eq!(meta.n_corpus, 40);
    assert_eq!(cases.len(), meta.n_cases);
    assert_eq!(meta.n_cases + meta.skipped.len(), EXPECTED_CASES);
    assert!(meta.n_cases >= 500, "skipped: {:?}", meta.skipped);
    let with_clusters = cases.iter().filter(|c| !c.clusters.is_empty()).count();
    assert_eq!(with_clusters, meta.n_cases_with_clusters);
    assert!(
        with_clusters >= 40,
        "only {with_clusters} cases form clusters"
    );

    for (i, u) in corpus.iter().enumerate() {
        assert_eq!(u.unit as usize, i, "corpus.json is in unit order");
        assert!(
            u.path.ends_with(".java") && !u.source.is_empty(),
            "{}",
            u.path
        );
        assert!(
            u.ast.is_array() && !u.features.is_empty() && u.n_leaves > 0,
            "{}",
            u.path
        );
    }
    let mut ids = HashSet::new();
    for case in &cases {
        let id = &case.id;
        assert!(ids.insert(id.as_str()), "duplicate case {id}");
        assert!(!case.source.is_empty() && case.ast.is_array(), "{id}");
        let distinct: HashSet<&String> = case.features.iter().collect();
        assert!(case.known_query_features as usize <= distinct.len(), "{id}");
        assert_eq!(case.hits.len(), case.prunes.len(), "{id}");
        for (h, p) in case.hits.iter().zip(&case.prunes) {
            assert_eq!(h.0, p.unit, "{id}");
            assert_eq!(p.kept.len(), corpus[p.unit as usize].n_leaves, "{id}");
            assert!((0.0..=1.0).contains(&p.score), "{id}");
        }
        for (u, s) in &case.candidates {
            assert!(*s > c.min_pruned_score, "{id}");
            assert!(
                case.prunes.iter().any(|p| p.unit == *u && p.score == *s),
                "{id}"
            );
        }
        for cl in &case.clusters {
            assert!(cl.members.len() >= 2, "{id}");
            assert_eq!(
                cl.kept.len(),
                corpus[cl.members[0] as usize].n_leaves,
                "{id}"
            );
        }
    }
}

const LANG: Lang = Lang::Java;

/// One mismatch row of the report table.
struct Row {
    case: String,
    stage: &'static str,
    expected: String,
    actual: String,
}

fn row(case: &str, stage: &'static str, expected: String, actual: String) -> Row {
    Row {
        case: case.to_string(),
        stage,
        expected,
        actual,
    }
}

fn clip(s: &str) -> String {
    const MAX: usize = 160;
    let mut out: String = s.chars().take(MAX).collect();
    if s.chars().count() > MAX {
        out.push('…');
    }
    out.replace('|', "\\|")
}

/// Markdown table of the first 40 rows plus per-stage counts.
fn table(rows: &[Row]) -> String {
    let mut by_stage: BTreeMap<&str, usize> = BTreeMap::new();
    for r in rows {
        *by_stage.entry(r.stage).or_default() += 1;
    }
    let mut s = String::new();
    writeln!(s, "{} mismatches by stage: {by_stage:?}", rows.len()).unwrap();
    writeln!(
        s,
        "| case | stage | expected (similar.py) | actual (Rust) |"
    )
    .unwrap();
    writeln!(s, "|---|---|---|---|").unwrap();
    for r in rows.iter().take(40) {
        writeln!(
            s,
            "| {} | {} | {} | {} |",
            r.case,
            r.stage,
            clip(&r.expected),
            clip(&r.actual)
        )
        .unwrap();
    }
    if rows.len() > 40 {
        writeln!(s, "... and {} more rows", rows.len() - 40).unwrap();
    }
    s
}

fn tree(ast: &Value, what: &str) -> Spt {
    Spt::from_reference_json(ast)
        .unwrap_or_else(|| panic!("{what}: golden ast is not a similar.py tree"))
}

/// The Compat-mode Java pipeline of `xtask/src/gen_goldens.rs::adapt` (T14), which mirrors
/// the index build (T15): parse, repository types as globals, one unit, normalize +
/// classify + collapse.
fn adapt(src: &str) -> Result<Spt, String> {
    let spec = spec_for(LANG).unwrap();
    let limits = Limits::DEFAULT;
    let tree = parse(spec, src, &limits).map_err(|e| e.to_string())?;
    let globals = Globals::new(spec, declared_types(&tree, src, spec));
    let units = extract_units(&tree, src, spec, &limits);
    let [unit] = units.as_slice() else {
        return Err(format!(
            "expected exactly one method, found {}",
            units.len()
        ));
    };
    unit_spt(unit, spec, &globals, VarMode::Compat).ok_or_else(|| "empty method body".to_string())
}

/// Leaf identity as similar.py sees it (byte spans and SymbolIds are not in the JSON).
fn leaf_view(s: &Spt) -> Vec<(String, bool, u32)> {
    s.leaves
        .iter()
        .map(|l| (l.text.to_string(), l.is_var, l.line))
        .collect()
}

fn same_tree(a: &Spt, b: &Spt) -> bool {
    a.nodes == b.nodes && leaf_view(a) == leaf_view(b)
}

fn first_difference(golden: &Spt, actual: &Spt) -> (String, String) {
    for (i, (g, a)) in golden.nodes.iter().zip(&actual.nodes).enumerate() {
        if g != a {
            return (
                format!("node {i}: {} {:?}", g.label, g.children),
                format!("node {i}: {} {:?}", a.label, a.children),
            );
        }
    }
    if golden.nodes.len() != actual.nodes.len() {
        return (
            format!("{} nodes", golden.nodes.len()),
            format!("{} nodes", actual.nodes.len()),
        );
    }
    let (g, a) = (leaf_view(golden), leaf_view(actual));
    for (i, (x, y)) in g.iter().zip(&a).enumerate() {
        if x != y {
            return (format!("leaf {i}: {x:?}"), format!("leaf {i}: {y:?}"));
        }
    }
    (format!("{} leaves", g.len()), format!("{} leaves", a.len()))
}

#[test]
fn java_compat_adapter_reproduces_the_golden_trees() {
    let corpus: Vec<CorpusUnit> = read_json("corpus.json");
    let cases: Vec<Case> = read_json("cases.json");
    let inputs = corpus
        .iter()
        .map(|u| (&u.path, &u.source, &u.ast))
        .chain(cases.iter().map(|c| (&c.id, &c.source, &c.ast)));
    let mut rows = Vec::new();
    for (what, source, ast) in inputs {
        let golden = tree(ast, what);
        match adapt(source) {
            Ok(spt) if same_tree(&golden, &spt) => {}
            Ok(spt) => {
                let (e, a) = first_difference(&golden, &spt);
                rows.push(row(what, "adapter", e, a));
            }
            Err(e) => rows.push(row(what, "adapter", "a Java method".into(), e)),
        }
    }
    assert!(
        rows.is_empty(),
        "the Java Compat adapter no longer produces the trees the goldens were generated from; \
         if the change is intended, run `cargo run -p xtask -- gen-goldens` and review the diff\n{}",
        table(&rows)
    );
}

const SCORE_TOL: f64 = 1e-9;

/// The golden corpus as Rust sees it: trees, bags and a Phase I index (unit i = corpus[i]).
struct Corpus {
    spts: Vec<Spt>,
    bags: Vec<Bag>,
    index: MemIndex,
}

fn mask(kept: &[bool]) -> String {
    kept.iter().map(|&k| if k { '1' } else { '0' }).collect()
}

fn sorted_features(per_leaf: &[Vec<String>]) -> Vec<String> {
    let mut all = per_leaf.concat();
    all.sort();
    all
}

/// (only in similar.py, only in Rust) as multisets.
fn multiset_diff(expected: &[String], actual: &[String]) -> (String, String) {
    let mut counts: BTreeMap<&str, i64> = BTreeMap::new();
    for f in expected {
        *counts.entry(f.as_str()).or_default() += 1;
    }
    for f in actual {
        *counts.entry(f.as_str()).or_default() -= 1;
    }
    let only = |sign: i64| -> Vec<String> {
        counts
            .iter()
            .filter(|(_, &c)| c * sign > 0)
            .map(|(f, &c)| format!("{f} x{}", c * sign))
            .collect()
    };
    (
        format!("only in similar.py: {:?}", only(1)),
        format!("only in Rust: {:?}", only(-1)),
    )
}

fn load_corpus(p: &Profile) -> (Corpus, Vec<Row>) {
    let units: Vec<CorpusUnit> = read_json("corpus.json");
    let mut rows = Vec::new();
    let mut corpus = Corpus {
        spts: Vec::new(),
        bags: Vec::new(),
        index: MemIndex::default(),
    };
    for (i, u) in units.iter().enumerate() {
        let spt = tree(&u.ast, &u.path);
        assert_eq!(spt.leaves.len(), u.n_leaves, "{}", u.path);
        let per_leaf = featurize(&spt, &p.features, None);
        let features = sorted_features(&per_leaf);
        if features != u.features {
            let (e, a) = multiset_diff(&u.features, &features);
            rows.push(row(&u.path, "corpus-features", e, a));
        }
        let bag = unit_bag(LANG, &per_leaf);
        corpus.index.insert(i as u32, &bag);
        corpus.spts.push(spt);
        corpus.bags.push(bag);
    }
    (corpus, rows)
}

/// Run one golden query through the Rust Compat pipeline; one row per differing stage.
fn compare_case(corpus: &Corpus, case: &Case, p: &Profile) -> Vec<Row> {
    let none = Deadline::none();
    let cfg = &p.features;
    let id = case.id.as_str();
    let mut rows = Vec::new();

    let query = tree(&case.ast, id);
    let per_leaf = featurize(&query, cfg, None);
    let features = sorted_features(&per_leaf);
    if features != case.features {
        let (e, a) = multiset_diff(&case.features, &features);
        rows.push(row(id, "features", e, a));
    }
    let q = unit_bag(LANG, &per_leaf);

    let outcome = search(
        &corpus.index,
        &q,
        p.params.min_similarity,
        p.params.num_similars,
        &|_: u32| true,
    );
    if outcome.known_query_features != case.known_query_features {
        rows.push(row(
            id,
            "known-features",
            case.known_query_features.to_string(),
            outcome.known_query_features.to_string(),
        ));
    }
    let hits: Vec<(u32, u32)> = outcome.hits.iter().map(|h| (h.unit, h.overlap)).collect();
    if hits != case.hits {
        rows.push(row(
            id,
            "phase1",
            format!("{:?}", case.hits),
            format!("{hits:?}"),
        ));
    }

    for g in &case.prunes {
        let u = g.unit as usize;
        let pruned = prune(&[&q], &corpus.spts[u], None, LANG, cfg, p.objective, &none)
            .expect("Deadline::none never cancels");
        let kept = mask(&pruned.kept);
        if kept != g.kept {
            rows.push(row(
                id,
                "prune-kept",
                format!("unit {u}: {}", g.kept),
                format!("unit {u}: {kept}"),
            ));
        }
        let score = jaccard(&q, &pruned.bag);
        if (score - g.score).abs() > SCORE_TOL {
            rows.push(row(
                id,
                "prune-score",
                format!("unit {u}: {}", g.score),
                format!("unit {u}: {score}"),
            ));
        }
    }

    let input: Vec<(u32, Spt, Bag)> = outcome
        .hits
        .iter()
        .map(|h| {
            let u = h.unit as usize;
            (h.unit, corpus.spts[u].clone(), corpus.bags[u].clone())
        })
        .collect();
    let (cands, cancelled) = rerank(
        &q,
        input,
        LANG,
        cfg,
        p.objective,
        p.params.min_pruned,
        &none,
    );
    assert!(cancelled.is_none(), "{id}: Deadline::none never cancels");
    let ranked: Vec<(u32, f64)> = cands.iter().map(|c| (c.unit, c.score)).collect();
    let same_ranking = ranked.len() == case.candidates.len()
        && ranked
            .iter()
            .zip(&case.candidates)
            .all(|(a, e)| a.0 == e.0 && (a.1 - e.1).abs() <= SCORE_TOL);
    if !same_ranking {
        rows.push(row(
            id,
            "rerank",
            format!("{:?}", case.candidates),
            format!("{ranked:?}"),
        ));
    }

    let tuples = cluster(&cands, p.params.tau3, p.params.tau2, p.params.top_n, &none)
        .expect("Deadline::none never cancels");
    let clusters: Vec<(Vec<u32>, String)> = tuples
        .iter()
        .map(|t| {
            let kept =
                intersect(&q, t, &cands, LANG, cfg, &none).expect("Deadline::none never cancels");
            (t.iter().map(|&i| cands[i].unit).collect(), mask(&kept))
        })
        .collect();
    let expected: Vec<(Vec<u32>, String)> = case
        .clusters
        .iter()
        .map(|c| (c.members.clone(), c.kept.clone()))
        .collect();
    if clusters != expected {
        rows.push(row(
            id,
            "clusters",
            format!("{expected:?}"),
            format!("{clusters:?}"),
        ));
    }
    rows
}

#[test]
fn rust_compat_pipeline_matches_similar_py() {
    let p = profile();
    let (corpus, mut rows) = load_corpus(&p);
    let cases: Vec<Case> = read_json("cases.json");
    for case in &cases {
        rows.extend(compare_case(&corpus, case, &p));
    }
    assert!(
        rows.is_empty(),
        "Rust Compat pipeline differs from similar.py on the D13 goldens\n{}",
        table(&rows)
    );
}

#[test]
fn diff_harness_reports_perturbed_goldens() {
    let p = profile();
    let (corpus, rows) = load_corpus(&p);
    assert!(rows.is_empty(), "{}", table(&rows));
    let cases: Vec<Case> = read_json("cases.json");
    let mut case = cases
        .iter()
        .find(|c| !c.prunes.is_empty() && !c.clusters.is_empty())
        .expect("the goldens contain cases with clusters")
        .clone();
    assert!(compare_case(&corpus, &case, &p).is_empty(), "{}", case.id);

    case.features.pop();
    case.hits.push((u32::MAX, 1));
    case.prunes[0].score += 1e-6;
    let flipped = if case.prunes[0].kept.starts_with('1') {
        "0"
    } else {
        "1"
    };
    case.prunes[0].kept.replace_range(0..1, flipped);
    case.clusters.clear();
    let stages: BTreeSet<&str> = compare_case(&corpus, &case, &p)
        .iter()
        .map(|r| r.stage)
        .collect();
    assert_eq!(
        stages,
        BTreeSet::from([
            "clusters",
            "features",
            "phase1",
            "prune-kept",
            "prune-score"
        ])
    );
}
