//! `Index` facade (T15) and the JSON output schema shared by the library, the CLI and
//! xtask.
//!
//! ```text
//! open:  .aroma/CURRENT -> gen-<n>/manifest.json -> check_compatible(expected from this
//!        binary's LangSpecs, stored profile + compat) -> mmap postings.bin,
//!        lexical.bin, units.bin, spt.bin; read globals.json
//!        (readers never take the writer lock)
//! query: parse_snippet(stored globals) -> run_query (BM25 identifier channel fused in
//!        unless `--no-lexical` or a `--compat` index, T23) -> render every result:
//!        file blake3 == indexed blake3 ? render_full_lines(source) : render_tokens
//!        (stale_source) -> QueryOutput (schema_version 1)
//! ```
//!
//! `SimilarOut::lines` are 1-based **inclusive** `[first, last]`; `kept_spans` are byte
//! ranges `[start, end)` into the file as indexed. `structural_rank`, `lexical_rank`,
//! `bm25` and `matched_terms` were added by T23 (additive: `schema_version` stays 1). `RecommendationOut::members` are
//! `"path:first-last"`; the recommendation text renders its first member.

use crate::build::{
    effective_profile, expected_manifest_with_mode, read_file_hashes, read_globals, var_mode,
    SfmetaMode, BUILD_REPORT_FILE, INVENTORY_FILE, LEXICAL_FILE, POSTINGS_FILE, SPT_FILE,
};
use crate::globals::Globals;
use crate::snippet::parse_snippet;
use crate::spec::spec_for;
use crate::table::Limits;
use aroma_core::cluster::RecGate;
use aroma_core::explain::{explain, Explanation};
use aroma_core::index::PostingSource;
use aroma_core::lang::Lang;
use aroma_core::lexical::{
    lexical_search, query_term_ids, rrf, spt_terms, term_frequency, term_id, LexicalSource,
};
use aroma_core::profile::{Profile, ProfileName};
use aroma_core::query::{run_query, Diagnostics, QueryRequest};
use aroma_core::render::{kept_spans, render_full_lines, render_tokens};
use aroma_core::spt::Spt;
use aroma_core::store::generation::IndexDir;
use aroma_core::store::lexical::MmapLexical;
use aroma_core::store::manifest::Manifest;
use aroma_core::store::postings::MmapPostings;
use aroma_core::store::spt_store::SptStore;
use aroma_core::store::unit_table::UnitTable;
use aroma_core::store::StoreError;
use aroma_core::unit::{UnitId, UnitKind, UnitRecord, UnitStore};
use serde::Serialize;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub const OUTPUT_SCHEMA_VERSION: u32 = 1;

pub struct Index {
    root: PathBuf,
    gen_dir: PathBuf,
    manifest: Manifest,
    profile: Profile,
    postings: MmapPostings,
    lexical: MmapLexical,
    units: UnitTable,
    spts: SptStore,
    globals: BTreeMap<Lang, Globals>,
    /// Per-file blake3 from `inventory.postcard`, loaded on the first render.
    file_hashes: OnceLock<Option<Vec<[u8; 32]>>>,
}

#[derive(Clone, Debug)]
pub struct QueryOptions {
    pub lang: Lang,
    pub top_similar: usize,
    pub experimental_recommend: bool,
    pub kind: Option<UnitKind>,
    pub include_tests: bool,
    pub budget: Option<Duration>,
    pub explain: bool,
    /// Fuse the BM25 identifier channel into the ranking (T23). Ignored (always off) on
    /// a `--compat` index, which runs the pure reference pipeline.
    pub lexical: bool,
}

impl QueryOptions {
    /// Ten similar results, no recommendations, no filters, no budget, no explain,
    /// lexical channel on.
    pub fn new(lang: Lang) -> QueryOptions {
        QueryOptions {
            lang,
            top_similar: 10,
            experimental_recommend: false,
            kind: None,
            include_tests: false,
            budget: None,
            explain: false,
            lexical: true,
        }
    }
}

