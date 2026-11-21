//! BM25 identifier channel (T23, D30): a lexical ranking fused with the structural one.
//!
//! ```text
//!  Spt leaves --identifier?--> whole term + camelCase/_/digit parts (lowercased,
//!             len >= 2, not a keyword) --> term multiset of the unit (tf, doc length)
//!  query terms --[lexical_search]--> BM25(k1 1.2, b 0.75, idf ln(1 + (N-df+.5)/(df+.5)))
//!             ranked (score desc, unit asc), eligibility decided lazily in rank order
//!  fusion: rrf(rank) = 1 / (60 + rank), summed over the rankings a unit appears in
//! ```
//!
//! Only leaves spelled like identifiers (`[A-Za-z_$][A-Za-z0-9_$]*`, Unicode letters
//! allowed) produce terms, so string, character and number literals never reach the
//! lexical index (privacy constraint). Apex leaves arrive ASCII-folded, so their
//! camelCase boundaries are already gone and only `_`/digit splits apply there.

use crate::features::FEATURE_SEED;
use crate::spt::Spt;
use crate::unit::UnitId;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use xxhash_rust::xxh3::xxh3_64_with_seed;

/// `xxh3_64_with_seed(term, TERM_SEED)`; never persisted as a vocabulary.
pub type TermId = u64;

pub const TERM_SEED: u64 = FEATURE_SEED ^ 1;
pub const K1: f64 = 1.2;
pub const B: f64 = 0.75;
/// Reciprocal-rank-fusion constant: `rrf(rank) = 1 / (RRF_K + rank)`, ranks 1-based.
pub const RRF_K: f64 = 60.0;
/// Terms shorter than this (in chars) are dropped.
pub const MIN_TERM_LEN: usize = 2;

/// Java and Apex keywords (and literal keywords) that may surface as leaves; never terms.
pub const STOP_WORDS: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "false",
    "final",
    "finally",
    "float",
    "for",
    "global",
    "goto",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "null",
    "override",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "sharing",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "testmethod",
    "this",
    "throw",
    "throws",
    "transient",
    "true",
    "try",
    "var",
    "virtual",
    "void",
    "volatile",
    "while",
    "with",
    "without",
];

pub fn term_id(term: &str) -> TermId {
    xxh3_64_with_seed(term.as_bytes(), TERM_SEED)
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// True when `text` is spelled like an identifier (literals are not).
pub fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(is_ident_start) && chars.all(is_ident_char)
}

fn keep(term: &str) -> bool {
    term.chars().count() >= MIN_TERM_LEN
        && !term.chars().all(|c| c.is_numeric())
        && STOP_WORDS.binary_search(&term).is_err()
}

/// camelCase / `_` / `$` / digit-run parts of an identifier, original case.
/// `HTTPRequest2xx` -> `HTTP`, `Request`, `2`, `xx`.
fn split_parts(text: &str) -> Vec<&str> {
    #[derive(PartialEq, Clone, Copy)]
    enum Class {
        Upper,
        Lower,
        Digit,
        Sep,
    }
    let class = |c: char| {
        if c.is_numeric() {
            Class::Digit
        } else if c.is_uppercase() {
            Class::Upper
        } else if c.is_alphabetic() {
            Class::Lower
        } else {
            Class::Sep
        }
    };
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut parts = Vec::new();
    let mut start: Option<usize> = None;
    for (i, &(at, c)) in chars.iter().enumerate() {
        let cl = class(c);
        if cl == Class::Sep {
            if let Some(s) = start.take() {
                parts.push(&text[s..at]);
            }
            continue;
        }
        if let Some(s) = start {
            let prev = class(chars[i - 1].1);
            let next = chars.get(i + 1).map(|&(_, n)| class(n));
            let boundary = match (prev, cl) {
                (Class::Digit, Class::Digit) => false,
                (Class::Digit, _) | (_, Class::Digit) => true,
                (Class::Lower, Class::Upper) => true,
                // `HTTPRequest`: the last capital of a run starts the next word.
                (Class::Upper, Class::Upper) => next == Some(Class::Lower),
                _ => false,
            };
            if boundary {
                parts.push(&text[s..at]);
                start = Some(at);
            }
        } else {
            start = Some(at);
        }
    }
    if let Some(s) = start {
        parts.push(&text[s..]);
    }
    parts
}

