//! Index build (T15; D10, D11, D15, D18, D19, D22, D23d).
//!
//! ```text
//! walk (ignore: .gitignore/.ignore, hidden entries skipped, no parent ignore files)
//!   -> supported files by spec.matches, sorted by repo-relative '/' path
//!   -> lock .aroma/lock -> previous generation (only if its manifest is compatible)
//!   -> pass 1 (rayon): blake3 per file; changed files are parsed for declared_types
//!   -> Globals per language; a language whose Globals changed is re-classified whole
//!   -> pass 2 (rayon): parse -> extract_units -> damage <= 10% -> unit_spt -> featurize
//!        -> unit_bag (sorted distinct ids) + body_hash + identifier terms (T23);
//!        unchanged files reuse the inventory features and the previous spt.bin trees
//!        (their terms are re-derived from those trees, never by re-parsing)
//!   -> UnitIds in (path, byte) order -> gen-<n>/ {postings.bin, lexical.bin, units.bin,
//!        files.json, spt.bin, globals.json, inventory.postcard, build.json,
//!        manifest.json}
//!   -> commit (atomic CURRENT) -> collect_garbage (keeps n and n-1)
//! ```
//!
//! Why a Globals change re-parses a whole language: the D18 rule marks an identifier
//! `#VAR` unless it is a known global (builtins plus repository type names), so adding,
//! removing or renaming a type anywhere can change the marking in any file of that
//! language. Body edits and new Apex triggers (which declare no type) keep the Globals
//! and re-parse only the changed files.

use crate::classify::{unit_spt, VarMode};
use crate::globals::Globals;
use crate::spec::{lang_for_path, spec_for};
use crate::table::{declared_types, extract_units, parse, AdapterError, Limits};
use aroma_core::features::{featurize, unit_bag, FEATURE_SCHEMA};
use aroma_core::lang::Lang;
use aroma_core::lexical::{doc_len, unit_terms, TermId, TermRule};
use aroma_core::multiset::FeatureId;
use aroma_core::profile::{Profile, ProfileName};
use aroma_core::spt::Spt;
use aroma_core::store::gc::collect_garbage;
use aroma_core::store::generation::IndexDir;
use aroma_core::store::lexical::write_lexical;
use aroma_core::store::lock::WriterLock;
pub use aroma_core::store::manifest::SfmetaMode;
use aroma_core::store::manifest::{Manifest, INDEX_SCHEMA_VERSION};
use aroma_core::store::postings::write_postings;
use aroma_core::store::spt_store::{write_spt_store, SptStore};
use aroma_core::store::unit_table::write_unit_table;
use aroma_core::store::StoreError;
use aroma_core::unit::{UnitKind, UnitRecord};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

pub const POSTINGS_FILE: &str = "postings.bin";
/// BM25 identifier index (T23); present in every generation since schema 2.
pub const LEXICAL_FILE: &str = "lexical.bin";
pub const SPT_FILE: &str = "spt.bin";
pub const GLOBALS_FILE: &str = "globals.json";
pub const INVENTORY_FILE: &str = "inventory.postcard";
pub const BUILD_REPORT_FILE: &str = "build.json";

const INVENTORY_MAGIC: &[u8; 8] = b"AROMAINV";
const INVENTORY_VERSION: u32 = 1;
const INVENTORY_HEADER: usize = 16;
const INVENTORY: &str = "inventory.postcard";

#[derive(Clone, Debug)]
pub struct BuildOptions {
    pub root: PathBuf,
    /// Languages to index; empty means every supported language.
    pub langs: Vec<Lang>,
    pub profile: ProfileName,
    /// Reference-compatible `#VAR` marking and feature config (`--compat`).
    pub compat: bool,
    /// How long to wait for another writer (`Duration::ZERO` tries once).
    pub lock_timeout: Duration,
    /// Worker threads; `None` uses rayon's default (one per core).
    pub jobs: Option<usize>,
}