/// One source file or snippet in a component query. Paths are diagnostic metadata;
/// even when all paths share a parent, that directory remains eligible as a hit.
#[derive(Clone, Debug)]
pub struct ComponentPart {
    pub lang: Lang,
    pub source: String,
    pub path: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ComponentQuery {
    pub parts: Vec<ComponentPart>,
    pub top: usize,
    /// Normal query options. Each part overrides `lang` and `top_similar` (100).
    /// Profile and compatibility come from the index, just as for `Index::query`.
    pub options: QueryOptions,
}

#[derive(Clone, Debug, Serialize)]
pub struct ComponentOutput {
    pub schema_version: u32,
    pub generation: u64,
    pub components: Vec<ComponentHit>,
    /// Diagnostics in the same order as the input parts.
    pub parts: Vec<PartDiagnostics>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ComponentHit {
    /// Repo-relative parent directory with `/` separators (`.` for the root).
    pub dir: String,
    pub score: f64,
    /// One slot per query part; only its best-ranked region in this directory.
    pub parts: Vec<Option<SimilarOut>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PartDiagnostics {
    pub lang: Lang,
    pub path: Option<String>,
    pub diagnostics: OutDiagnostics,
}

#[derive(Clone, Debug, Serialize)]
pub struct QueryOutput {
    pub schema_version: u32,
    pub generation: u64,
    pub profile: ProfileName,
    pub similar: Vec<SimilarOut>,
    pub recommendations: Vec<RecommendationOut>,
    pub diagnostics: OutDiagnostics,
}

#[derive(Clone, Debug, Serialize)]
pub struct SimilarOut {
    /// Repo-relative path, `/`-separated.
    pub path: String,
    /// First and last line of the unit, 1-based inclusive.
    pub lines: [u32; 2],
    pub kind: UnitKind,
    pub score: f64,
    pub text: String,
    /// Byte ranges `[start, end)` of the kept tokens in the indexed file.
    pub kept_spans: Vec<[u32; 2]>,
    /// The file changed since indexing: `text` shows the stored tokens.
    pub stale_source: bool,
    pub explain: Option<Explanation>,
    /// 1-based rank by structural rerank `score`.
    pub structural_rank: u32,
    /// 1-based rank among the BM25 identifier hits; `null` when the lexical channel is
    /// off or the unit is not among its best hits.
    pub lexical_rank: Option<u32>,
    /// BM25 score over the snippet's identifier terms; `null` when the channel is off.
    pub bm25: Option<f64>,
    /// Snippet identifier terms the unit contains (identifiers only, never literals).
    pub matched_terms: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RecommendationOut {
    /// `"path:first-last"` per cluster member; `text` renders the first one.
    pub members: Vec<String>,
    pub independent_support: usize,
    pub text: String,
    pub kept_spans: Vec<[u32; 2]>,
    pub stale_source: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct OutDiagnostics {
    pub parse_damage: f64,
    pub low_confidence: bool,
    pub wrapper: String,
    pub no_evidence: bool,
    /// sfmeta's experimental BM25-only baseline; no structural admission or rerank.
    pub experimental_lexical_only: bool,
    #[serde(flatten)]
    pub engine: Diagnostics,
}

/// Counts for `aroma inspect-index`.
#[derive(Clone, Debug, Serialize)]
pub struct IndexStats {
    pub generation: u64,
    pub n_files: u32,
    pub n_units: u32,
    pub test_units: u32,
    pub n_features: u64,
    pub n_postings: u64,
    pub units_by_lang: BTreeMap<String, u32>,
    pub units_by_kind: BTreeMap<String, u32>,
    pub oversize_units_by_kind: BTreeMap<String, u32>,
    pub split_units_by_kind: BTreeMap<String, u32>,
    /// `build.json` of the generation (the build report, D11 counters).
    pub build: Option<serde_json::Value>,
}

pub fn kind_name(kind: UnitKind) -> &'static str {
    match kind {
        UnitKind::Method => "method",
        UnitKind::Constructor => "constructor",
        UnitKind::Trigger => "trigger",
        UnitKind::Accessor => "accessor",
        UnitKind::Initializer => "initializer",
        UnitKind::Snippet => "snippet",
        UnitKind::Template => "template",
        UnitKind::MarkupBlock => "markupblock",
        UnitKind::WireMember => "wiremember",
        UnitKind::FlowElement => "flowelement",
        UnitKind::Field => "field",
        UnitKind::ValidationRule => "validationrule",
        UnitKind::MetadataRecord => "metadatarecord",
        UnitKind::Page => "page",
        UnitKind::Component => "component",
    }
}

pub fn profile_name(p: ProfileName) -> &'static str {
    match p {
        ProfileName::Reference => "reference",
        ProfileName::Paper => "paper",
    }
}

struct Rendered {
    text: String,
    spans: Vec<[u32; 2]>,
    stale_source: bool,
    spt: Spt,
}

impl Index {
    /// Open the current generation under `repo_root/.aroma` (no lock).
    pub fn open(repo_root: &Path) -> Result<Index, StoreError> {
        let dir = IndexDir::new(repo_root);
        let Some((_, gen_dir)) = dir.current()? else {
            return Err(StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "no index under {}; run `aroma index` in the repository root",
                    dir.aroma_dir().display()
                ),
            )));
        };
        let manifest = Manifest::read(&gen_dir)?;
        let mut langs = Vec::new();
        for key in manifest.adapters.keys() {
            match Lang::parse(key) {
                Some(l) => langs.push(l),
                None => {
                    return Err(StoreError::Incompatible(format!(
                        "unknown language {key:?} in manifest"
                    )))
                }
            }
        }
        manifest.check_compatible(&expected_manifest_with_mode(
            &langs,
            manifest.profile,
            manifest.compat,
            manifest.sfmeta_mode,
        ))?;
        let postings = MmapPostings::open(&gen_dir.join(POSTINGS_FILE))?;
        let lexical = MmapLexical::open(&gen_dir.join(LEXICAL_FILE))?;
        let units = UnitTable::open(&gen_dir)?;
        let spts = SptStore::open(&gen_dir.join(SPT_FILE))?;
        let n = manifest.n_units;
        if postings.n_units() != n
            || lexical.n_units() != n
            || units.n_units() != n
            || spts.len() != n
        {
            return Err(StoreError::Corrupt {
                file: "manifest.json",
                reason: format!(
                    "manifest has {n} units, postings.bin {}, lexical.bin {}, units.bin {}, spt.bin {}",
                    postings.n_units(),
                    lexical.n_units(),
                    units.n_units(),
                    spts.len()
                ),
            });
        }
        let mut globals = BTreeMap::new();
        for (key, g) in read_globals(&gen_dir)? {
            if let Some(l) = Lang::parse(&key) {
                globals.insert(l, g);
            }
        }
        Ok(Index {
            root: repo_root.to_path_buf(),
            gen_dir,
            profile: effective_profile(manifest.profile, manifest.compat),
            manifest,
            postings,
            lexical,
            units,
            spts,
            globals,
            file_hashes: OnceLock::new(),
        })
    }

