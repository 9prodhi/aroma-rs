//! Query engine (D22/D23): Phase I search with eligibility -> load stored SPTs ->
//! prune + rerank -> (with `experimental_recommend`) cluster, intersect and gate.
//!
//! ```text
//!  query Spt --featurize--> Bag q
//!     |
//!  [search]    overlap > min_similarity * known, ranked (overlap desc, unit asc);
//!              eligibility is decided lazily in rank order, so ineligible units never
//!              use one of the `num_similars` slots (backfill):
//!                not same lang / kind filter / is_test (unless include_tests) /
//!                n_leaves > MAX_PRUNE_LEAVES / clone of a better-ranked unit
//!                (same body_hash)                    -> ineligible_skipped
//!                stored SPT unreadable or unknown id -> stale_units
//!  [lexical]   (only with `req.lexical`, T23) BM25 over the snippet's identifier
//!              terms, ranked (bm25 desc, unit asc), same lazy eligibility; the
//!              candidates are the RRF fusion sum 1/(60 + rank) of the Phase I and
//!              BM25 rankings (ties by unit id), clones of a better-fused unit
//!              dropped, top num_similars
//!  [load]      featurize each admitted unit
//!  [rerank]    prune every admitted unit towards q, score it (no floor, D30),
//!              stable sort by score desc
//!  [similar]   without lexical: the score-sorted list truncated to top_similar;
//!              with lexical: exact token prefixes first (through the last query leaf,
//!              at least 3 leaves), then matching inter-leaf byte gaps desc and
//!              BM25 / candidate leaves desc, then unit lines desc; remaining
//!              candidates use RRF of
//!              (rerank rank, BM25 rank); ties by unit id; truncate to top_similar
//!  [cluster]   candidates with score > min_pruned, in rerank   } only with
//!              order; tuples (tau3, tau2, top_n)               } experimental_
//!  [intersect] per tuple, then RecGate (support, extension)    } recommend
//! ```
//!
//! `similar` has no score cutoff (D30): each result carries its rerank score, and a low
//! score means a weak match. The reference threshold `min_pruned` (0.65) still decides
//! which candidates may form a cluster, so recommendations are unchanged.
//!
//! The lexical channel never changes the reference pipeline: with `req.lexical = None`
//! (the facade passes `None` under `--compat` and `--no-lexical`) the candidates, their
//! order and every score are exactly the structural ones. Clustering always reads only
//! candidates whose structural score is `> min_pruned`.
//!
//! One `Deadline` built from `budget` flows through every stage. When it fires, the
//! stages that finished are returned (`similar` holds the candidates reranked so far)
//! and `Diagnostics::budget_exhausted` names the first stage that stopped: `"search"`,
//! `"lexical"`, `"load"`, `"rerank"`, `"cluster"` or `"intersect"`. A cancelled prune inside rerank
//! or intersect is reported as that enclosing stage. `stale_units` counts only units
//! whose stored tree could not be read; the index facade adds the units whose source
//! changed since indexing.

use crate::cluster::{cluster, independent_support, intersect, passes_gate, RecGate};
use crate::deadline::{Cancelled, Deadline};
use crate::features::{featurize, unit_bag, FeatureConfig};
use crate::index::{search, PostingSource};
use crate::lang::Lang;
use crate::lexical::{
    bm25_term, idf, lexical_search, rrf, spt_terms, term_frequency, term_id, LexicalSource, TermId,
    TermRule,
};
use crate::multiset::Bag;
use crate::profile::Profile;
use crate::prune::{rerank, Candidate, Objective};
use crate::spt::Spt;
use crate::unit::{UnitId, UnitKind, UnitStore, MAX_PRUNE_LEAVES};
use serde::Serialize;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

const SEARCH: &str = "search";
const LEXICAL: &str = "lexical";
const LOAD: &str = "load";
const RERANK: &str = "rerank";
const CLUSTER: &str = "cluster";
const INTERSECT: &str = "intersect";

/// Rerank floor below every weighted Jaccard / SimScore ratio (all are >= 0): `similar`
/// lists every admitted unit by score; low scores are weak matches (D30).
const NO_SCORE_FLOOR: f64 = -1.0;