/// Optional metadata configuration without breaking existing BuildOptions struct
/// literals (including the recall harness). Both forms work with `build_index`.
#[derive(Clone, Debug)]
pub struct ConfiguredBuildOptions {
    options: BuildOptions,
    sfmeta_mode: SfmetaMode,
}

impl BuildOptions {
    /// Select the metadata mode; plain BuildOptions defaults to Structural.
    pub fn with_sfmeta_mode(self, sfmeta_mode: SfmetaMode) -> ConfiguredBuildOptions {
        ConfiguredBuildOptions {
            options: self,
            sfmeta_mode,
        }
    }
}

pub trait BuildConfiguration {
    fn options(&self) -> &BuildOptions;
    fn sfmeta_mode(&self) -> SfmetaMode;
}

impl BuildConfiguration for BuildOptions {
    fn options(&self) -> &BuildOptions {
        self
    }
    fn sfmeta_mode(&self) -> SfmetaMode {
        SfmetaMode::BUILD_DEFAULT
    }
}

impl BuildConfiguration for ConfiguredBuildOptions {
    fn options(&self) -> &BuildOptions {
        &self.options
    }
    fn sfmeta_mode(&self) -> SfmetaMode {
        self.sfmeta_mode
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct BuildReport {
    pub generation: u64,
    pub files_seen: u32,
    pub files_reparsed: u32,
    pub files_unchanged: u32,
    pub files_skipped_large: u32,
    pub files_failed: u32,
    pub units: u32,
    pub units_skipped_damaged: u32,
    pub gc_removed: Vec<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("no supported source files under {0}")]
    Empty(PathBuf),
    #[error("{0} is not supported yet")]
    Unsupported(String),
}

/// One indexed unit in the build-only inventory.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnitEntry {
    pub kind: UnitKind,
    pub bytes: Range<u32>,
    /// 1-based, end exclusive.
    pub lines: Range<u32>,
    pub n_leaves: u32,
    pub body_hash: u64,
    pub is_test: bool,
    /// Sorted distinct feature ids: the unit's postings.
    pub features: Vec<FeatureId>,
}

/// One indexed file in the build-only inventory.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Repo-relative, `/`-separated.
    pub path: String,
    pub lang: Lang,
    pub blake3: [u8; 32],
    /// Informational only: change detection always uses `blake3`.
    pub size: u64,
    pub mtime_ns: Option<u64>,
    pub declared_types: Vec<String>,
    pub units_skipped_damaged: u32,
    /// `UnitId` of this file's first unit in the generation that wrote the inventory.
    pub first_unit: u32,
    pub units: Vec<UnitEntry>,
}

/// `Profile::by_name(name)`, with the reference feature config when `compat`.
pub fn effective_profile(name: ProfileName, compat: bool) -> Profile {
    let profile = Profile::by_name(name);
    if compat {
        profile.compat()
    } else {
        profile
    }
}

/// `VarMode::Compat` under `--compat`, the D18 context-free rule otherwise.
pub fn var_mode(compat: bool) -> VarMode {
    if compat {
        VarMode::Compat
    } else {
        VarMode::ContextFree
    }
}

/// The manifest this binary writes for `langs` (generation and counts zero).
/// `Index::open` and incremental builds compare stored manifests against it.
pub fn expected_manifest(langs: &[Lang], profile: ProfileName, compat: bool) -> Manifest {
    expected_manifest_with_mode(langs, profile, compat, SfmetaMode::BUILD_DEFAULT)
}

pub fn expected_manifest_with_mode(
    langs: &[Lang],
    profile: ProfileName,
    compat: bool,
    sfmeta_mode: SfmetaMode,
) -> Manifest {
    let features = effective_profile(profile, compat).features;
    let mut adapters = BTreeMap::new();
    let mut grammars = BTreeMap::new();
    for &lang in langs {
        let Some(spec) = spec_for(lang) else { continue };
        let spec = crate::spec::sfmeta::for_mode(spec, sfmeta_mode);
        adapters.insert(lang.as_str().to_string(), spec.adapter_version.to_string());
        grammars.insert(lang.as_str().to_string(), spec.grammar_version.to_string());
    }
    Manifest {
        schema_version: INDEX_SCHEMA_VERSION,
        feature_schema: FEATURE_SCHEMA,
        feature_fingerprint: features.fingerprint(),
        profile,
        compat,
        sfmeta_mode,
        adapters,
        grammars,
        generation: 0,
        n_units: 0,
        n_files: 0,
    }
}

