//! `aroma` command-line interface (T17): index a supported repository and search it for
//! structurally similar code.
//!
//! ```text
//! aroma index [PATH] [--lang java,apex] [--profile reference|paper] [--compat]
//!             [--jobs N] [--lock-timeout SECS] [--json]
//! aroma query (--file F | --stdin) --lang L [--root DIR] [--similar N]
//! aroma query (--component DIR | --file F --file F...) [--root DIR] [--similar N]
//!             [--experimental-recommend] [--top N] [--kind K] [--include-tests]
//!             [--budget-ms MS] [--no-lexical] [--explain] [--json]
//! aroma dump-spt FILE [--lang L] [--compat]
//! aroma dump-features FILE [--lang L] [--with-origins] [--compat]
//! aroma feature-diff A B [--lang L] [--compat]
//! aroma inspect-index [--root DIR] [--json]
//! ```
//!
//! Results go to stdout (one JSON document with `--json`), logs to stderr (`RUST_LOG`,
//! default `warn`). Exit codes: 0 ok, 1 failure, 2 usage, 3 index missing or
//! incompatible, 4 writer lock timeout.

use aroma_core::features::featurize;
use aroma_core::lang::Lang;
use aroma_core::profile::ProfileName;
use aroma_core::spt::{Elem, Spt};
use aroma_core::store::StoreError;
use aroma_core::unit::UnitKind;
use aroma_lang::build::{
    build_index, effective_profile, var_mode, BuildError, BuildOptions, BuildReport, SfmetaMode,
};
use aroma_lang::classify::unit_spt;
use aroma_lang::globals::Globals;
use aroma_lang::index::{
    kind_name, profile_name, ComponentOutput, ComponentPart, ComponentQuery, Index, IndexStats,
    QueryOptions, QueryOutput, SimilarOut,
};
use aroma_lang::snippet::parse_snippet;
use aroma_lang::spec::{lang_for_path, resolve_query_lang, spec_for};
use aroma_lang::table::{declared_types, extract_units, parse, Limits};
use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

const EXIT_FAILURE: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_INDEX: u8 = 3;
const EXIT_LOCKED: u8 = 4;

#[derive(Parser, Debug)]
#[command(
    name = "aroma",
    version,
    about = "Structural code search for Java, Apex, LWC JavaScript/HTML, Salesforce XML metadata, Aura markup/JavaScript, and Visualforce (Aroma)",
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Build or incrementally update the index under PATH/.aroma
    Index(IndexArgs),
    /// Find indexed code similar to a snippet
    Query(QueryArgs),
    /// Print the simplified parse tree of every unit in FILE
    DumpSpt(DumpArgs),
    /// Print the features of every unit in FILE
    DumpFeatures(DumpFeaturesArgs),
    /// Compare the features of two files or snippets
    FeatureDiff(FeatureDiffArgs),
    /// Print the manifest and counts of an index
    InspectIndex(InspectArgs),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum LangArg {
    Java,
    Apex,
    Sfmeta,
}