    /// Open only if the index was built in the requested metadata mode.
    pub fn open_with_sfmeta_mode(repo_root: &Path, mode: SfmetaMode) -> Result<Index, StoreError> {
        let index = Self::open(repo_root)?;
        let mut expected = index.manifest.clone();
        expected.sfmeta_mode = mode;
        index.manifest.check_compatible(&expected)?;
        Ok(index)
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Full integrity scan of the mapped postings and lexical files (O(n_entries)).
    /// `open` checks only headers and sizes so one-shot queries stay fast; this is what
    /// `aroma inspect-index` runs.
    pub fn verify(&self) -> Result<(), StoreError> {
        self.postings.verify()?;
        self.lexical.verify()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn stats(&self) -> IndexStats {
        let mut units_by_lang = BTreeMap::new();
        let mut units_by_kind = BTreeMap::new();
        let mut oversize_units_by_kind = BTreeMap::new();
        let mut split_units_by_kind = BTreeMap::new();
        let mut test_units = 0;
        for id in 0..self.units.n_units() {
            let r = self.units.record(id);
            *units_by_lang
                .entry(r.lang.as_str().to_string())
                .or_insert(0) += 1;
            *units_by_kind
                .entry(kind_name(r.kind).to_string())
                .or_insert(0) += 1;
            if r.n_leaves > aroma_core::unit::MAX_PRUNE_LEAVES {
                *oversize_units_by_kind
                    .entry(kind_name(r.kind).to_string())
                    .or_insert(0) += 1;
            }
            if r.kind == UnitKind::MarkupBlock {
                *split_units_by_kind
                    .entry(kind_name(r.kind).to_string())
                    .or_insert(0) += 1;
            }
            if r.is_test {
                test_units += 1;
            }
        }
        let build = std::fs::read_to_string(self.gen_dir.join(BUILD_REPORT_FILE))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok());
        IndexStats {
            generation: self.manifest.generation,
            n_files: self.manifest.n_files,
            n_units: self.units.n_units(),
            test_units,
            n_features: self.postings.n_features(),
            n_postings: self.postings.n_entries(),
            units_by_lang,
            units_by_kind,
            oversize_units_by_kind,
            split_units_by_kind,
            build,
        }
    }

    /// Search each part normally (top 100), then expand each displayed hit to all
    /// indexed exact-body clones of that part's language passing its kind/test
    /// filters. Every clone's directory receives the admitted hit's displayed-rank
    /// RRF, showing its own unit. A directory keeps only its best-ranked region per
    /// part (ties by unit id); missing parts contribute zero. Sum the parts' RRF
    /// values and break equal scores by directory. Normal query clone dedup stays
    /// unchanged. Unsupported languages use the normal CLI error.
    pub fn query_component(&self, query: &ComponentQuery) -> Result<ComponentOutput, String> {
        if query.parts.is_empty() {
            return Err("a component query requires at least one part".into());
        }
        for part in &query.parts {
            if spec_for(part.lang).is_none() {
                return Err(format!("{} is not supported yet", part.lang.as_str()));
            }
        }
        // One table scan per component query, with clone lists in unit-id order.
        let mut clones: HashMap<(Lang, u64), Vec<UnitId>> = HashMap::new();
        for unit in 0..self.units.n_units() {
            let rec = self.units.record(unit);
            if query.options.kind.is_none_or(|kind| kind == rec.kind)
                && (query.options.include_tests || !rec.is_test)
            {
                clones
                    .entry((rec.lang, rec.body_hash))
                    .or_default()
                    .push(unit);
            }
        }
        let mut by_dir: BTreeMap<String, ComponentHit> = BTreeMap::new();
        let mut parts = Vec::with_capacity(query.parts.len());
        let mut sources = HashMap::new();
        let mut stale = BTreeSet::new();
        for (part_idx, part) in query.parts.iter().enumerate() {
            let mut opts = query.options.clone();
            opts.lang = part.lang;
            opts.top_similar = 100;
            let mut admitted = Vec::new();
            let out = self.query_with_hits(&part.source, &opts, &mut |unit, kept| {
                admitted.push((unit, kept.to_vec()));
            });
            parts.push(PartDiagnostics {
                lang: part.lang,
                path: part.path.clone(),
                diagnostics: out.diagnostics,
            });
            for (rank, (hit, (unit, kept))) in out.similar.into_iter().zip(admitted).enumerate() {
                let rec = self.units.record(unit);
                for &clone in &clones[&(part.lang, rec.body_hash)] {
                    let clone_rec = self.units.record(clone);
                    let (path, lines) = self.location(&clone_rec);
                    let normalized = path.replace('\\', "/");
                    let dir = normalized
                        .rsplit_once('/')
                        .map_or(".", |(parent, _)| parent);
                    if by_dir
                        .get(dir)
                        .is_some_and(|component| component.parts[part_idx].is_some())
                    {
                        continue;
                    }
                    let own_hit = if clone == unit {
                        hit.clone()
                    } else {
                        let Some(rendered) = self.render(clone, &kept, &mut sources, &mut stale)
                        else {
                            continue;
                        };
                        SimilarOut {
                            path,
                            lines,
                            kind: clone_rec.kind,
                            text: rendered.text,
                            kept_spans: rendered.spans,
                            stale_source: rendered.stale_source,
                            ..hit.clone()
                        }
                    };
                    let component = by_dir
                        .entry(dir.to_string())
                        .or_insert_with(|| ComponentHit {
                            dir: dir.to_string(),
                            score: 0.0,
                            parts: vec![None; query.parts.len()],
                        });
                    component.score += rrf(rank + 1);
                    component.parts[part_idx] = Some(own_hit);
                }
            }
        }
        let mut components: Vec<_> = by_dir.into_values().collect();
        components.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.dir.cmp(&b.dir)));
        components.truncate(query.top);
        Ok(ComponentOutput {
            schema_version: OUTPUT_SCHEMA_VERSION,
            generation: self.manifest.generation,
            components,
            parts,
        })
    }

    pub fn query(&self, text: &str, opts: &QueryOptions) -> QueryOutput {
        self.query_with_hits(text, opts, &mut |_, _| {})
    }

    /// Preserve the identity and kept-leaf mask of each successfully rendered hit
    /// for component expansion; normal query output and admission are unchanged.
    fn query_with_hits(
        &self,
        text: &str,
        opts: &QueryOptions,
        on_hit: &mut dyn FnMut(UnitId, &[bool]),
    ) -> QueryOutput {
        let Some(spec) = spec_for(opts.lang) else {
            return QueryOutput {
                schema_version: OUTPUT_SCHEMA_VERSION,
                generation: self.manifest.generation,
                profile: self.manifest.profile,
                similar: Vec::new(),
                recommendations: Vec::new(),
                diagnostics: OutDiagnostics {
                    parse_damage: 0.0,
                    low_confidence: false,
                    wrapper: "none".to_string(),
                    no_evidence: true,
                    experimental_lexical_only: false,
                    engine: Diagnostics::default(),
                },
            };
        };
        let spec = crate::spec::sfmeta::for_mode(spec, self.manifest.sfmeta_mode);
        let mut profile = self.profile;
        profile.features = (spec.feature_config)(profile.features);
        let globals: Cow<'_, Globals> = match self.globals.get(&opts.lang) {
            Some(g) => Cow::Borrowed(g),
            None => Cow::Owned(Globals::new(spec, Vec::new())),
        };
        let limits = Limits::DEFAULT;
        let snippet = parse_snippet(
            spec,
            text,
            &globals,
            var_mode(self.manifest.compat),
            &limits,
        );
        let mut out = QueryOutput {
            schema_version: OUTPUT_SCHEMA_VERSION,
            generation: self.manifest.generation,
            profile: self.manifest.profile,
            similar: Vec::new(),
            recommendations: Vec::new(),
            diagnostics: OutDiagnostics {
                parse_damage: snippet.damage,
                low_confidence: snippet.damage > limits.max_damage,
                wrapper: snippet.wrapper.to_string(),
                no_evidence: true,
                experimental_lexical_only: opts.lang == Lang::SfMeta
                    && self.manifest.sfmeta_mode == SfmetaMode::Lexical,
                engine: Diagnostics::default(),
            },
        };
        if out.diagnostics.experimental_lexical_only {
            return self.query_metadata_lexical(text, opts, &snippet, out, on_hit);
        }
        let Some(query_spt) = snippet.spt else {
            return out;
        };
        let req = QueryRequest {
            lang: opts.lang,
            spt: &query_spt,
            profile: &profile,
            term_rule: spec.term_rule,
            top_similar: opts.top_similar,
            experimental_recommend: opts.experimental_recommend,
            gate: RecGate::DEFAULT,
            kind: opts.kind,
            include_tests: opts.include_tests,
            budget: opts.budget,
            lexical: (opts.lexical && !self.manifest.compat)
                .then_some(&self.lexical as &dyn LexicalSource),
        };
        let result = run_query(&self.postings, self, &req);

        let mut sources: HashMap<u32, Option<String>> = HashMap::new();
        let mut stale: BTreeSet<UnitId> = BTreeSet::new();
        for s in &result.similar {
            let Some(r) = self.render(s.unit, &s.kept, &mut sources, &mut stale) else {
                continue;
            };
            let rec = self.units.record(s.unit);
            let (path, lines) = self.location(&rec);
            let explanation = opts
                .explain
                .then(|| explain(opts.lang, &profile.features, &query_spt, &r.spt, &s.kept));
            on_hit(s.unit, &s.kept);
            out.similar.push(SimilarOut {
                path,
                lines,
                kind: rec.kind,
                score: s.score,
                text: r.text,
                kept_spans: r.spans,
                stale_source: r.stale_source,
                explain: explanation,
                structural_rank: s.structural_rank,
                lexical_rank: s.lexical_rank,
                bm25: s.bm25,
                matched_terms: s.matched_terms.clone(),
            });
        }
        for rec in &result.recommendations {
            let Some(&first) = rec.members.first() else {
                continue;
            };
            let Some(r) = self.render(first, &rec.kept, &mut sources, &mut stale) else {
                continue;
            };
            let members = rec
                .members
                .iter()
                .map(|&u| {
                    let (path, lines) = self.location(&self.units.record(u));
                    format!("{path}:{}-{}", lines[0], lines[1])
                })
                .collect();
            out.recommendations.push(RecommendationOut {
                members,
                independent_support: rec.independent_support,
                text: r.text,
                kept_spans: r.spans,
                stale_source: r.stale_source,
            });
        }
        let mut engine = result.diagnostics;
        engine.stale_units += stale.len() as u32;
        out.diagnostics.no_evidence =
            engine.known_query_features == 0 && engine.known_query_terms == 0;
        out.diagnostics.engine = engine;
        out
    }

    /// BM25-only metadata deliberately bypasses structural size/clone admission,
    /// pruning, RRF and recommendations. Kind/language/test filters still apply.
    /// The index mode is authoritative, including under --compat/--no-lexical.
    fn query_metadata_lexical(
        &self,
        text: &str,
        opts: &QueryOptions,
        snippet: &crate::snippet::SnippetParse,
        mut out: QueryOutput,
        on_hit: &mut dyn FnMut(UnitId, &[bool]),
    ) -> QueryOutput {
        let start = Instant::now();
        let expired = || opts.budget.is_some_and(|budget| start.elapsed() >= budget);
        let rule = aroma_core::lexical::TermRule::Markup;
        let terms = if snippet.damage <= Limits::DEFAULT.max_damage {
            snippet.spt.as_ref().map(|spt| spt_terms(spt, rule))
        } else {
            None
        }
        .unwrap_or_else(|| rule.terms(text).into_iter().map(|term| (term, 1)).collect());
        out.diagnostics.engine.lexical = true;
        out.diagnostics.engine.query_terms = terms.len() as u32;
        if expired() {
            out.diagnostics.engine.budget_exhausted = Some("lexical");
            return out;
        }
        let eligible = |unit| {
            if unit >= self.units.n_units() {
                return false;
            }
            let record = self.units.record(unit);
            record.lang == Lang::SfMeta
                && opts.kind.is_none_or(|kind| kind == record.kind)
                && (opts.include_tests || !record.is_test)
        };
        let result = lexical_search(
            &self.lexical,
            &query_term_ids(&terms),
            opts.top_similar,
            &eligible,
        );
        out.diagnostics.engine.known_query_terms = result.known_query_terms;
        out.diagnostics.no_evidence = result.known_query_terms == 0;
        let mut sources = HashMap::new();
        let mut stale = BTreeSet::new();
        for (rank, hit) in result.hits.iter().enumerate() {
            if expired() {
                out.diagnostics.engine.budget_exhausted = Some("lexical");
                break;
            }
            let rec = self.units.record(hit.unit);
            let kept = vec![true; rec.n_leaves as usize];
            let Some(rendered) = self.render(hit.unit, &kept, &mut sources, &mut stale) else {
                stale.insert(hit.unit);
                continue;
            };
            let (path, lines) = self.location(&rec);
            let matched_terms = terms
                .keys()
                .filter(|term| term_frequency(&self.lexical, term_id(term), hit.unit).is_some())
                .cloned()
                .collect();
            on_hit(hit.unit, &kept);
            out.similar.push(SimilarOut {
                path,
                lines,
                kind: rec.kind,
                score: hit.score,
                text: rendered.text,
                kept_spans: rendered.spans,
                stale_source: rendered.stale_source,
                explain: None,
                structural_rank: 0,
                lexical_rank: Some(rank as u32 + 1),
                bm25: Some(hit.score),
                matched_terms,
            });
        }
        out.diagnostics.engine.stale_units = stale.len() as u32;
        out
    }

    fn location(&self, rec: &UnitRecord) -> (String, [u32; 2]) {
        let path = self
            .units
            .file_path(rec.file)
            .to_string_lossy()
            .into_owned();
        let last = rec.lines.end.saturating_sub(1).max(rec.lines.start);
        (path, [rec.lines.start, last])
    }

    /// The file's text when its blake3 still equals the indexed one.
    fn fresh_source(&self, file: u32) -> Option<String> {
        let hashes = self.file_hashes.get_or_init(|| {
            match read_file_hashes(&self.gen_dir.join(INVENTORY_FILE)) {
                Ok(h) => Some(h),
                Err(e) => {
                    tracing::warn!(error = %e, "cannot read file hashes; rendering stored tokens");
                    None
                }
            }
        });
        let expected = hashes.as_ref()?.get(file as usize)?;
        let bytes = std::fs::read(self.root.join(self.units.file_path(file))).ok()?;
        (blake3::hash(&bytes).as_bytes() == expected)
            .then(|| String::from_utf8_lossy(&bytes).into_owned())
    }

    fn render(
        &self,
        unit: UnitId,
        kept: &[bool],
        sources: &mut HashMap<u32, Option<String>>,
        stale: &mut BTreeSet<UnitId>,
    ) -> Option<Rendered> {
        let rec = self.units.record(unit);
        let spt = match self.spts.get(unit) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(unit, error = %e, "cannot load a stored tree for rendering");
                return None;
            }
        };
        let source = sources
            .entry(rec.file)
            .or_insert_with(|| self.fresh_source(rec.file));
        let (text, stale_source) = match source.as_deref() {
            Some(src) => (
                render_full_lines(
                    src,
                    rec.lines.clone(),
                    &spt,
                    kept,
                    spec_for(rec.lang)?.omission_marker,
                ),
                false,
            ),
            None => {
                stale.insert(unit);
                (render_tokens(&spt, kept), true)
            }
        };
        let spans = kept_spans(&spt, kept)
            .into_iter()
            .map(|r| [r.start, r.end])
            .collect();
        Some(Rendered {
            text,
            spans,
            stale_source,
            spt,
        })
    }
}

impl UnitStore for Index {
    fn n_units(&self) -> u32 {
        self.units.n_units()
    }

    fn record(&self, id: UnitId) -> UnitRecord {
        self.units.record(id)
    }

    fn spt(&self, id: UnitId) -> Result<Spt, StoreError> {
        self.spts.get(id)
    }

    fn file_path(&self, file: u32) -> &Path {
        self.units.file_path(file)
    }
}