/// D22d clone key: xxh3 of the unit's token stream joined by U+001F.
pub fn body_hash(spt: &Spt) -> u64 {
    xxhash_rust::xxh3::xxh3_64(spt.tokens(None).join("\u{1f}").as_bytes())
}

fn corrupt(file: &'static str, reason: impl Into<String>) -> StoreError {
    StoreError::Corrupt {
        file,
        reason: reason.into(),
    }
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), StoreError> {
    let mut bytes =
        serde_json::to_vec_pretty(value).map_err(|e| StoreError::Codec(e.to_string()))?;
    bytes.push(b'\n');
    write_synced(path, &bytes)
}

/// `inventory.postcard`: `b"AROMAINV" | version u32 LE | n_files u32 LE |
/// n_files x blake3 | postcard(Vec<FileEntry>)`.
pub fn write_inventory(path: &Path, files: &[FileEntry]) -> Result<(), StoreError> {
    let n =
        u32::try_from(files.len()).map_err(|_| corrupt(INVENTORY, "more than u32::MAX files"))?;
    let mut buf = Vec::with_capacity(INVENTORY_HEADER + 32 * files.len());
    buf.extend_from_slice(INVENTORY_MAGIC);
    buf.extend_from_slice(&INVENTORY_VERSION.to_le_bytes());
    buf.extend_from_slice(&n.to_le_bytes());
    for f in files {
        buf.extend_from_slice(&f.blake3);
    }
    let body = postcard::to_stdvec(files).map_err(|e| StoreError::Codec(e.to_string()))?;
    buf.extend_from_slice(&body);
    write_synced(path, &buf)
}

fn inventory_len(head: &[u8]) -> Result<usize, StoreError> {
    if head.len() < INVENTORY_HEADER {
        return Err(corrupt(INVENTORY, "shorter than its header"));
    }
    if head[..8] != INVENTORY_MAGIC[..] {
        return Err(corrupt(INVENTORY, "bad magic"));
    }
    let version = u32::from_le_bytes(head[8..12].try_into().expect("4 bytes"));
    if version != INVENTORY_VERSION {
        return Err(corrupt(INVENTORY, format!("unsupported version {version}")));
    }
    Ok(u32::from_le_bytes(head[12..16].try_into().expect("4 bytes")) as usize)
}

/// Only the header and the per-file blake3 table (query-time freshness check, D23c:
/// feature inventories are never decoded by a query).
pub fn read_file_hashes(path: &Path) -> Result<Vec<[u8; 32]>, StoreError> {
    let mut file = std::fs::File::open(path)?;
    let mut head = [0u8; INVENTORY_HEADER];
    file.read_exact(&mut head)
        .map_err(|_| corrupt(INVENTORY, "shorter than its header"))?;
    let n = inventory_len(&head)?;
    let mut raw = vec![0u8; n * 32];
    file.read_exact(&mut raw)
        .map_err(|_| corrupt(INVENTORY, "truncated file hash table"))?;
    Ok(raw
        .chunks_exact(32)
        .map(|c| {
            let mut h = [0u8; 32];
            h.copy_from_slice(c);
            h
        })
        .collect())
}

/// The whole inventory (build only).
pub fn read_inventory(path: &Path) -> Result<Vec<FileEntry>, StoreError> {
    let bytes = std::fs::read(path)?;
    let n = inventory_len(&bytes)?;
    let table_end = INVENTORY_HEADER + n * 32;
    let table = bytes
        .get(INVENTORY_HEADER..table_end)
        .ok_or_else(|| corrupt(INVENTORY, "truncated file hash table"))?;
    let files: Vec<FileEntry> =
        postcard::from_bytes(&bytes[table_end..]).map_err(|e| StoreError::Codec(e.to_string()))?;
    if files.len() != n
        || files
            .iter()
            .zip(table.chunks_exact(32))
            .any(|(f, h)| f.blake3[..] != *h)
    {
        return Err(corrupt(
            INVENTORY,
            "hash table does not match the file entries",
        ));
    }
    Ok(files)
}