/// Distinct terms of one identifier leaf: the lowercased whole spelling, then its
/// lowercased camelCase / `_` / digit parts, each kept only when `len >= 2`, not all
/// digits and not a stop word. Empty for anything that is not an identifier.
pub fn identifier_terms(text: &str) -> Vec<String> {
    if !is_identifier(text) {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    let whole = text.to_lowercase();
    if keep(&whole) {
        out.push(whole);
    }
    for p in split_parts(text) {
        let p = p.to_lowercase();
        if keep(&p) && !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// How token text contributes lexical terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermRule {
    /// The existing identifier-only, camelCase-aware code rule.
    Identifier,
    /// Split markup names and expressions while retaining compound token spellings.
    Markup,
}

fn markup_separator(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '-' | '.'
                | ':'
                | '{'
                | '}'
                | '!'
                | '$'
                | '/'
                | '('
                | ')'
                | '['
                | ']'
                | ','
                | '"'
                | '\''
                | '='
                | '<'
                | '>'
                | '@'
                | '#'
        )
}

impl TermRule {
    /// Distinct terms from one token, lowercased and filtered by the shared rules.
    pub fn terms(self, text: &str) -> Vec<String> {
        if self == Self::Identifier {
            return identifier_terms(text);
        }
        let mut out = Vec::new();
        for piece in text.split(markup_separator).filter(|p| !p.is_empty()) {
            for term in identifier_terms(piece) {
                if !out.contains(&term) {
                    out.push(term);
                }
            }
        }
        if text.contains(markup_separator) {
            let whole = text.trim_matches(markup_separator).to_lowercase();
            if keep(&whole) && !out.contains(&whole) {
                out.push(whole);
            }
        }
        out
    }
}

/// Term frequencies of a unit (or query) tree: every leaf adds 1 to each of its
/// distinct terms under `rule`.
pub fn spt_terms(spt: &Spt, rule: TermRule) -> BTreeMap<String, u32> {
    let mut tf: BTreeMap<String, u32> = BTreeMap::new();
    for leaf in &spt.leaves {
        for t in rule.terms(&leaf.text) {
            *tf.entry(t).or_insert(0) += 1;
        }
    }
    tf
}

/// `(term id, tf)` sorted by term id (what `lexical.bin` stores per unit).
pub fn unit_terms(spt: &Spt, rule: TermRule) -> Vec<(TermId, u32)> {
    let mut v: Vec<(TermId, u32)> = Vec::new();
    for (t, n) in spt_terms(spt, rule) {
        v.push((term_id(&t), n));
    }
    v.sort_unstable();
    // Distinct strings with equal hashes (practically never) are merged.
    v.dedup_by(|b, a| {
        if a.0 == b.0 {
            a.1 += b.1;
            true
        } else {
            false
        }
    });
    v
}

/// BM25 document length: the sum of a unit's term frequencies.
pub fn doc_len(terms: &[(TermId, u32)]) -> u32 {
    terms.iter().map(|&(_, n)| n).sum()
}

/// Robertson–Spärck Jones idf with +1 smoothing: `ln(1 + (N - df + 0.5) / (df + 0.5))`.
pub fn idf(n_units: u32, df: u32) -> f64 {
    let (n, df) = (f64::from(n_units), f64::from(df));
    (1.0 + (n - df + 0.5) / (df + 0.5)).ln()
}

/// One term's BM25 contribution.
pub fn bm25_term(idf: f64, tf: u32, dl: u32, avgdl: f64) -> f64 {
    let tf = f64::from(tf);
    let norm = if avgdl > 0.0 {
        K1 * (1.0 - B + B * f64::from(dl) / avgdl)
    } else {
        K1
    };
    idf * tf * (K1 + 1.0) / (tf + norm)
}

/// Reciprocal rank fusion term for a 1-based rank.
pub fn rrf(rank: usize) -> f64 {
    1.0 / (RRF_K + rank as f64)
}

/// The lexical index a query reads: implemented by `store::lexical::MmapLexical` and by
/// [`MemLexical`] (tests).
pub trait LexicalSource: Sync {
    fn n_units(&self) -> u32;
    /// Mean document length over all units (0 for an empty index).
    fn avgdl(&self) -> f64;
    /// Document length of `unit` (0 when out of range).
    fn doc_len(&self, unit: UnitId) -> u32;
    /// `[unit, tf]` pairs with ascending unit ids; empty when the term is unknown.
    fn postings(&self, term: TermId) -> &[[u32; 2]];
}

/// In-memory lexical index for tests.
#[derive(Default)]
pub struct MemLexical {
    postings: HashMap<TermId, Vec<[u32; 2]>>,
    doc_len: Vec<u32>,
}

impl MemLexical {
    /// Units must be inserted in increasing id order; skipped ids have no terms.
    pub fn insert(&mut self, unit: UnitId, terms: &[(TermId, u32)]) {
        for &(t, n) in terms {
            self.postings.entry(t).or_default().push([unit, n]);
        }
        let u = unit as usize;
        if self.doc_len.len() <= u {
            self.doc_len.resize(u + 1, 0);
        }
        self.doc_len[u] = doc_len(terms);
    }
}

/// `sum(doc_len) / n_units`, 0 for no units (shared by the writer and `MemLexical`).
pub fn mean_doc_len(doc_len: &[u32]) -> f64 {
    if doc_len.is_empty() {
        return 0.0;
    }
    let total: u64 = doc_len.iter().map(|&d| u64::from(d)).sum();
    total as f64 / doc_len.len() as f64
}

impl LexicalSource for MemLexical {
    fn n_units(&self) -> u32 {
        self.doc_len.len() as u32
    }

    fn avgdl(&self) -> f64 {
        mean_doc_len(&self.doc_len)
    }

    fn doc_len(&self, unit: UnitId) -> u32 {
        self.doc_len.get(unit as usize).copied().unwrap_or(0)
    }

    fn postings(&self, term: TermId) -> &[[u32; 2]] {
        self.postings.get(&term).map_or(&[], Vec::as_slice)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LexHit {
    pub unit: UnitId,
    pub score: f64,
}

pub struct LexicalOutcome {
    /// At most `cap` eligible hits, ranked `(score desc, unit asc)`.
    pub hits: Vec<LexHit>,
    /// Distinct query terms with a non-empty posting list.
    pub known_query_terms: u32,
}

thread_local! {
    static SCORES: RefCell<Vec<f64>> = const { RefCell::new(Vec::new()) };
}

fn rank(a: &LexHit, b: &LexHit) -> Ordering {
    b.score.total_cmp(&a.score).then(a.unit.cmp(&b.unit))
}

/// BM25 scores of every unit sharing a term with `query` (distinct term ids, summed in
/// the given order), unordered; and the number of known terms.
pub fn bm25_scores(src: &dyn LexicalSource, query: &[TermId]) -> (Vec<LexHit>, u32) {
    SCORES.with(|cell| {
        let mut scores = cell.borrow_mut();
        let n = src.n_units() as usize;
        if scores.len() < n {
            scores.resize(n, 0.0);
        }
        let avgdl = src.avgdl();
        let mut touched: Vec<UnitId> = Vec::new();
        let mut known = 0u32;
        for &t in query {
            let list = src.postings(t);
            if list.is_empty() {
                continue;
            }
            known += 1;
            let w = idf(src.n_units(), list.len() as u32);
            for &[u, tf] in list {
                let Some(s) = scores.get_mut(u as usize) else {
                    continue;
                };
                if *s == 0.0 {
                    touched.push(u);
                }
                *s += bm25_term(w, tf, src.doc_len(u), avgdl);
            }
        }
        let mut hits = Vec::with_capacity(touched.len());
        for &u in &touched {
            let s = &mut scores[u as usize];
            if *s > 0.0 {
                hits.push(LexHit { unit: u, score: *s });
            }
            *s = 0.0;
        }
        // A unit whose running score stayed 0 (tf 0) may be listed twice in `touched`.
        hits.sort_unstable_by_key(|h| h.unit);
        hits.dedup_by_key(|h| h.unit);
        (hits, known)
    })
}

/// Distinct query term ids, sorted (the summation order of `bm25_scores`).
pub fn query_term_ids(terms: &BTreeMap<String, u32>) -> Vec<TermId> {
    let mut ids: Vec<TermId> = terms.keys().map(|t| term_id(t)).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// The best `cap` eligible units by BM25. `eligible` is called in rank order, at most
/// once per unit, never after the cap is filled (the contract of `index::search`).
pub fn lexical_search(
    src: &dyn LexicalSource,
    query: &[TermId],
    cap: usize,
    eligible: &dyn Fn(UnitId) -> bool,
) -> LexicalOutcome {
    let (mut scored, known) = bm25_scores(src, query);
    let mut hits = Vec::with_capacity(cap.min(scored.len()));
    let mut start = 0;
    while start < scored.len() && hits.len() < cap {
        let rest = &mut scored[start..];
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
            }
        }
        start += window;
    }
    LexicalOutcome {
        hits,
        known_query_terms: known,
    }
}

/// Binary search of `unit` in a term's posting list.
pub fn term_frequency(src: &dyn LexicalSource, term: TermId, unit: UnitId) -> Option<u32> {
    let list = src.postings(term);
    list.binary_search_by_key(&unit, |p| p[0])
        .ok()
        .map(|i| list[i][1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_words_are_sorted_for_binary_search() {
        assert!(STOP_WORDS.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn scores_are_reset_after_each_query() {
        let mut m = MemLexical::default();
        m.insert(0, &[(1, 1)]);
        m.insert(2, &[(1, 2)]);
        let (hits, known) = bm25_scores(&m, &[1, 7]);
        assert_eq!(known, 1);
        assert_eq!(hits.iter().map(|h| h.unit).collect::<Vec<_>>(), [0, 2]);
        SCORES.with(|s| assert!(s.borrow().iter().all(|&x| x == 0.0)));
    }
}