pub struct QueryRequest<'a> {
    pub lang: Lang,
    pub spt: &'a Spt,
    /// Already adjusted for the query language; all candidates share its language.
    pub profile: &'a Profile,
    /// Language-selected lexical tokenization.
    pub term_rule: TermRule,
    pub top_similar: usize,
    pub experimental_recommend: bool,
    pub gate: RecGate,
    pub kind: Option<UnitKind>,
    pub include_tests: bool,
    pub budget: Option<Duration>,
    /// BM25 identifier channel fused with the structural ranking (T23); `None` runs the
    /// pure reference pipeline.
    pub lexical: Option<&'a dyn LexicalSource>,
}

#[derive(Clone, Debug)]
pub struct Similar {
    pub unit: UnitId,
    pub score: f64,
    /// Kept-leaf mask over the unit's stored SPT.
    pub kept: Vec<bool>,
    /// 1-based rank in the structural rerank order (by `score`).
    pub structural_rank: u32,
    /// 1-based rank among the eligible BM25 hits; `None` when the channel is off or the
    /// unit is not among the best `num_similars` BM25 hits.
    pub lexical_rank: Option<u32>,
    /// BM25 score; `None` when the channel is off.
    pub bm25: Option<f64>,
    /// Query identifier terms the unit holds, sorted (identifiers only, never literals).
    pub matched_terms: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Recommendation {
    /// Cluster members in rerank order; `kept` refers to the first member's SPT.
    pub members: Vec<UnitId>,
    pub independent_support: usize,
    pub kept: Vec<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Diagnostics {
    /// Distinct query features.
    pub query_features: u32,
    /// Distinct query features present in the index.
    pub known_query_features: u32,
    /// Units passed over by the eligibility filter (lang, kind, tests, size, clones).
    pub ineligible_skipped: u32,
    /// Units whose stored SPT could not be read (the facade adds stale sources).
    pub stale_units: u32,
    /// First stage stopped by the budget, if any.
    pub budget_exhausted: Option<&'static str>,
    /// The BM25 identifier channel ran (T23).
    pub lexical: bool,
    /// Distinct identifier terms of the query (0 when the channel is off).
    pub query_terms: u32,
    /// Distinct query terms present in the lexical index.
    pub known_query_terms: u32,
}

#[derive(Clone, Debug)]
pub struct QueryResult {
    pub similar: Vec<Similar>,
    pub recommendations: Vec<Recommendation>,
    pub diagnostics: Diagnostics,
}

/// State of the lazy eligibility filter, consulted by `search` (and the lexical
/// search) in rank order.
#[derive(Default)]
struct Admission {
    /// `body_hash` of every unit admitted by the current walk: later units with the same
    /// hash are clones. Reset between the structural and the lexical walk.
    hashes: HashSet<u64>,
    /// Stored trees of the admitted units (shared by both walks).
    spts: HashMap<UnitId, Spt>,
    /// Units rejected because their tree could not be read (or the id is unknown).
    stale: HashSet<UnitId>,
    /// Units passed over by the filter (lang, kind, tests, size, clones).
    skipped: HashSet<UnitId>,
}

fn admit(store: &dyn UnitStore, req: &QueryRequest, state: &RefCell<Admission>, u: UnitId) -> bool {
    let mut st = state.borrow_mut();
    if st.stale.contains(&u) {
        return false;
    }
    if u >= store.n_units() {
        tracing::warn!(unit = u, "posting refers to a unit the store does not have");
        st.stale.insert(u);
        return false;
    }
    let r = store.record(u);
    let fits = r.lang == req.lang
        && req.kind.is_none_or(|k| r.kind == k)
        && (req.include_tests || !r.is_test)
        && r.n_leaves <= MAX_PRUNE_LEAVES;
    if !fits || st.hashes.contains(&r.body_hash) {
        st.skipped.insert(u);
        return false;
    }
    if st.spts.contains_key(&u) {
        st.hashes.insert(r.body_hash);
        return true;
    }
    match store.spt(u) {
        Ok(spt) => {
            st.hashes.insert(r.body_hash);
            st.spts.insert(u, spt);
            true
        }
        Err(e) => {
            tracing::warn!(unit = u, error = %e, "skipping unit whose stored SPT failed to load");
            st.stale.insert(u);
            false
        }
    }
}

/// One fused candidate: its 1-based Phase I and BM25 ranks.
struct Fused {
    unit: UnitId,
    lexical_rank: Option<u32>,
    rrf: f64,
}

/// RRF fusion of the Phase I and BM25 rankings (`sum 1/(60 + rank)`, ties by unit id),
/// clones of a better-fused unit dropped (added to `skipped`), at most `cap` units.
fn fuse(
    structural: &[UnitId],
    lexical: &[UnitId],
    store: &dyn UnitStore,
    cap: usize,
    skipped: &mut HashSet<UnitId>,
) -> Vec<Fused> {
    let mut by_unit: HashMap<UnitId, Fused> = HashMap::new();
    for (i, &u) in structural.iter().enumerate() {
        by_unit
            .entry(u)
            .or_insert(Fused {
                unit: u,
                lexical_rank: None,
                rrf: 0.0,
            })
            .rrf += rrf(i + 1);
    }
    for (i, &u) in lexical.iter().enumerate() {
        let f = by_unit.entry(u).or_insert(Fused {
            unit: u,
            lexical_rank: None,
            rrf: 0.0,
        });
        f.rrf += rrf(i + 1);
        f.lexical_rank = Some(i as u32 + 1);
    }
    let mut fused: Vec<Fused> = by_unit.into_values().collect();
    fused.sort_unstable_by(|a, b| b.rrf.total_cmp(&a.rrf).then(a.unit.cmp(&b.unit)));
    let mut hashes: HashSet<u64> = HashSet::new();
    let mut out = Vec::with_capacity(cap.min(fused.len()));
    for f in fused {
        if out.len() == cap {
            break;
        }
        if hashes.insert(store.record(f.unit).body_hash) {
            out.push(f);
        } else {
            skipped.insert(f.unit);
        }
    }
    out
}

/// The lexical side of one query: its terms and the BM25 source.
struct LexicalQuery<'a> {
    src: &'a dyn LexicalSource,
    /// `(term id, term)` sorted by id: the BM25 summation order.
    terms: Vec<(TermId, String)>,
}

impl LexicalQuery<'_> {
    /// BM25 score of `unit` and the query terms it holds (sorted), summed in the same
    /// order as `lexical::bm25_scores`, so the value equals the ranking score.
    fn explain(&self, unit: UnitId) -> (f64, Vec<String>) {
        let n = self.src.n_units();
        let avgdl = self.src.avgdl();
        let dl = self.src.doc_len(unit);
        let mut score = 0.0;
        let mut matched = Vec::new();
        for (t, text) in &self.terms {
            if let Some(tf) = term_frequency(self.src, *t, unit) {
                let df = self.src.postings(*t).len() as u32;
                score += bm25_term(idf(n, df), tf, dl, avgdl);
                matched.push(text.clone());
            }
        }
        matched.sort();
        (score, matched)
    }
}