/// `globals.json`: language name -> `Globals`.
pub fn read_globals(gen_dir: &Path) -> Result<BTreeMap<String, Globals>, StoreError> {
    let text = std::fs::read_to_string(gen_dir.join(GLOBALS_FILE))?;
    serde_json::from_str(&text).map_err(|e| corrupt("globals.json", e.to_string()))
}

fn normalize_langs(langs: &[Lang]) -> Vec<Lang> {
    let mut out = if langs.is_empty() {
        Lang::SPECCED.to_vec()
    } else {
        langs.to_vec()
    };
    out.sort();
    out.dedup();
    out
}

struct SourceFile {
    rel: String,
    abs: PathBuf,
    lang: Lang,
    size: u64,
    mtime_ns: Option<u64>,
}

fn rel_string(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn walk(root: &Path, langs: &[Lang]) -> Vec<SourceFile> {
    let walker = ignore::WalkBuilder::new(root)
        .hidden(true)
        .parents(false)
        .require_git(false)
        .follow_links(false)
        .build();
    let mut out = Vec::new();
    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "skipping an unreadable path");
                continue;
            }
        };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        let rel = rel_string(rel);
        let Some(lang) = lang_for_path(&rel, langs) else {
            continue;
        };
        let Ok(meta) = std::fs::metadata(path) else {
            continue;
        };
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as u64);
        out.push(SourceFile {
            rel,
            abs: path.to_path_buf(),
            lang,
            size: meta.len(),
            mtime_ns,
        });
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

struct Previous {
    files: HashMap<String, FileEntry>,
    globals: BTreeMap<String, Globals>,
    spts: SptStore,
}

fn try_load_previous(gen_dir: &Path, expected: &Manifest) -> Result<Previous, StoreError> {
    Manifest::read(gen_dir)?.check_compatible(expected)?;
    let files = read_inventory(&gen_dir.join(INVENTORY_FILE))?;
    let globals = read_globals(gen_dir)?;
    let spts = SptStore::open(&gen_dir.join(SPT_FILE))?;
    let mut next = 0u32;
    for f in &files {
        if f.first_unit != next {
            return Err(corrupt(
                INVENTORY,
                format!("{}: first_unit {} != {next}", f.path, f.first_unit),
            ));
        }
        next += f.units.len() as u32;
    }
    if next != spts.len() {
        return Err(corrupt(INVENTORY, "unit count differs from spt.bin"));
    }
    Ok(Previous {
        files: files.into_iter().map(|f| (f.path.clone(), f)).collect(),
        globals,
        spts,
    })
}

fn load_previous(dir: &IndexDir, expected: &Manifest) -> Option<Previous> {
    let gen_dir = match dir.current() {
        Ok(Some((_, d))) => d,
        Ok(None) => return None,
        Err(e) => {
            tracing::warn!(error = %e, "ignoring the previous generation");
            return None;
        }
    };
    match try_load_previous(&gen_dir, expected) {
        Ok(p) => Some(p),
        Err(e) => {
            tracing::info!(reason = %e, "previous generation not reusable; full rebuild");
            None
        }
    }
}

enum Scan {
    TooLarge,
    Failed,
    Unchanged([u8; 32]),
    Changed {
        hash: [u8; 32],
        declared: Vec<String>,
    },
}