impl From<LangArg> for Lang {
    fn from(l: LangArg) -> Lang {
        match l {
            LangArg::Java => Lang::Java,
            LangArg::Apex => Lang::Apex,
            LangArg::Sfmeta => Lang::SfMeta,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum ProfileArg {
    Reference,
    Paper,
}

impl From<ProfileArg> for ProfileName {
    fn from(p: ProfileArg) -> ProfileName {
        match p {
            ProfileArg::Reference => ProfileName::Reference,
            ProfileArg::Paper => ProfileName::Paper,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum KindArg {
    Method,
    Constructor,
    Trigger,
    Accessor,
    Initializer,
    Snippet,
    Template,
    Markupblock,
    Wiremember,
    Flowelement,
    Field,
    Validationrule,
    Metadatarecord,
    Page,
    Component,
}

impl From<KindArg> for UnitKind {
    fn from(k: KindArg) -> UnitKind {
        match k {
            KindArg::Method => UnitKind::Method,
            KindArg::Constructor => UnitKind::Constructor,
            KindArg::Trigger => UnitKind::Trigger,
            KindArg::Accessor => UnitKind::Accessor,
            KindArg::Initializer => UnitKind::Initializer,
            KindArg::Snippet => UnitKind::Snippet,
            KindArg::Template => UnitKind::Template,
            KindArg::Markupblock => UnitKind::MarkupBlock,
            KindArg::Wiremember => UnitKind::WireMember,
            KindArg::Flowelement => UnitKind::FlowElement,
            KindArg::Field => UnitKind::Field,
            KindArg::Validationrule => UnitKind::ValidationRule,
            KindArg::Metadatarecord => UnitKind::MetadataRecord,
            KindArg::Page => UnitKind::Page,
            KindArg::Component => UnitKind::Component,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum SfmetaModeArg {
    Structural,
    Lexical,
}

impl From<SfmetaModeArg> for SfmetaMode {
    fn from(mode: SfmetaModeArg) -> Self {
        match mode {
            SfmetaModeArg::Structural => Self::Structural,
            SfmetaModeArg::Lexical => Self::Lexical,
        }
    }
}

#[derive(Args, Debug)]
struct IndexArgs {
    /// Repository root
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Languages to index, comma-separated (default: all): java, apex, lwcjs,
    /// lwchtml, sfmeta, aura, aurajs, vf. Aliases: lwc (JS+HTML), aura (markup+JS).
    #[arg(long, value_delimiter = ',')]
    lang: Vec<String>,
    /// Algorithm profile recorded in the index
    #[arg(long, value_enum, default_value_t = ProfileArg::Reference)]
    profile: ProfileArg,
    /// Metadata search mode (default: structural; lexical is experimental BM25-only)
    #[arg(long, value_enum, default_value = "structural")]
    sfmeta_mode: SfmetaModeArg,
    /// Reference-compatible #VAR marking (golden comparisons)
    #[arg(long)]
    compat: bool,
    /// Worker threads (default: one per core)
    #[arg(long)]
    jobs: Option<usize>,
    /// Seconds to wait for another writer; 0 tries once
    #[arg(long, default_value_t = 30)]
    lock_timeout: u64,
    /// Print the build report as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
#[command(group(ArgGroup::new("input").required(true).args(["file", "stdin", "component"])))]
struct QueryArgs {
    /// Read the snippet from this file; repeat to search a component (languages inferred)
    #[arg(long)]
    file: Vec<PathBuf>,
    /// Search routable files directly inside this directory (languages inferred)
    #[arg(long)]
    component: Option<PathBuf>,
    /// Read the snippet from standard input
    #[arg(long)]
    stdin: bool,
    /// Snippet language (required for a single file or stdin): java, apex, lwcjs,
    /// lwchtml, sfmeta, aura, aurajs, vf. Aliases lwc and aura select markup or JS
    /// from the snippet content.
    #[arg(long)]
    lang: Option<String>,
    /// Repository root holding .aroma (default: nearest ancestor with an index)
    #[arg(long)]
    root: Option<PathBuf>,
    /// Number of similar results or components
    #[arg(long, default_value_t = 10)]
    similar: usize,
    /// Also compute clustered recommendations (gated, experimental)
    #[arg(long)]
    experimental_recommend: bool,
    /// Maximum number of recommendations to print
    #[arg(long, default_value_t = 5)]
    top: usize,
    /// Only return units of this kind
    #[arg(long, value_enum)]
    kind: Option<KindArg>,
    /// Also return units from test files
    #[arg(long)]
    include_tests: bool,
    /// Query time budget in milliseconds (partial results when exceeded)
    #[arg(long)]
    budget_ms: Option<u64>,
    /// Rank by structure only: turn off the BM25 identifier channel (always off on a
    /// --compat index)
    #[arg(long)]
    no_lexical: bool,
    /// Attach matched / query-only / candidate-only features and the lexical ranks
    #[arg(long)]
    explain: bool,
    /// Print the result as JSON (schema_version 1)
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct DumpArgs {
    file: PathBuf,
    /// Language (default: from the file extension)
    #[arg(long, value_enum)]
    lang: Option<LangArg>,
    /// Reference-compatible #VAR marking
    #[arg(long)]
    compat: bool,
}

#[derive(Args, Debug)]
struct DumpFeaturesArgs {
    file: PathBuf,
    /// Language (default: from the file extension)
    #[arg(long, value_enum)]
    lang: Option<LangArg>,
    /// List the features of each leaf instead of the counted multiset
    #[arg(long)]
    with_origins: bool,
    /// Reference-compatible #VAR marking and feature config
    #[arg(long)]
    compat: bool,
}

#[derive(Args, Debug)]
struct FeatureDiffArgs {
    a: PathBuf,
    b: PathBuf,
    /// Language (default: from the first file's extension)
    #[arg(long, value_enum)]
    lang: Option<LangArg>,
    /// Reference-compatible #VAR marking and feature config
    #[arg(long)]
    compat: bool,
}

#[derive(Args, Debug)]
struct InspectArgs {
    /// Repository root holding .aroma (default: nearest ancestor with an index)
    #[arg(long)]
    root: Option<PathBuf>,
    /// Print manifest and counts as JSON
    #[arg(long)]
    json: bool,
}

struct Failure {
    code: u8,
    message: String,
}

impl Failure {
    fn new(code: u8, message: impl Into<String>) -> Failure {
        Failure {
            code,
            message: message.into(),
        }
    }
}

type CliResult = Result<(), Failure>;

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging();
    let result = match cli.command {
        Command::Index(a) => cmd_index(a),
        Command::Query(a) => cmd_query(a),
        Command::DumpSpt(a) => cmd_dump_spt(a),
        Command::DumpFeatures(a) => cmd_dump_features(a),
        Command::FeatureDiff(a) => cmd_feature_diff(a),
        Command::InspectIndex(a) => cmd_inspect(a),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(f) => {
            eprintln!("aroma: {}", f.message);
            ExitCode::from(f.code)
        }
    }
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

/// `serde` is not a direct dependency of the CLI, so JSON goes through `serde_json`'s
/// generic functions at each call site and only the error is mapped here.
fn json_err(e: serde_json::Error) -> Failure {
    Failure::new(EXIT_FAILURE, e.to_string())
}

fn build_failure(e: BuildError) -> Failure {
    match e {
        BuildError::Store(StoreError::Locked { pid }) => Failure::new(
            EXIT_LOCKED,
            format!(
                "index is locked by pid {pid}: another `aroma index` is running (wait, or raise --lock-timeout)"
            ),
        ),
        other => Failure::new(EXIT_FAILURE, other.to_string()),
    }
}

fn index_failure(e: StoreError) -> Failure {
    const HINT: &str = "run `aroma index` in the repository root";
    let msg = e.to_string();
    // `Index::open` already carries the hint for a missing index; add it only once.
    let msg = if msg.contains(HINT) {
        format!("cannot open the index: {msg}")
    } else {
        format!("cannot open the index: {msg}; {HINT}")
    };
    Failure::new(EXIT_INDEX, msg)
}

/// Nearest ancestor of the working directory holding `.aroma/CURRENT`, else the
/// working directory.
fn find_root(explicit: Option<PathBuf>) -> Result<PathBuf, Failure> {
    if let Some(root) = explicit {
        return Ok(root);
    }
    let cwd = std::env::current_dir()
        .map_err(|e| Failure::new(EXIT_FAILURE, format!("no working directory: {e}")))?;
    Ok(cwd
        .ancestors()
        .find(|d| d.join(".aroma").join("CURRENT").is_file())
        .unwrap_or(cwd.as_path())
        .to_path_buf())
}

fn read_text(path: &Path) -> Result<String, Failure> {
    let bytes = std::fs::read(path)
        .map_err(|e| Failure::new(EXIT_FAILURE, format!("cannot read {}: {e}", path.display())))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn resolve_lang(explicit: Option<LangArg>, file: &Path) -> Result<Lang, Failure> {
    if let Some(l) = explicit {
        return Ok(l.into());
    }
    lang_for_path(&file.to_string_lossy().replace('\\', "/"), Lang::SPECCED).ok_or_else(|| {
        Failure::new(
            EXIT_USAGE,
            format!(
                "cannot infer the language of {}; pass --lang java|apex|sfmeta",
                file.display()
            ),
        )
    })
}

fn index_langs(names: &[String]) -> Result<Vec<Lang>, Failure> {
    let mut langs = Vec::new();
    for name in names {
        let expanded: &[&str] = match name.as_str() {
            "lwc" => &["lwcjs", "lwchtml"],
            "aura" => &["aura", "aurajs"],
            _ => &[name.as_str()],
        };
        for name in expanded {
            // Markup text resolves the exact `aura` identity rather than its JS alias.
            let lang = resolve_query_lang(name, "<").map_err(|e| Failure::new(EXIT_USAGE, e))?;
            langs.push(lang);
        }
    }
    Ok(langs)
}

fn cmd_index(a: IndexArgs) -> CliResult {
    let opts = BuildOptions {
        root: a.path,
        langs: index_langs(&a.lang)?,
        profile: a.profile.into(),
        compat: a.compat,
        lock_timeout: Duration::from_secs(a.lock_timeout),
        jobs: a.jobs,
    }
    .with_sfmeta_mode(a.sfmeta_mode.into());
    let report = build_index(&opts).map_err(build_failure)?;
    if a.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(json_err)?
        );
    } else {
        print!("{}", report_text(&report));
    }
    Ok(())
}

fn report_text(r: &BuildReport) -> String {
    let mut out = format!(
        "indexed generation {}: {} units from {} files ({} parsed, {} unchanged, {} too large, {} failed), {} units skipped as damaged\n",
        r.generation,
        r.units,
        r.files_reparsed + r.files_unchanged,
        r.files_reparsed,
        r.files_unchanged,
        r.files_skipped_large,
        r.files_failed,
        r.units_skipped_damaged
    );
    if !r.gc_removed.is_empty() {
        let removed: Vec<String> = r.gc_removed.iter().map(u64::to_string).collect();
        let _ = writeln!(out, "removed old generations: {}", removed.join(", "));
    }
    out
}

fn cmd_query(a: QueryArgs) -> CliResult {
    if a.component.is_some() || a.file.len() >= 2 {
        return cmd_component_query(&a);
    }
    let lang_name = a.lang.as_deref().ok_or_else(|| {
        Failure::new(
            EXIT_USAGE,
            "--lang is required for a single file or stdin query",
        )
    })?;
    let text = match a.file.first() {
        Some(path) => read_text(path)?,
        None => {
            let mut buf = Vec::new();
            std::io::stdin()
                .read_to_end(&mut buf)
                .map_err(|e| Failure::new(EXIT_FAILURE, format!("cannot read stdin: {e}")))?;
            String::from_utf8_lossy(&buf).into_owned()
        }
    };
    let lang = resolve_query_lang(lang_name, &text).map_err(|e| Failure::new(EXIT_USAGE, e))?;
    let root = find_root(a.root.clone())?;
    let index = Index::open(&root).map_err(index_failure)?;
    let opts = query_options(&a, lang);
    let mut out = index.query(&text, &opts);
    out.recommendations.truncate(a.top);
    if a.json {
        println!("{}", serde_json::to_string_pretty(&out).map_err(json_err)?);
    } else {
        print!("{}", query_text(&out));
    }
    if out.diagnostics.low_confidence {
        eprintln!(
            "aroma: warning: {:.0}% of the snippet did not parse; results are low-confidence",
            out.diagnostics.parse_damage * 100.0
        );
    }
    Ok(())
}

fn query_options(a: &QueryArgs, lang: Lang) -> QueryOptions {
    QueryOptions {
        lang,
        top_similar: a.similar,
        experimental_recommend: a.experimental_recommend,
        kind: a.kind.map(Into::into),
        include_tests: a.include_tests,
        budget: a.budget_ms.map(Duration::from_millis),
        explain: a.explain,
        lexical: !a.no_lexical,
    }
}

fn component_part(path: &Path, lang: Lang) -> Result<ComponentPart, Failure> {
    Ok(ComponentPart {
        lang,
        source: read_text(path)?,
        path: Some(path.to_string_lossy().replace('\\', "/")),
    })
}

fn directory_parts(dir: &Path, enabled: &[Lang]) -> Result<Vec<ComponentPart>, Failure> {
    let read_error = |e| {
        Failure::new(
            EXIT_FAILURE,
            format!("cannot read directory {}: {e}", dir.display()),
        )
    };
    let mut entries = std::fs::read_dir(dir)
        .map_err(read_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(read_error)?;
    entries.sort_by_key(|e| e.path());
    let mut parts = Vec::new();
    for entry in entries {
        let path = entry.path();
        if !entry.file_type().map_err(read_error)?.is_file()
            || path.components().any(|c| c.as_os_str() == "__tests__")
        {
            continue;
        }
        let rel = path.to_string_lossy().replace('\\', "/");
        if let Some(lang) = lang_for_path(&rel, enabled) {
            parts.push(component_part(&path, lang)?);
        }
    }
    if parts.is_empty() {
        return Err(Failure::new(
            EXIT_USAGE,
            format!(
                "no routable files in {} for the index's enabled languages",
                dir.display()
            ),
        ));
    }
    Ok(parts)
}

fn cmd_component_query(a: &QueryArgs) -> CliResult {
    let root = find_root(a.root.clone())?;
    let index = Index::open(&root).map_err(index_failure)?;
    let parts = if let Some(dir) = &a.component {
        let enabled: Vec<_> = index
            .manifest()
            .adapters
            .keys()
            .filter_map(|l| Lang::parse(l))
            .collect();
        directory_parts(dir, &enabled)?
    } else {
        a.file.iter().map(|path| {
            let lang = lang_for_path(&path.to_string_lossy().replace('\\', "/"), Lang::SPECCED)
                .ok_or_else(|| Failure::new(EXIT_USAGE, format!(
                    "cannot infer the language of {} for component search; use a routable file path",
                    path.display()
                )))?;
            component_part(path, lang)
        }).collect::<Result<Vec<_>, _>>()?
    };
    let options = query_options(a, parts[0].lang);
    let out = index
        .query_component(&ComponentQuery {
            parts,
            top: a.similar,
            options,
        })
        .map_err(|e| Failure::new(EXIT_USAGE, e))?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&out).map_err(json_err)?);
    } else {
        print!("{}", component_text(&out));
    }
    for (i, part) in out.parts.iter().enumerate() {
        if part.diagnostics.low_confidence {
            eprintln!(
                "aroma: warning: {:.0}% of component part {} did not parse; results are low-confidence",
                part.diagnostics.parse_damage * 100.0, i + 1
            );
        }
    }
    Ok(())
}

fn component_text(out: &ComponentOutput) -> String {
    let mut w = String::new();
    if out.components.is_empty() {
        w.push_str("no similar components found\n");
    }
    for (i, component) in out.components.iter().enumerate() {
        let _ = writeln!(
            w,
            "#{} score {:.6} {}",
            i + 1,
            component.score,
            component.dir
        );
        for (part_idx, hit) in component.parts.iter().enumerate() {
            if let Some(hit) = hit {
                similar_text(&mut w, part_idx + 1, hit);
            }
        }
    }
    w
}

fn similar_text(w: &mut String, rank: usize, s: &SimilarOut) {
    let stale = if s.stale_source {
        " [stale source: showing indexed tokens]"
    } else {
        ""
    };
    let _ = writeln!(
        w,
        "#{} {}:{}-{} {} score {:.3}{stale}",
        rank,
        s.path,
        s.lines[0],
        s.lines[1],
        kind_name(s.kind),
        s.score
    );
    w.push_str(&s.text);
    if !s.text.ends_with('\n') {
        w.push('\n');
    }
    if let Some(e) = &s.explain {
        let _ = writeln!(
            w,
            "  explain: jaccard {:.3}, {} matched, {} query-only, {} candidate-only",
            e.jaccard,
            e.matched.len(),
            e.query_only.len(),
            e.candidate_only.len()
        );
        for f in e.query_only.iter().take(20) {
            let _ = writeln!(w, "    query-only: {}", f.escape_debug());
        }
        if let Some(bm25) = s.bm25 {
            let lexical_rank = s
                .lexical_rank
                .map_or_else(|| "-".to_string(), |r| r.to_string());
            let _ = writeln!(
                w,
                "  lexical: structural rank {}, bm25 rank {lexical_rank}, bm25 {bm25:.3}, terms: {}",
                s.structural_rank,
                s.matched_terms.join(" ")
            );
        }
    }
    w.push('\n');
}

fn query_text(out: &QueryOutput) -> String {
    let mut w = String::new();
    if out.similar.is_empty() {
        w.push_str("no similar code found\n");
    }
    for (i, s) in out.similar.iter().enumerate() {
        similar_text(&mut w, i + 1, s);
    }
    for (i, r) in out.recommendations.iter().enumerate() {
        let _ = writeln!(
            w,
            "recommendation #{} (independent support {}): {}",
            i + 1,
            r.independent_support,
            r.members.join(", ")
        );
        w.push_str(&r.text);
        if !r.text.ends_with('\n') {
            w.push('\n');
        }
        w.push('\n');
    }
    let d = &out.diagnostics;
    let _ = write!(
        w,
        "-- generation {}, profile {}, wrapper {}, parse damage {:.2}, {}/{} query features known, {} ineligible skipped, {} stale",
        out.generation,
        profile_name(out.profile),
        d.wrapper,
        d.parse_damage,
        d.engine.known_query_features,
        d.engine.query_features,
        d.engine.ineligible_skipped,
        d.engine.stale_units
    );
    if d.engine.lexical {
        let _ = write!(
            w,
            ", {}/{} query terms known",
            d.engine.known_query_terms, d.engine.query_terms
        );
    }
    if let Some(stage) = d.engine.budget_exhausted {
        let _ = write!(w, ", budget exhausted during {stage}");
    }
    if d.no_evidence {
        w.push_str(", no evidence");
    }
    w.push('\n');
    w
}

struct DumpUnit {
    title: String,
    spt: Spt,
}

/// Units of one file classified with the file's own declared types as repository
/// globals; a file without units is treated as a snippet.
fn file_units(lang: Lang, src: &str, compat: bool) -> Result<Vec<DumpUnit>, Failure> {
    let spec = spec_for(lang).ok_or_else(|| {
        Failure::new(
            EXIT_USAGE,
            format!("{} is not supported yet", lang.as_str()),
        )
    })?;
    let limits = Limits::DEFAULT;
    let mode = var_mode(compat);
    let tree = parse(spec, src, &limits)
        .map_err(|e| Failure::new(EXIT_FAILURE, format!("cannot parse: {e}")))?;
    let globals = Globals::new(spec, declared_types(&tree, src, spec));
    let mut units = Vec::new();
    for u in extract_units(&tree, src, spec, &limits) {
        if let Some(spt) = unit_spt(&u, spec, &globals, mode) {
            units.push(DumpUnit {
                title: format!(
                    "{} lines {}-{} (damage {:.3})",
                    kind_name(u.kind),
                    u.lines.start,
                    u.lines.end.saturating_sub(1),
                    u.damage
                ),
                spt,
            });
        }
    }
    if units.is_empty() {
        let snippet = parse_snippet(spec, src, &globals, mode, &limits);
        if let Some(spt) = snippet.spt {
            units.push(DumpUnit {
                title: format!(
                    "snippet (wrapper {}, damage {:.3})",
                    snippet.wrapper, snippet.damage
                ),
                spt,
            });
        }
    }
    if units.is_empty() {
        return Err(Failure::new(EXIT_FAILURE, "no unit or snippet found"));
    }
    Ok(units)
}

enum TreeItem {
    Node(u32),
    Leaf(u32),
}

/// Labels and leaves, two spaces per depth (explicit stack).
fn tree_text(spt: &Spt, out: &mut String) {
    let mut stack = vec![(TreeItem::Node(spt.root), 0usize)];
    while let Some((item, depth)) = stack.pop() {
        let indent = "  ".repeat(depth);
        match item {
            TreeItem::Node(n) => {
                let _ = writeln!(out, "{indent}{}", spt.label(n).escape_debug());
                for child in spt.nodes[n as usize].children.iter().rev() {
                    match child {
                        Elem::Tree(c) => stack.push((TreeItem::Node(*c), depth + 1)),
                        Elem::Leaf(l) => stack.push((TreeItem::Leaf(*l), depth + 1)),
                        Elem::Keyword(_) => {}
                    }
                }
            }
            TreeItem::Leaf(l) => {
                let leaf = &spt.leaves[l as usize];
                let var = match (leaf.is_var, leaf.binding) {
                    (true, Some(s)) => format!("  #VAR s{}", s.0),
                    (true, None) => "  #VAR".to_string(),
                    (false, _) => String::new(),
                };
                let _ = writeln!(out, "{indent}{}{var}", leaf.text.escape_debug());
            }
        }
    }
}

fn cmd_dump_spt(a: DumpArgs) -> CliResult {
    let lang = resolve_lang(a.lang, &a.file)?;
    let src = read_text(&a.file)?;
    let mut out = String::new();
    for u in file_units(lang, &src, a.compat)? {
        let _ = writeln!(out, "== {}", u.title);
        tree_text(&u.spt, &mut out);
    }
    print!("{out}");
    Ok(())
}

fn cmd_dump_features(a: DumpFeaturesArgs) -> CliResult {
    let lang = resolve_lang(a.lang, &a.file)?;
    let src = read_text(&a.file)?;
    let cfg = (spec_for(lang)
        .ok_or_else(|| Failure::new(EXIT_USAGE, "unsupported language"))?
        .feature_config)(effective_profile(ProfileName::Reference, a.compat).features);
    let mut out = String::new();
    for u in file_units(lang, &src, a.compat)? {
        let _ = writeln!(out, "== {}", u.title);
        let per_leaf = featurize(&u.spt, &cfg, None);
        if a.with_origins {
            for (i, (leaf, feats)) in u.spt.leaves.iter().zip(&per_leaf).enumerate() {
                let _ = writeln!(out, "leaf {i} line {} {:?}", leaf.line, &*leaf.text);
                for f in feats {
                    let _ = writeln!(out, "  {}", f.escape_debug());
                }
            }
        } else {
            let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
            for f in per_leaf.iter().flatten() {
                *counts.entry(f.as_str()).or_insert(0) += 1;
            }
            for (f, n) in counts {
                let _ = writeln!(out, "{n:>4}  {}", f.escape_debug());
            }
        }
    }
    print!("{out}");
    Ok(())
}

fn feature_counts(lang: Lang, path: &Path, compat: bool) -> Result<BTreeMap<String, u32>, Failure> {
    let src = read_text(path)?;
    let cfg = (spec_for(lang)
        .ok_or_else(|| Failure::new(EXIT_USAGE, "unsupported language"))?
        .feature_config)(effective_profile(ProfileName::Reference, compat).features);
    let mut counts = BTreeMap::new();
    for u in file_units(lang, &src, compat)? {
        for f in featurize(&u.spt, &cfg, None).into_iter().flatten() {
            *counts.entry(f).or_insert(0) += 1;
        }
    }
    Ok(counts)
}

fn cmd_feature_diff(a: FeatureDiffArgs) -> CliResult {
    let lang = resolve_lang(a.lang, &a.a)?;
    let left = feature_counts(lang, &a.a, a.compat)?;
    let right = feature_counts(lang, &a.b, a.compat)?;
    let keys: BTreeSet<&String> = left.keys().chain(right.keys()).collect();
    let (mut shared, mut union) = (0u64, 0u64);
    let mut only_left: Vec<(&String, u32)> = Vec::new();
    let mut only_right: Vec<(&String, u32)> = Vec::new();
    for k in keys {
        let x = left.get(k).copied().unwrap_or(0);
        let y = right.get(k).copied().unwrap_or(0);
        shared += u64::from(x.min(y));
        union += u64::from(x.max(y));
        if x > y {
            only_left.push((k, x - y));
        } else if y > x {
            only_right.push((k, y - x));
        }
    }
    let jaccard = if union == 0 {
        0.0
    } else {
        shared as f64 / union as f64
    };
    let mut out = format!("jaccard {jaccard:.3} ({shared} shared of {union})\n");
    for (path, list) in [(&a.a, &only_left), (&a.b, &only_right)] {
        let total: u32 = list.iter().map(|(_, n)| n).sum();
        let _ = writeln!(out, "only in {} ({total}):", path.display());
        for (f, n) in list.iter() {
            let _ = writeln!(out, "{n:>4}  {}", f.escape_debug());
        }
    }
    print!("{out}");
    Ok(())
}

fn cmd_inspect(a: InspectArgs) -> CliResult {
    let root = find_root(a.root)?;
    let index = Index::open(&root).map_err(index_failure)?;
    index.verify().map_err(index_failure)?;
    let stats = index.stats();
    if a.json {
        let v = serde_json::json!({
            "root": root.display().to_string(),
            "manifest": index.manifest(),
            "stats": stats,
            "integrity": "ok",
        });
        println!("{}", serde_json::to_string_pretty(&v).map_err(json_err)?);
    } else {
        print!("{}", inspect_text(&root, &index, &stats));
    }
    Ok(())
}

fn inspect_text(root: &Path, index: &Index, s: &IndexStats) -> String {
    let m = index.manifest();
    let pairs = |map: &BTreeMap<String, String>| {
        map.iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let counts = |map: &BTreeMap<String, u32>| {
        map.iter()
            .map(|(k, v)| format!("{k} {v}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut w = String::new();
    let _ = writeln!(
        w,
        "index generation {} at {}",
        m.generation,
        root.join(".aroma").display()
    );
    let _ = writeln!(
        w,
        "profile {}, compat {}, schema {}, feature schema {}, fingerprint {:#018x}",
        profile_name(m.profile),
        m.compat,
        m.schema_version,
        m.feature_schema,
        m.feature_fingerprint
    );
    let _ = writeln!(w, "adapters: {}", pairs(&m.adapters));
    let _ = writeln!(w, "grammars: {}", pairs(&m.grammars));
    let _ = writeln!(
        w,
        "units {} ({} tests) in {} files; {} features, {} postings",
        s.n_units, s.test_units, s.n_files, s.n_features, s.n_postings
    );
    let _ = writeln!(w, "integrity ok (postings and lexical arrays verified)");
    let _ = writeln!(w, "by language: {}", counts(&s.units_by_lang));
    let _ = writeln!(w, "by kind: {}", counts(&s.units_by_kind));
    let _ = writeln!(w, "oversize by kind: {}", counts(&s.oversize_units_by_kind));
    let _ = writeln!(w, "split by kind: {}", counts(&s.split_units_by_kind));
    if let Some(b) = &s.build {
        let _ = writeln!(
            w,
            "last build: {} files seen, {} parsed, {} unchanged, {} too large, {} failed, {} units skipped as damaged",
            b["files_seen"], b["files_reparsed"], b["files_unchanged"],
            b["files_skipped_large"], b["files_failed"], b["units_skipped_damaged"]
        );
    }
    w
}