/// Upper bound on worker threads for [load] + [rerank] in one query.
const MAX_QUERY_THREADS: usize = 8;
/// Below this many candidates per thread, threads cost more than they save.
const MIN_CANDIDATES_PER_THREAD: usize = 8;

/// Featurize each candidate (`[load]`) and rerank it against `q` with no score floor, on
/// up to `threads` scoped threads. Contiguous chunks keep rank order: per-chunk results
/// are concatenated in chunk order and stably re-sorted by score, which reproduces the
/// sequential tie order exactly (`prune::rerank` is a stable sort over independent
/// per-candidate prunes). On cancellation the first cancelled chunk's stage is reported
/// (`"load"` or the rerank stage); results of finished chunks are kept.
fn load_and_rerank(
    q: &Bag,
    candidates: Vec<(UnitId, Spt)>,
    lang: Lang,
    cfg: &FeatureConfig,
    obj: Objective,
    deadline: &Deadline,
    threads: usize,
) -> (Vec<Candidate>, Option<Cancelled>) {
    let work = |chunk: Vec<(UnitId, Spt)>| -> (Vec<Candidate>, Option<Cancelled>) {
        let mut hits = Vec::with_capacity(chunk.len());
        for (u, spt) in chunk {
            if let Err(c) = deadline.check(LOAD) {
                return (Vec::new(), Some(c));
            }
            let bag = unit_bag(lang, &featurize(&spt, cfg, None));
            hits.push((u, spt, bag));
        }
        rerank(q, hits, lang, cfg, obj, NO_SCORE_FLOOR, deadline)
    };
    let threads = threads.clamp(1, (candidates.len() / MIN_CANDIDATES_PER_THREAD).max(1));
    if threads == 1 {
        return work(candidates);
    }
    let chunk_len = candidates.len().div_ceil(threads);
    let mut chunks = Vec::with_capacity(threads);
    let mut rest = candidates.into_iter();
    loop {
        let chunk: Vec<_> = rest.by_ref().take(chunk_len).collect();
        if chunk.is_empty() {
            break;
        }
        chunks.push(chunk);
    }
    let work = &work;
    let results: Vec<(Vec<Candidate>, Option<Cancelled>)> = std::thread::scope(|s| {
        let handles: Vec<_> = chunks
            .into_iter()
            .map(|chunk| s.spawn(move || work(chunk)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("load/rerank worker panicked"))
            .collect()
    });
    let mut out = Vec::new();
    let mut cancelled = None;
    for (mut part, c) in results {
        out.append(&mut part);
        if cancelled.is_none() {
            cancelled = c;
        }
    }
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    (out, cancelled)
}

/// Final display order. Admission, structural ranks and recommendation inputs are
/// unchanged; the lexical-off path returns the original structural order.
fn similar_order(
    cands: &[Candidate],
    lexical_ranks: &HashMap<UnitId, u32>,
    lex: Option<&LexicalQuery<'_>>,
    query: &Spt,
    unit_lines: impl Fn(UnitId) -> u32,
) -> Vec<usize> {
    let Some(lex) = lex else {
        return (0..cands.len()).collect();
    };
    // A repaired snippet may end with synthetic closing delimiters. Compare the
    // ordered tokens through its final real leaf, retaining operators and keywords.
    // Tiny snippets do not supply enough evidence to override fusion.
    let mut prefix = query.tokens(None);
    let end = query
        .leaves
        .last()
        .filter(|_| query.leaves.len() >= 3)
        .and_then(|last| prefix.iter().rposition(|t| *t == last.text.as_ref()))
        .map_or(0, |i| i + 1);
    prefix.truncate(end);

    struct DisplayRank {
        index: usize,
        prefix: bool,
        matching_gaps: usize,
        score: f64,
        unit_lines: u32,
    }
    let mut order: Vec<_> = cands
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let exact = !prefix.is_empty()
                && c.spt.leaves.len() >= query.leaves.len()
                && query
                    .leaves
                    .iter()
                    .zip(&c.spt.leaves)
                    .all(|(a, b)| a.text == b.text)
                && c.spt.tokens(None).starts_with(&prefix);
            let matching_gaps = if exact
                && query.leaves.iter().all(|l| !l.bytes.is_empty())
                && c.spt.leaves[..query.leaves.len()]
                    .iter()
                    .all(|l| !l.bytes.is_empty())
            {
                query
                    .leaves
                    .windows(2)
                    .zip(c.spt.leaves.windows(2))
                    .filter(|(a, b)| {
                        a[1].bytes.start.saturating_sub(a[0].bytes.end)
                            == b[1].bytes.start.saturating_sub(b[0].bytes.end)
                    })
                    .count()
            } else {
                0
            };
            let score = if exact {
                // Among literal prefixes, prefer concentrated lexical evidence over
                // repetitions elsewhere in a much longer candidate.
                lex.explain(c.unit).0 / c.spt.leaves.len() as f64
            } else {
                rrf(i + 1) + lexical_ranks.get(&c.unit).map_or(0.0, |&r| rrf(r as usize))
            };
            DisplayRank {
                index: i,
                prefix: exact,
                matching_gaps,
                score,
                unit_lines: if exact { unit_lines(c.unit) } else { 0 },
            }
        })
        .collect();
    order.sort_by(|a, b| {
        b.prefix
            .cmp(&a.prefix)
            .then(b.matching_gaps.cmp(&a.matching_gaps))
            .then(b.score.total_cmp(&a.score))
            // With otherwise equal prefix evidence, retain more extension.
            .then(b.unit_lines.cmp(&a.unit_lines))
            .then(cands[a.index].unit.cmp(&cands[b.index].unit))
    });
    order.into_iter().map(|r| r.index).collect()
}