/// Pass 1: hash; parse changed files for their declared types.
fn scan(f: &SourceFile, prev: Option<&Previous>, limits: &Limits) -> Scan {
    if f.size > limits.max_file_bytes as u64 {
        return Scan::TooLarge;
    }
    let bytes = match std::fs::read(&f.abs) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(path = %f.rel, error = %e, "cannot read a source file");
            return Scan::Failed;
        }
    };
    if bytes.len() > limits.max_file_bytes {
        return Scan::TooLarge;
    }
    let hash = *blake3::hash(&bytes).as_bytes();
    if let Some(old) = prev.and_then(|p| p.files.get(&f.rel)) {
        if old.blake3 == hash && old.lang == f.lang {
            return Scan::Unchanged(hash);
        }
    }
    let src = String::from_utf8_lossy(&bytes);
    let Some(spec) = spec_for(f.lang) else {
        return Scan::Failed;
    };
    match parse(spec, &src, limits) {
        Ok(tree) => Scan::Changed {
            hash,
            declared: declared_types(&tree, &src, spec),
        },
        Err(AdapterError::TooLarge(_)) => Scan::TooLarge,
        Err(e) => {
            tracing::warn!(path = %f.rel, error = %e, "cannot parse a source file");
            Scan::Failed
        }
    }
}

struct Ctx<'a> {
    limits: Limits,
    profile: Profile,
    mode: VarMode,
    sfmeta_mode: SfmetaMode,
    globals: &'a BTreeMap<Lang, Globals>,
}

struct Indexed {
    entry: FileEntry,
    spts: Vec<Spt>,
    /// Identifier terms per unit (`lexical::unit_terms` of each tree).
    terms: Vec<Vec<(TermId, u32)>>,
    reparsed: bool,
}

fn terms_of(spts: &[Spt], rule: TermRule) -> Vec<Vec<(TermId, u32)>> {
    spts.iter().map(|spt| unit_terms(spt, rule)).collect()
}

/// Pass 2: parse, extract, classify and featurize one file.
fn index_file(f: &SourceFile, hash: [u8; 32], ctx: &Ctx<'_>) -> Option<Indexed> {
    let bytes = match std::fs::read(&f.abs) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(path = %f.rel, error = %e, "cannot read a source file");
            return None;
        }
    };
    let src = String::from_utf8_lossy(&bytes);
    let spec = crate::spec::sfmeta::for_mode(spec_for(f.lang)?, ctx.sfmeta_mode);
    let tree = match parse(spec, &src, &ctx.limits) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(path = %f.rel, error = %e, "cannot parse a source file");
            return None;
        }
    };
    let globals = ctx.globals.get(&f.lang)?;
    let is_test = (spec.is_test_path)(Path::new(&f.rel));
    let mut extracted = extract_units(&tree, &src, spec, &ctx.limits);
    extracted.sort_by_key(|u| (u.bytes.start, u.bytes.end));
    let mut units = Vec::with_capacity(extracted.len());
    let mut spts = Vec::with_capacity(extracted.len());
    let mut skipped = 0u32;
    for u in &extracted {
        if u.damage > ctx.limits.max_damage {
            skipped += 1;
            continue;
        }
        let Some(spt) = unit_spt(u, spec, globals, ctx.mode) else {
            continue;
        };
        let features: Vec<FeatureId> =
            if f.lang == Lang::SfMeta && ctx.sfmeta_mode == SfmetaMode::Lexical {
                Vec::new()
            } else {
                let config = (spec.feature_config)(ctx.profile.features);
                let per_leaf = featurize(&spt, &config, None);
                unit_bag(f.lang, &per_leaf).distinct().collect()
            };
        units.push(UnitEntry {
            kind: u.kind,
            bytes: u.bytes.clone(),
            lines: u.lines.clone(),
            n_leaves: spt.leaves.len() as u32,
            body_hash: body_hash(&spt),
            is_test,
            features,
        });
        spts.push(spt);
    }
    Some(Indexed {
        entry: FileEntry {
            path: f.rel.clone(),
            lang: f.lang,
            blake3: hash,
            size: f.size,
            mtime_ns: f.mtime_ns,
            declared_types: declared_types(&tree, &src, spec),
            units_skipped_damaged: skipped,
            first_unit: 0,
            units,
        },
        terms: terms_of(&spts, spec.term_rule),
        spts,
        reparsed: true,
    })
}

/// Unchanged file: inventory entry + trees from the previous `spt.bin`.
fn reuse(f: &SourceFile, prev: &Previous) -> Option<Indexed> {
    let spec = spec_for(f.lang)?;
    let old = prev.files.get(&f.rel)?;
    let spts: Result<Vec<Spt>, StoreError> = (0..old.units.len() as u32)
        .map(|i| prev.spts.get(old.first_unit + i))
        .collect();
    match spts {
        Ok(spts) => {
            let mut entry = old.clone();
            entry.size = f.size;
            entry.mtime_ns = f.mtime_ns;
            Some(Indexed {
                entry,
                terms: terms_of(&spts, spec.term_rule),
                spts,
                reparsed: false,
            })
        }
        Err(e) => {
            tracing::warn!(path = %f.rel, error = %e, "stored trees unreadable; re-parsing");
            None
        }
    }
}

fn in_pool<R: Send>(pool: Option<&rayon::ThreadPool>, f: impl FnOnce() -> R + Send) -> R {
    match pool {
        Some(p) => p.install(f),
        None => f(),
    }
}

/// Build (or incrementally update) the index under `opts.root/.aroma`.
pub fn build_index(configuration: &impl BuildConfiguration) -> Result<BuildReport, BuildError> {
    let opts = configuration.options();
    let sfmeta_mode = configuration.sfmeta_mode();
    for &lang in &opts.langs {
        if spec_for(lang).is_none() {
            return Err(BuildError::Unsupported(lang.as_str().to_string()));
        }
    }
    let langs = normalize_langs(&opts.langs);
    let files = walk(&opts.root, &langs);
    if files.is_empty() {
        return Err(BuildError::Empty(opts.root.clone()));
    }
    let dir = IndexDir::new(&opts.root);
    let lock = WriterLock::acquire(dir.aroma_dir(), opts.lock_timeout)?;
    let pool = match opts.jobs {
        Some(n) => Some(
            rayon::ThreadPoolBuilder::new()
                .num_threads(n.max(1))
                .build()
                .map_err(|e| StoreError::Io(std::io::Error::other(e.to_string())))?,
        ),
        None => None,
    };
    let expected = expected_manifest_with_mode(&langs, opts.profile, opts.compat, sfmeta_mode);
    let prev = load_previous(&dir, &expected);
    let limits = Limits::DEFAULT;
    let mut report = BuildReport {
        files_seen: files.len() as u32,
        ..BuildReport::default()
    };

    // Pass 1: hashes and the declared types of changed files.
    let scans: Vec<Scan> = in_pool(pool.as_ref(), || {
        files
            .par_iter()
            .map(|f| scan(f, prev.as_ref(), &limits))
            .collect()
    });

    // D18 globals per language; a changed set re-classifies the whole language.
    let mut types: BTreeMap<Lang, Vec<String>> = langs.iter().map(|&l| (l, Vec::new())).collect();
    for (f, s) in files.iter().zip(&scans) {
        let declared = match s {
            Scan::Unchanged(_) => prev
                .as_ref()
                .and_then(|p| p.files.get(&f.rel))
                .map(|e| e.declared_types.clone())
                .unwrap_or_default(),
            Scan::Changed { declared, .. } => declared.clone(),
            Scan::TooLarge | Scan::Failed => continue,
        };
        types.entry(f.lang).or_default().extend(declared);
    }
    let globals: BTreeMap<Lang, Globals> = types
        .into_iter()
        .filter_map(|(l, t)| spec_for(l).map(|spec| (l, Globals::new(spec, t))))
        .collect();
    let changed_langs: Vec<Lang> = globals
        .iter()
        .filter(|(l, g)| prev.as_ref().and_then(|p| p.globals.get(l.as_str())) != Some(*g))
        .map(|(l, _)| *l)
        .collect();
    if prev.is_some() {
        for l in &changed_langs {
            tracing::info!(
                lang = l.as_str(),
                "known types changed; re-classifying every file of this language"
            );
        }
    }

    // Pass 2: parse/classify/featurize changed files, reuse the rest.
    let ctx = Ctx {
        limits,
        profile: effective_profile(opts.profile, opts.compat),
        mode: var_mode(opts.compat),
        sfmeta_mode,
        globals: &globals,
    };
    let outcomes: Vec<Option<Indexed>> = in_pool(pool.as_ref(), || {
        files
            .par_iter()
            .zip(scans.par_iter())
            .map(|(f, s)| match s {
                Scan::TooLarge | Scan::Failed => None,
                Scan::Unchanged(hash) => {
                    let reused = if changed_langs.contains(&f.lang) {
                        None
                    } else {
                        prev.as_ref().and_then(|p| reuse(f, p))
                    };
                    reused.or_else(|| index_file(f, *hash, &ctx))
                }
                Scan::Changed { hash, .. } => index_file(f, *hash, &ctx),
            })
            .collect()
    });
    for (s, o) in scans.iter().zip(&outcomes) {
        match (s, o) {
            (Scan::TooLarge, _) => report.files_skipped_large += 1,
            (_, None) => report.files_failed += 1,
            (_, Some(i)) if i.reparsed => report.files_reparsed += 1,
            (_, Some(_)) => report.files_unchanged += 1,
        }
    }

    // UnitIds in (path, byte) order.
    let mut entries: Vec<FileEntry> = Vec::new();
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut records: Vec<UnitRecord> = Vec::new();
    let mut spts: Vec<Spt> = Vec::new();
    let mut postings: Vec<(FeatureId, u32)> = Vec::new();
    let mut lexical: Vec<(TermId, u32, u32)> = Vec::new();
    let mut doc_lens: Vec<u32> = Vec::new();
    for Indexed {
        mut entry,
        spts: file_spts,
        terms: file_terms,
        ..
    } in outcomes.into_iter().flatten()
    {
        let file = entries.len() as u32;
        entry.first_unit = records.len() as u32;
        for ((u, spt), terms) in entry.units.iter().zip(file_spts).zip(file_terms) {
            let id = records.len() as u32;
            records.push(UnitRecord {
                id,
                lang: entry.lang,
                kind: u.kind,
                file,
                bytes: u.bytes.clone(),
                lines: u.lines.clone(),
                n_leaves: u.n_leaves,
                body_hash: u.body_hash,
                is_test: u.is_test,
            });
            postings.extend(u.features.iter().map(|&f| (f, id)));
            lexical.extend(terms.iter().map(|&(t, n)| (t, id, n)));
            doc_lens.push(doc_len(&terms));
            spts.push(spt);
        }
        report.units_skipped_damaged += entry.units_skipped_damaged;
        paths.push(PathBuf::from(&entry.path));
        entries.push(entry);
    }
    let n_units = u32::try_from(records.len()).map_err(|_| {
        StoreError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "more than u32::MAX units",
        ))
    })?;
    report.units = n_units;

    // Publish one immutable generation.
    let writer = dir.begin(&lock)?;
    let gen_dir = writer.dir().to_path_buf();
    report.generation = writer.number();
    write_postings(&gen_dir.join(POSTINGS_FILE), postings, n_units)?;
    write_lexical(&gen_dir.join(LEXICAL_FILE), lexical, &doc_lens)?;
    write_unit_table(&gen_dir, &records, &paths)?;
    write_spt_store(&gen_dir.join(SPT_FILE), &spts)?;
    let globals_out: BTreeMap<&str, &Globals> =
        globals.iter().map(|(l, g)| (l.as_str(), g)).collect();
    write_json(&gen_dir.join(GLOBALS_FILE), &globals_out)?;
    write_inventory(&gen_dir.join(INVENTORY_FILE), &entries)?;
    write_json(&gen_dir.join(BUILD_REPORT_FILE), &report)?;
    let manifest = Manifest {
        generation: report.generation,
        n_units,
        n_files: entries.len() as u32,
        ..expected
    };
    manifest.write(&gen_dir)?;
    let committed = writer.commit()?;
    debug_assert_eq!(committed, report.generation);
    report.gc_removed = collect_garbage(&dir, &lock)?;
    drop(lock);
    Ok(report)
}