pub fn run_query(
    idx: &dyn PostingSource,
    store: &dyn UnitStore,
    req: &QueryRequest,
) -> QueryResult {
    let deadline = req.budget.map_or_else(Deadline::none, Deadline::after);
    let cfg = &req.profile.features;
    let params = &req.profile.params;
    let q = unit_bag(req.lang, &featurize(req.spt, cfg, None));
    let mut out = QueryResult {
        similar: Vec::new(),
        recommendations: Vec::new(),
        diagnostics: Diagnostics {
            query_features: q.distinct().count() as u32,
            ..Diagnostics::default()
        },
    };

    // [search] with lazy eligibility, clone dedup and stale detection before the cap.
    let admission = RefCell::new(Admission::default());
    let found = {
        let eligible = |u: UnitId| admit(store, req, &admission, u);
        search(
            idx,
            &q,
            params.min_similarity,
            params.num_similars,
            &eligible,
        )
    };
    let report = |out: &mut QueryResult, a: &RefCell<Admission>| {
        let a = a.borrow();
        out.diagnostics.stale_units = a.stale.len() as u32;
        out.diagnostics.ineligible_skipped = a.skipped.len() as u32;
    };
    out.diagnostics.known_query_features = found.known_query_features;
    report(&mut out, &admission);
    if deadline.check(SEARCH).is_err() {
        out.diagnostics.budget_exhausted = Some(SEARCH);
        return out;
    }
    let structural: Vec<UnitId> = found.hits.iter().map(|h| h.unit).collect();

    // [lexical] BM25 ranking with the same lazy eligibility, then RRF fusion.
    let lex = req.lexical.map(|src| LexicalQuery {
        src,
        terms: {
            let mut v: Vec<(TermId, String)> = spt_terms(req.spt, req.term_rule)
                .into_keys()
                .map(|t| (term_id(&t), t))
                .collect();
            v.sort_unstable();
            v.dedup_by_key(|(id, _)| *id);
            v
        },
    });
    let (candidates, lexical_ranks): (Vec<UnitId>, HashMap<UnitId, u32>) = match &lex {
        None => (structural, HashMap::new()),
        Some(lq) => {
            admission.borrow_mut().hashes.clear();
            let ids: Vec<TermId> = lq.terms.iter().map(|(t, _)| *t).collect();
            let lexical = {
                let eligible = |u: UnitId| admit(store, req, &admission, u);
                lexical_search(lq.src, &ids, params.num_similars, &eligible)
            };
            let d = &mut out.diagnostics;
            d.lexical = true;
            d.query_terms = ids.len() as u32;
            d.known_query_terms = lexical.known_query_terms;
            let lexical: Vec<UnitId> = lexical.hits.iter().map(|h| h.unit).collect();
            let fused = fuse(
                &structural,
                &lexical,
                store,
                params.num_similars,
                &mut admission.borrow_mut().skipped,
            );
            {
                let mut a = admission.borrow_mut();
                for f in &fused {
                    a.skipped.remove(&f.unit);
                }
            }
            report(&mut out, &admission);
            if deadline.check(LEXICAL).is_err() {
                out.diagnostics.budget_exhausted = Some(LEXICAL);
                return out;
            }
            let ranks = fused
                .iter()
                .filter_map(|f| f.lexical_rank.map(|r| (f.unit, r)))
                .collect();
            (fused.into_iter().map(|f| f.unit).collect(), ranks)
        }
    };
    let mut spts = std::mem::take(&mut admission.borrow_mut().spts);

    // [load] unpruned bags of the candidates + [rerank] every admitted unit, no score
    // floor (D30), in Phase I (or fused) rank order. Both are per-candidate work, so they
    // run on up to MAX_QUERY_THREADS threads with a result identical to the sequential
    // one (see `load_and_rerank`); partial results survive a cancellation.
    let todo: Vec<(UnitId, Spt)> = candidates
        .iter()
        .filter_map(|&u| spts.remove(&u).map(|spt| (u, spt)))
        .collect();
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(MAX_QUERY_THREADS);
    let (mut cands, cancelled) = load_and_rerank(
        &q,
        todo,
        req.lang,
        cfg,
        req.profile.objective,
        &deadline,
        threads,
    );
    if cancelled.is_some_and(|c| c.stage == LOAD) {
        out.diagnostics.budget_exhausted = Some(LOAD);
        return out;
    }
    let order = similar_order(&cands, &lexical_ranks, lex.as_ref(), req.spt, |u| {
        let lines = store.record(u).lines;
        lines.end - lines.start
    });
    out.similar = order
        .iter()
        .take(req.top_similar)
        .map(|&i| {
            let c = &cands[i];
            let (bm25, matched_terms) = match &lex {
                Some(lq) => {
                    let (score, terms) = lq.explain(c.unit);
                    (Some(score), terms)
                }
                None => (None, Vec::new()),
            };
            Similar {
                unit: c.unit,
                score: c.score,
                kept: c.pruned.kept.clone(),
                structural_rank: i as u32 + 1,
                lexical_rank: lexical_ranks.get(&c.unit).copied(),
                bm25,
                matched_terms,
            }
        })
        .collect();
    if cancelled.is_some() {
        out.diagnostics.budget_exhausted = Some(RERANK);
        return out;
    }
    if req.experimental_recommend {
        // [cluster] input stays the reference one: only `score > min_pruned`, in
        // rerank order (a stable filter of the score-sorted list).
        cands.retain(|c| c.score > params.min_pruned);
        recommend(store, req, &q, &cands, &deadline, &mut out);
    }
    out
}

/// [cluster] + [intersect] + gate. Support is checked before the (costly) intersect;
/// the result is the same as gating afterwards.
fn recommend(
    store: &dyn UnitStore,
    req: &QueryRequest,
    q: &Bag,
    cands: &[Candidate],
    deadline: &Deadline,
    out: &mut QueryResult,
) {
    let params = &req.profile.params;
    let tuples = match cluster(cands, params.tau3, params.tau2, params.top_n, deadline) {
        Ok(t) => t,
        Err(_) => {
            out.diagnostics.budget_exhausted = Some(CLUSTER);
            return;
        }
    };
    let body_hash = |u: UnitId| store.record(u).body_hash;
    let query_leaves = req.spt.leaves.len();
    for tuple in tuples {
        let support = independent_support(&tuple, cands, &body_hash);
        if support < req.gate.min_independent {
            continue;
        }
        let kept = match intersect(q, &tuple, cands, req.lang, &req.profile.features, deadline) {
            Ok(k) => k,
            Err(_) => {
                out.diagnostics.budget_exhausted = Some(INTERSECT);
                return;
            }
        };
        if passes_gate(&req.gate, support, &kept, query_leaves) {
            out.recommendations.push(Recommendation {
                members: tuple.iter().map(|&i| cands[i].unit).collect(),
                independent_support: support,
                kept,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spt::{Leaf, Raw};

    fn leaf(s: &str, is_var: bool) -> Raw {
        Raw::Leaf(Leaf {
            text: s.into(),
            is_var,
            binding: None,
            bytes: 0..0,
            line: 1,
        })
    }

    /// `n` statements `a_i = r.m_j(x_k);` drawn deterministically from small vocabularies,
    /// so many candidates share structure and some tie on score.
    fn unit(seed: u64, n: usize) -> Spt {
        let names = ["get", "put", "add", "size", "close"];
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let stmts = (0..n)
            .map(|_| {
                let m = names[(next() % names.len() as u64) as usize];
                Raw::List(vec![
                    leaf("a", true),
                    Raw::Keyword("=".into()),
                    leaf("r", true),
                    Raw::Keyword(".".into()),
                    Raw::List(vec![
                        leaf(m, false),
                        Raw::Keyword("(".into()),
                        leaf("x", true),
                        Raw::Keyword(")".into()),
                    ]),
                    Raw::Keyword(";".into()),
                ])
            })
            .collect();
        Spt::build(Raw::List(stmts)).unwrap()
    }

    #[test]
    fn parallel_load_and_rerank_equals_sequential() {
        let cfg = FeatureConfig::DEFAULT;
        let q = unit_bag(Lang::Java, &featurize(&unit(7, 4), &cfg, None));
        let candidates: Vec<(UnitId, Spt)> = (0..90u32)
            .map(|u| (u, unit(u64::from(u % 23), 2 + (u as usize % 9))))
            .collect();
        let run = |threads: usize| {
            load_and_rerank(
                &q,
                candidates.clone(),
                Lang::Java,
                &cfg,
                Objective::Jaccard,
                &Deadline::none(),
                threads,
            )
        };
        let (seq, c1) = run(1);
        assert!(c1.is_none());
        assert_eq!(seq.len(), 90);
        assert!(
            seq.windows(2).any(|w| w[0].score == w[1].score),
            "fixture must have ties"
        );
        for threads in [2, 3, 4, 8] {
            let (par, c) = run(threads);
            assert!(c.is_none());
            let key = |v: &[Candidate]| -> Vec<(UnitId, u64, Vec<bool>)> {
                v.iter()
                    .map(|c| (c.unit, c.score.to_bits(), c.pruned.kept.clone()))
                    .collect()
            };
            assert_eq!(key(&par), key(&seq), "threads {threads}");
        }
    }
    fn calls(names: &[&str]) -> Spt {
        let mut statements = vec![Raw::Keyword("{".into())];
        for name in names {
            statements.push(Raw::List(vec![
                leaf("client", true),
                Raw::Keyword(".".into()),
                leaf(name, false),
                Raw::Keyword("(".into()),
                leaf("value", true),
                Raw::Keyword(")".into()),
                Raw::Keyword(";".into()),
            ]));
        }
        statements.push(Raw::Keyword("}".into()));
        Spt::build(Raw::List(statements)).unwrap()
    }

    fn display_candidate(unit: UnitId, spt: Spt, score: f64) -> Candidate {
        Candidate {
            unit,
            pruned: crate::prune::Pruned {
                kept: vec![true; spt.leaves.len()],
                bag: Bag::default(),
            },
            spt,
            bag: Bag::default(),
            score,
        }
    }

    fn display_units(q: &Spt, cands: &[Candidate], lexical: bool) -> Vec<UnitId> {
        use crate::lexical::{unit_terms, MemLexical};
        let mut src = MemLexical::default();
        for c in cands {
            src.insert(c.unit, &unit_terms(&c.spt, TermRule::Identifier));
        }
        let mut terms: Vec<_> = spt_terms(q, TermRule::Identifier)
            .into_keys()
            .map(|t| (term_id(&t), t))
            .collect();
        terms.sort_unstable();
        let lex = LexicalQuery { src: &src, terms };
        let ranks = cands
            .iter()
            .enumerate()
            .map(|(i, c)| (c.unit, i as u32 + 1))
            .collect();
        similar_order(cands, &ranks, lexical.then_some(&lex), q, |u| {
            cands
                .iter()
                .find(|c| c.unit == u)
                .unwrap()
                .spt
                .leaves
                .last()
                .map_or(0, |l| l.line)
        })
        .into_iter()
        .map(|i| cands[i].unit)
        .collect()
    }

    #[test]
    fn exact_prefix_beats_a_stronger_fusion_rank_only_with_lexical() {
        let q = calls(&["open", "read"]);
        let cands = [
            display_candidate(0, calls(&["other", "read"]), 1.0),
            display_candidate(1, calls(&["open", "read", "close"]), 0.9),
        ];
        assert_eq!(display_units(&q, &cands, false), [0, 1]);
        assert_eq!(display_units(&q, &cands, true), [1, 0]);
    }

    #[test]
    fn prefix_evidence_includes_keywords_not_just_leaf_names() {
        let q = calls(&["open", "read"]);
        let mut different = q.clone();
        for node in &mut different.nodes {
            for child in &mut node.children {
                if matches!(child, crate::spt::Elem::Keyword(k) if k.as_ref() == ".") {
                    *child = crate::spt::Elem::Keyword("::".into());
                }
            }
        }
        let cands = [
            display_candidate(0, different, 1.0),
            display_candidate(1, calls(&["open", "read", "close"]), 0.9),
        ];
        assert_eq!(display_units(&q, &cands, true), [1, 0]);
    }

    #[test]
    fn exact_prefixes_prefer_lexical_density_to_repeated_query_terms() {
        let q = calls(&["open", "read"]);
        let cands = [
            display_candidate(
                0,
                calls(&["open", "read", "open", "read", "open", "read"]),
                1.0,
            ),
            display_candidate(1, calls(&["open", "read", "close"]), 1.0),
        ];
        assert_eq!(display_units(&q, &cands, true), [1, 0]);
    }

    fn spaced(mut spt: Spt, stride: u32) -> Spt {
        for (i, leaf) in spt.leaves.iter_mut().enumerate() {
            let start = 1 + i as u32 * stride;
            leaf.bytes = start..start + leaf.text.len() as u32;
        }
        spt
    }

    #[test]
    fn prefix_layout_agreement_precedes_lexical_density() {
        let q = spaced(calls(&["open", "read"]), 30);
        let cands = [
            display_candidate(0, spaced(calls(&["open", "read"]), 40), 1.0),
            display_candidate(1, spaced(calls(&["open", "read", "close"]), 30), 0.9),
        ];
        assert_eq!(display_units(&q, &cands, true), [1, 0]);
    }

    #[test]
    fn prefix_evidence_does_not_promote_an_interior_match() {
        let q = calls(&["open", "read"]);
        let cands = [
            display_candidate(0, calls(&["other", "read"]), 1.0),
            display_candidate(1, calls(&["other", "open", "read"]), 0.9),
        ];
        assert_eq!(display_units(&q, &cands, true), [0, 1]);
    }

    #[test]
    fn empty_and_tiny_queries_keep_rrf_order() {
        let cands = [
            display_candidate(0, calls(&["other", "read"]), 1.0),
            display_candidate(1, calls(&["open", "read"]), 0.9),
        ];
        let empty = Spt::build(Raw::List(vec![Raw::Keyword("{}".into())])).unwrap();
        assert_eq!(display_units(&empty, &cands, true), [0, 1]);
        let tiny = Spt::build(Raw::List(vec![
            Raw::Keyword("{".into()),
            leaf("client", true),
            Raw::Keyword("}".into()),
        ]))
        .unwrap();
        let tiny_cands = [
            display_candidate(0, calls(&["other", "read", "close"]), 1.0),
            display_candidate(1, calls(&["open"]), 0.9),
        ];
        assert_eq!(display_units(&tiny, &tiny_cands, true), [0, 1]);
        assert!(display_units(&empty, &[], true).is_empty());
    }

    #[test]
    fn token_text_without_matching_leaves_is_not_prefix_evidence() {
        let q = spaced(calls(&["open", "read"]), 30);
        let mut keywords = q.clone();
        for node in &mut keywords.nodes {
            for elem in &mut node.children {
                if let crate::spt::Elem::Leaf(i) = elem {
                    *elem = crate::spt::Elem::Keyword(keywords.leaves[*i as usize].text.clone());
                }
            }
        }
        keywords.leaves.clear();
        let cands = [
            display_candidate(0, keywords, 1.0),
            display_candidate(1, spaced(calls(&["open", "read", "close"]), 30), 0.9),
        ];
        assert_eq!(display_units(&q, &cands, true), [1, 0]);
    }

    #[test]
    fn otherwise_equal_prefixes_prefer_more_lines_of_extension() {
        let q = calls(&["open", "read"]);
        let short = calls(&["open", "read", "close"]);
        let mut long = short.clone();
        long.leaves.last_mut().unwrap().line = 5;
        let cands = [
            display_candidate(0, short, 1.0),
            display_candidate(1, long, 1.0),
        ];
        assert_eq!(display_units(&q, &cands, true), [1, 0]);
    }

    #[test]
    fn equal_prefix_evidence_uses_unit_id_as_the_final_tie_break() {
        let q = calls(&["open", "read"]);
        let cands = [
            display_candidate(7, calls(&["open", "read", "close"]), 1.0),
            display_candidate(2, calls(&["open", "read", "close"]), 1.0),
        ];
        assert_eq!(display_units(&q, &cands, true), [2, 7]);
    }
}
