//! `recall`: paper §5 search recall (Table 4) on real repositories, plus the helpers the
//! `idiom` and `perf` harnesses share (seeded RNG, pinned clones, corpus paths, sampling).
//!
//! ```text
//! cargo run --release -p xtask -- recall [--only java|apex] [--java-repos N] [--samples N] [--seed S]
//! cargo run --release -p xtask -- recall --repo PATH --lang <specced-lang> [--samples N] [--seed S]
//! cargo run --release -p xtask -- recall --suite v1.1
//! cargo run --release -p xtask -- recall --suite v1.2 --sfmeta-mode lexical|structural
//! cargo run --release -p xtask -- recall --suite v1.3
//! ```
//!
//! Protocol (paper evaluation, D43): build the index once with the `Index` API; sample
//! (xorshift, seed 119, as in the M-1 pilot) non-test units meeting the language's sampling
//! policy. Java and Apex keep the historical rule: >= 12 lines and >= 5 non-blank interior
//! lines, start-only slice. Every other language samples short units (>= 4 lines, >= 3
//! interior) with start, middle and end slice offsets and a query length of min(5, interior
//! lines). Each slice is queried (contiguous-style) and 5 random interior lines in source
//! order are queried non-contiguously. The source stays in the index: Recall@n = share of
//! queries whose source (or its clone representative: same body_hash) ranks within the top
//! n, counted by the source's displayed position (RRF order) in the result list. 30
//! bootstrap resamples of the per-query outcomes give mean ± 1.96·sd/√30. Extraction
//! coverage (units / files / units per kind) and a per-kind breakdown are printed for the
//! newer languages; kinds with fewer than 100 queries are reported, not gated.
//!
//! Gates (definition of done): Java contiguous Recall@1 >= 0.95 and Recall@100 >= 0.99 on
//! the first N repositories of the reference `github_repositories.csv`; Apex contiguous
//! Recall@100 >= 0.98 pooled over the three M-1 pilot repositories. `--repo` only reports.
//! The v1.1 suite builds each pinned v1.1 repository and pilot separately with both LWC
//! languages, then pools query outcomes by language, kind and offset (D38, D43).
//! v1.2 does the same for sfmeta (up to --samples units per kind per repository),
//! gating pooled start R@100 >= 0.95 for flow_element, validation_rule and field.
//! v1.3 builds each pinned v1.3 repository and pilot separately for the v1.3 languages
//! that are in `Lang::SPECCED` at run time (aura, aurajs, vf), gates each on pooled
//! start R@100 >= 0.98 and enforces the corpus floor (>= 300 Aura bundles, >= 300 VF
//! pages) from the indexer's own extraction.

use aroma_core::lang::Lang;
use aroma_core::profile::ProfileName;
use aroma_core::unit::{UnitKind, UnitStore};
use aroma_lang::build::{build_index, BuildError, BuildOptions, BuildReport, SfmetaMode};
use aroma_lang::index::{Index, QueryOptions, QueryOutput};
use aroma_lang::kraw::KRaw;
use aroma_lang::spec::spec_for;
use aroma_lang::table::{extract_units, parse, Limits};
use std::collections::{BTreeSet, HashMap};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// M-1 pilot repositories (docs/results/17-pilot-results.md) at the pilot commits.
pub(crate) const PILOT_REPOS: [(&str, &str, &str); 3] = [
    (
        "apex-recipes",
        "https://github.com/trailheadapps/apex-recipes",
        "eda6167c5126a34daf66d9e1371ff6957ae223bd",
    ),
    (
        "NebulaLogger",
        "https://github.com/jongpie/NebulaLogger",
        "b792ac8d8b4eb31b14a164a24951c3cb4125a616",
    ),
    (
        "NPSP",
        "https://github.com/SalesforceFoundation/NPSP",
        "f5072b44781e7f31cb396c6919cfa7d0ead0ec6a",
    ),
];
pub(crate) const DEFAULT_SEED: u64 = 119;
const DEFAULT_JAVA_REPOS: usize = 20;
const DEFAULT_SAMPLES: usize = 1000;
const BOOTSTRAP_RESAMPLES: usize = 30;
/// Maximum number of interior lines in a single query; shorter units use fewer.
const QUERY_LINES: usize = 5;
/// A (language, kind) needs this many queries before it is gated (D43).
const GATED_KIND_QUERIES: usize = 100;
const JAVA_MIN_AT1: f64 = 0.95;
const JAVA_MIN_AT100: f64 = 0.99;
const APEX_MIN_AT100: f64 = 0.98;

/// Where a contiguous (contiguous-style) query slice is taken from a unit's interior.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Offset {
    /// First lines (today's contiguous slice).
    Start,
    /// Slice centred in the interior lines.
    Middle,
    /// Last lines.
    End,
}

/// Per-language recall-sampling policy (D43).
pub(crate) struct SamplingPolicy {
    pub(crate) min_unit_lines: u32,
    pub(crate) min_interior_lines: usize,
    pub(crate) offsets: &'static [Offset],
}

/// Code languages keep today's exact 12-line / 5-interior, start-only behaviour.
static CODE_POLICY: SamplingPolicy = SamplingPolicy {
    min_unit_lines: 12,
    min_interior_lines: 5,
    offsets: &[Offset::Start],
};

/// Markup and metadata languages sample short units (4 lines / 3 interior) at start,
/// middle and end.
static MARKUP_POLICY: SamplingPolicy = SamplingPolicy {
    min_unit_lines: 4,
    min_interior_lines: 3,
    offsets: &[Offset::Start, Offset::Middle, Offset::End],
};

pub(crate) fn policy_for(lang: Lang) -> &'static SamplingPolicy {
    match lang {
        Lang::Java | Lang::Apex => &CODE_POLICY,
        _ => &MARKUP_POLICY,
    }
}

const USAGE: &str = "usage: cargo run --release -p xtask -- recall [--suite v1.1|v1.2|v1.3] [--sfmeta-mode structural|lexical (v1.2 only)] [--only java|apex] [--java-repos N] [--samples N] [--seed S] | --repo PATH --lang <specced-lang>";

/// The M-1 pilot's xorshift64 generator (`x ^= x << 13; x ^= x >> 7; x ^= x << 17`).
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Rng {
        Rng(seed.max(1))
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Uniform-enough index in `0..n` (`n > 0`).
    pub(crate) fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// Fisher–Yates, as in the pilot.
    pub(crate) fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.below(i + 1);
            v.swap(i, j);
        }
    }
}

pub(crate) fn num<T: std::str::FromStr>(value: Option<&String>, flag: &str) -> Result<T, String> {
    value
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| format!("{flag} needs a number"))
}

/// `$CARGO_TARGET_DIR` (relative to the workspace) or `<workspace>/target`.
pub(crate) fn target_dir() -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(p) => {
            let p = PathBuf::from(p);
            if p.is_absolute() {
                p
            } else {
                crate::workspace_root().join(p)
            }
        }
        None => crate::workspace_root().join("target"),
    }
}

pub(crate) fn corpus_dir() -> PathBuf {
    target_dir().join("corpus")
}

/// `$AROMA_REFERENCE_DIR` or `<workspace>/ref/aroma-paper-artifacts`.
pub(crate) fn reference_dir() -> PathBuf {
    std::env::var_os("AROMA_REFERENCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            crate::workspace_root()
                .join("ref")
                .join("aroma-paper-artifacts")
        })
}

/// Shallow checkout of `url` at exactly `sha` in `dest` (reused when already there).
pub(crate) fn clone_at(url: &str, sha: &str, dest: &Path) -> Result<(), String> {
    if dest.join(".git").exists() {
        if crate::git(Some(dest), &["rev-parse", "HEAD"])
            .ok()
            .as_deref()
            == Some(sha)
        {
            return Ok(());
        }
    } else {
        std::fs::create_dir_all(dest).map_err(|e| format!("create {}: {e}", dest.display()))?;
        crate::git(Some(dest), &["init", "--quiet"])?;
        crate::git(Some(dest), &["remote", "add", "origin", url])?;
    }
    crate::git(
        Some(dest),
        &["fetch", "--quiet", "--depth", "1", "origin", sha],
    )?;
    crate::git(
        Some(dest),
        &["checkout", "--quiet", "--force", "--detach", "FETCH_HEAD"],
    )?;
    let head = crate::git(Some(dest), &["rev-parse", "HEAD"])?;
    if head != sha {
        return Err(format!("{url}: checked out {head}, expected {sha}"));
    }
    Ok(())
}

/// One M-1 pilot repository checked out under `target/corpus/apex/<name>`.
pub(crate) fn pilot_repo(name: &str) -> Result<PathBuf, String> {
    let (_, url, sha) = PILOT_REPOS
        .iter()
        .find(|(n, _, _)| *n == name)
        .ok_or_else(|| format!("unknown pilot repository {name}"))?;
    let dest = corpus_dir().join("apex").join(name);
    clone_at(url, sha, &dest)?;
    Ok(dest)
}

pub(crate) struct RepoRow {
    pub(crate) name: String,
    pub(crate) sha: String,
}

/// `repository,url,commit_sha,commit_date` rows; malformed rows are skipped.
pub(crate) fn parse_repo_csv(text: &str) -> Vec<RepoRow> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(',').collect();
            let (name, sha) = (*fields.first()?, *fields.get(2)?);
            (name.contains('/') && sha.len() == 40).then(|| RepoRow {
                name: name.to_string(),
                sha: sha.to_string(),
            })
        })
        .collect()
}

/// The first `n_repos` reference repositories under `target/corpus/java` (one index
/// root); other directories there are removed so the root holds exactly that subset.
pub(crate) fn java_corpus(n_repos: usize) -> Result<PathBuf, String> {
    let csv = reference_dir()
        .join("datasets")
        .join("github_repositories.csv");
    let text = std::fs::read_to_string(&csv).map_err(|e| {
        format!(
            "{}: {e}; run `cargo run -p xtask -- fetch-reference` or set AROMA_REFERENCE_DIR",
            csv.display()
        )
    })?;
    let root = corpus_dir().join("java");
    let mut keep = BTreeSet::new();
    for row in parse_repo_csv(&text).into_iter().take(n_repos) {
        let dir_name = row.name.replace('/', "__");
        let dest = root.join(&dir_name);
        match clone_at(&format!("https://github.com/{}", row.name), &row.sha, &dest) {
            Ok(()) => {
                keep.insert(dir_name);
            }
            Err(e) => eprintln!("recall: skipping {}: {e}", row.name),
        }
    }
    if keep.is_empty() {
        return Err("no Java corpus repository could be fetched".into());
    }
    let entries = std::fs::read_dir(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name != ".aroma" && !keep.contains(&name) {
            eprintln!("recall: removing {name} (outside the first {n_repos} repositories)");
            std::fs::remove_dir_all(entry.path()).map_err(|e| format!("{name}: {e}"))?;
        }
    }
    Ok(root)
}

/// Incremental build of `root` for `langs` with the reference profile.
pub(crate) fn build(root: &Path, langs: &[Lang]) -> Result<BuildReport, String> {
    build_with_mode(root, langs, SfmetaMode::BUILD_DEFAULT)
}

pub(crate) fn parse_sfmeta_mode(value: Option<&String>) -> Result<SfmetaMode, String> {
    match value.map(String::as_str) {
        Some("lexical") => Ok(SfmetaMode::Lexical),
        Some("structural") => Ok(SfmetaMode::Structural),
        _ => Err("--sfmeta-mode needs structural or lexical".into()),
    }
}

pub(crate) fn mode_name(mode: SfmetaMode) -> &'static str {
    match mode {
        SfmetaMode::Lexical => "lexical",
        SfmetaMode::Structural => "structural",
    }
}

pub(crate) fn build_with_mode(
    root: &Path,
    langs: &[Lang],
    mode: SfmetaMode,
) -> Result<BuildReport, String> {
    build_index(
        &BuildOptions {
            root: root.to_path_buf(),
            langs: langs.to_vec(),
            profile: ProfileName::Reference,
            compat: false,
            lock_timeout: Duration::from_secs(60),
            jobs: None,
        }
        .with_sfmeta_mode(mode),
    )
    .map_err(|e| format!("index {}: {e}", root.display()))
}

fn token_bounds(k: &KRaw) -> Option<(u32, u32)> {
    let (mut first, mut last) = (None, None);
    let mut stack = vec![k];
    while let Some(node) = stack.pop() {
        match node {
            KRaw::Token { bytes, .. } => {
                if first.is_none() {
                    first = Some(bytes.start);
                }
                last = Some(bytes.start);
            }
            KRaw::Node { children, .. } => stack.extend(children.iter().rev()),
        }
    }
    Some((first?, last?))
}

fn line_of(src: &str, byte: u32) -> u32 {
    let end = (byte as usize).min(src.len());
    src.as_bytes()[..end]
        .iter()
        .filter(|&&b| b == b'\n')
        .count() as u32
        + 1
}

/// Non-blank, non-comment-only lines strictly between the line of the unit body's first
/// token (`{`) and the line of its last token (`}`), verbatim. Lines that hold only a `//`
/// comment or lie inside a `/* */` block are skipped (paper §5 samples lines of code); a
/// line with code before a comment is kept. `None` when no unit has `unit_bytes`.
pub(crate) fn body_lines(src: &str, lang: Lang, unit_bytes: &Range<u32>) -> Option<Vec<String>> {
    let spec = spec_for(lang)?;
    let limits = Limits::DEFAULT;
    let tree = parse(spec, src, &limits).ok()?;
    let unit = extract_units(&tree, src, spec, &limits)
        .into_iter()
        .find(|u| u.bytes == *unit_bytes)?;
    interior_lines(src, &unit.body)
}

fn interior_lines(src: &str, body: &KRaw) -> Option<Vec<String>> {
    let (open, close) = token_bounds(body)?;
    let inside = (line_of(src, open) + 1)..line_of(src, close);
    Some(
        src.lines()
            .enumerate()
            .filter(|(i, _)| inside.contains(&(*i as u32 + 1)))
            .map(|(_, l)| l)
            .filter(code_line_filter())
            .map(str::to_string)
            .collect(),
    )
}

/// D44 split units can share a large metadata file. Extract it once, preserving
/// exactly the same body-interior lines as individual `body_lines` calls.
fn metadata_body_lines(src: &str) -> HashMap<(u32, u32), Vec<String>> {
    let spec = spec_for(Lang::SfMeta).expect("sfmeta adapter");
    let limits = Limits::DEFAULT;
    let Ok(tree) = parse(spec, src, &limits) else {
        return HashMap::new();
    };
    let mut lines = HashMap::new();
    for unit in extract_units(&tree, src, spec, &limits) {
        if let Some(interior) = interior_lines(src, &unit.body) {
            lines
                .entry((unit.bytes.start, unit.bytes.end))
                .or_insert(interior);
        }
    }
    lines
}

/// Stateful line filter: true for lines that carry code (see [`body_lines`]).
fn code_line_filter() -> impl FnMut(&&str) -> bool {
    let mut in_block = false;
    move |line: &&str| {
        let mut t = line.trim();
        if in_block {
            match t.find("*/") {
                Some(end) => {
                    in_block = false;
                    t = t[end + 2..].trim();
                }
                None => return false,
            }
        }
        loop {
            if t.is_empty() || t.starts_with("//") {
                return false;
            }
            if let Some(rest) = t.strip_prefix("/*") {
                match rest.find("*/") {
                    Some(end) => t = rest[end + 2..].trim(),
                    None => {
                        in_block = true;
                        return false;
                    }
                }
            } else {
                return true;
            }
        }
    }
}

pub(crate) struct Sample {
    pub(crate) body_hash: u64,
    pub(crate) kind: UnitKind,
    /// Non-blank body-interior lines (at least the policy's `min_interior_lines`).
    pub(crate) lines: Vec<String>,
}

/// Up to `n` seeded-random non-test units of `lang` meeting the language's sampling
/// policy (D43): Java/Apex need >= 12 lines and >= 5 interior lines (start only); markup
/// and metadata need >= 4 lines and >= 3 interior lines (start, middle, end).
pub(crate) fn sample_units(index: &Index, lang: Lang, n: usize, rng: &mut Rng) -> Vec<Sample> {
    let policy = policy_for(lang);
    let mut ids: Vec<u32> = (0..index.n_units())
        .filter(|&id| {
            let r = index.record(id);
            r.lang == lang && !r.is_test && r.lines.end - r.lines.start >= policy.min_unit_lines
        })
        .collect();
    rng.shuffle(&mut ids);
    let mut sources: HashMap<u32, Option<String>> = HashMap::new();
    let mut metadata_lines = HashMap::new();
    let mut out = Vec::new();
    let mut per_kind: HashMap<UnitKind, usize> = HashMap::new();
    for id in ids {
        if lang != Lang::SfMeta && out.len() == n {
            break;
        }
        let rec = index.record(id);
        // Metadata's sparse rules/formulas must not compete with ordinary fields
        // for the per-repository sample budget (D43: sampling per unit kind).
        if lang == Lang::SfMeta && per_kind.get(&rec.kind).copied().unwrap_or(0) >= n {
            continue;
        }
        let src = sources.entry(rec.file).or_insert_with(|| {
            std::fs::read(index.root().join(index.file_path(rec.file)))
                .ok()
                .map(|b| String::from_utf8_lossy(&b).into_owned())
        });
        let Some(src) = src.as_deref() else {
            continue;
        };
        let lines = if lang == Lang::SfMeta {
            metadata_lines
                .entry(rec.file)
                .or_insert_with(|| metadata_body_lines(src))
                .get(&(rec.bytes.start, rec.bytes.end))
                .cloned()
        } else {
            body_lines(src, lang, &rec.bytes)
        };
        let Some(lines) = lines else {
            continue;
        };
        if lines.len() >= policy.min_interior_lines {
            *per_kind.entry(rec.kind).or_default() += 1;
            out.push(Sample {
                body_hash: rec.body_hash,
                kind: rec.kind,
                lines,
            });
        }
    }
    out
}

/// Number of interior lines in a query: at most `QUERY_LINES` (shorter units use fewer).
pub(crate) fn query_len(interior: usize) -> usize {
    QUERY_LINES.min(interior)
}

/// The first `n` interior lines, newline-terminated (offset Start).
pub(crate) fn start_query(s: &Sample, n: usize) -> String {
    let mut q = s.lines[..n].join("\n");
    q.push('\n');
    q
}

/// `n` interior lines centred in the interior (offset Middle), newline-terminated.
pub(crate) fn middle_query(s: &Sample, n: usize) -> String {
    let start = (s.lines.len() - n) / 2;
    let mut q = s.lines[start..start + n].join("\n");
    q.push('\n');
    q
}

/// The last `n` interior lines (offset End), newline-terminated.
pub(crate) fn end_query(s: &Sample, n: usize) -> String {
    let len = s.lines.len();
    let mut q = s.lines[len - n..].join("\n");
    q.push('\n');
    q
}

/// First `min(5, interior)` interior lines, newline-terminated (today's contiguous query;
/// offset Start).
pub(crate) fn contiguous_query(s: &Sample) -> String {
    start_query(s, query_len(s.lines.len()))
}

/// Up to five random interior lines in source order, newline-terminated.
pub(crate) fn random_query(s: &Sample, rng: &mut Rng) -> String {
    let mut idx: Vec<usize> = (0..s.lines.len()).collect();
    rng.shuffle(&mut idx);
    let mut pick = idx[..query_len(s.lines.len())].to_vec();
    pick.sort_unstable();
    let mut q = pick
        .iter()
        .map(|&i| s.lines[i].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    q.push('\n');
    q
}

/// 0-based rank of the source's body hash in the result list: the displayed position
/// (RRF order) of the first result whose body hash equals the source hash. There is no
/// tie promotion based on structural score: `out.similar` is ordered by reciprocal-rank
/// fusion, not by `score`, so a source shown at position 3 ranks as 3 even if its
/// `score` equals or exceeds `similar[0].score`.
pub(crate) fn source_rank(
    out: &QueryOutput,
    source_hash: u64,
    hash_of: &HashMap<(String, u32), u64>,
) -> Option<usize> {
    out.similar
        .iter()
        .position(|s| hash_of.get(&(s.path.clone(), s.lines[0])) == Some(&source_hash))
}

/// Per-query Recall@1 / Recall@100 outcomes.
#[derive(Clone, Debug, Default)]
pub(crate) struct Outcomes {
    pub(crate) at1: Vec<bool>,
    pub(crate) at100: Vec<bool>,
}

impl Outcomes {
    fn push(&mut self, rank: Option<usize>) {
        self.at1.push(rank == Some(0));
        self.at100.push(rank.is_some_and(|r| r < 100));
    }

    fn extend(&mut self, other: &Outcomes) {
        self.at1.extend_from_slice(&other.at1);
        self.at100.extend_from_slice(&other.at100);
    }
}

fn mean(v: &[bool]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().filter(|&&b| b).count() as f64 / v.len() as f64
    }
}

/// Mean and 95% half-width (1.96·sd/√B) of `resamples` bootstrap means.
pub(crate) fn bootstrap(outcomes: &[bool], resamples: usize, rng: &mut Rng) -> (f64, f64) {
    if outcomes.is_empty() || resamples == 0 {
        return (0.0, 0.0);
    }
    let n = outcomes.len();
    let means: Vec<f64> = (0..resamples)
        .map(|_| (0..n).filter(|_| outcomes[rng.below(n)]).count() as f64 / n as f64)
        .collect();
    let b = resamples as f64;
    let m = means.iter().sum::<f64>() / b;
    let var = means.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (b - 1.0).max(1.0);
    (m, 1.96 * var.sqrt() / b.sqrt())
}

pub(crate) struct Recall {
    pub(crate) n: usize,
    pub(crate) at1: f64,
    pub(crate) at100: f64,
    pub(crate) boot_at1: (f64, f64),
    pub(crate) boot_at100: (f64, f64),
}

impl std::fmt::Display for Recall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let precision = f.precision().unwrap_or(3);
        write!(
            f,
            "n={} Recall@1 {:.precision$} (bootstrap {:.precision$} ± {:.precision$}) Recall@100 {:.precision$} (bootstrap {:.precision$} ± {:.precision$})",
            self.n,
            self.at1,
            self.boot_at1.0,
            self.boot_at1.1,
            self.at100,
            self.boot_at100.0,
            self.boot_at100.1
        )
    }
}

pub(crate) fn summarize(o: &Outcomes, seed: u64) -> Recall {
    let mut rng = Rng::new(seed.wrapping_add(1));
    Recall {
        n: o.at1.len(),
        at1: mean(&o.at1),
        at100: mean(&o.at100),
        boot_at1: bootstrap(&o.at1, BOOTSTRAP_RESAMPLES, &mut rng),
        boot_at100: bootstrap(&o.at100, BOOTSTRAP_RESAMPLES, &mut rng),
    }
}

pub(crate) struct Evaluation {
    pub(crate) units: u32,
    pub(crate) contiguous: Outcomes,
    pub(crate) random: Outcomes,
    /// Per (kind, offset) contiguous-style query outcomes, sorted for stable output.
    pub(crate) by_kind: Vec<(UnitKind, Offset, Outcomes)>,
    pub(crate) coverage: Coverage,
}

/// Extraction coverage (D43, item 3): units indexed for a language, distinct files
/// routed to it, and units per kind, read from the index.
pub(crate) struct Coverage {
    pub(crate) units: u32,
    pub(crate) files: u32,
    pub(crate) per_kind: Vec<(UnitKind, u32)>,
}

fn coverage(index: &Index, lang: Lang) -> Coverage {
    let mut files = BTreeSet::new();
    let mut counts: HashMap<UnitKind, u32> = HashMap::new();
    let mut units = 0;
    for id in 0..index.n_units() {
        let r = index.record(id);
        if r.lang == lang {
            units += 1;
            files.insert(r.file);
            *counts.entry(r.kind).or_insert(0) += 1;
        }
    }
    let mut per_kind: Vec<(UnitKind, u32)> = counts.into_iter().collect();
    per_kind.sort_by_key(|(k, n)| (kind_name(*k).to_string(), *n));
    Coverage {
        units,
        files: files.len() as u32,
        per_kind,
    }
}

/// The v1.3 gate languages that have a landed adapter at run time. `aurajs` is added
/// by another lane and joins the suite automatically once it is in [`Lang::SPECCED`].
fn v13_languages() -> Vec<Lang> {
    [Lang::Aura, Lang::Vf, Lang::AuraJs]
        .into_iter()
        .filter(|l| Lang::SPECCED.contains(l))
        .collect()
}

/// Corpus floors from the indexer's own extraction (D46): Aura bundles = distinct
/// `aura/<bundle>/` folders holding >= 1 `aura` unit; VF pages = distinct `.page`
/// files with a `Page` unit. `.component` files with a `Page` unit are reported
/// separately (the floor counts pages only).
#[derive(Debug, Default)]
struct V13Floors {
    aura_bundles: BTreeSet<String>,
    vf_pages: BTreeSet<String>,
    vf_components: BTreeSet<String>,
}

impl V13Floors {
    /// Fold this repository's index into the running floor counts.
    fn add_index(&mut self, index: &Index) {
        for id in 0..index.n_units() {
            let rec = index.record(id);
            let rel = index
                .file_path(rec.file)
                .to_string_lossy()
                .replace('\\', "/");
            match rec.lang {
                Lang::Aura => {
                    if let Some(bundle) = aura_bundle(&rel) {
                        self.aura_bundles.insert(bundle);
                    }
                }
                Lang::Vf if rec.kind == UnitKind::Page => {
                    let lower = rel.to_ascii_lowercase();
                    if lower.ends_with(".page") {
                        self.vf_pages.insert(rel);
                    } else if lower.ends_with(".component") {
                        self.vf_components.insert(rel);
                    }
                }
                _ => {}
            }
        }
    }
}

/// The bundle folder name of an `aura/<bundle>/<file>` path, if any.
fn aura_bundle(rel: &str) -> Option<String> {
    let mut parts = rel.split('/');
    let _file = parts.next_back()?;
    let bundle = parts.next_back()?;
    let aura = parts.next_back()?;
    aura.eq_ignore_ascii_case("aura")
        .then(|| bundle.to_string())
}

/// Build `root` once, sample, query, rank.
pub(crate) fn evaluate(
    root: &Path,
    lang: Lang,
    samples: usize,
    seed: u64,
) -> Result<Evaluation, String> {
    build(root, &[lang])?;
    let index = Index::open(root).map_err(|e| format!("open {}: {e}", root.display()))?;
    Ok(evaluate_index(&index, lang, samples, seed))
}

/// Evaluate a language without rebuilding the repository's shared LWC index.
fn evaluate_index(index: &Index, lang: Lang, samples: usize, seed: u64) -> Evaluation {
    let mut hash_of = HashMap::new();
    for id in 0..index.n_units() {
        let r = index.record(id);
        hash_of.insert(
            (
                index.file_path(r.file).to_string_lossy().into_owned(),
                r.lines.start,
            ),
            r.body_hash,
        );
    }
    let mut rng = Rng::new(seed);
    let policy = policy_for(lang);
    let picked = sample_units(index, lang, samples, &mut rng);
    let mut opts = QueryOptions::new(lang);
    opts.top_similar = 100;
    let mut contiguous = Outcomes::default();
    let mut random = Outcomes::default();
    let mut by_kind: HashMap<(UnitKind, Offset), Outcomes> = HashMap::new();
    for s in &picked {
        let n = query_len(s.lines.len());
        let mut per_offset: HashMap<Offset, Outcomes> = HashMap::new();
        for &off in policy.offsets {
            let q = match off {
                Offset::Start => start_query(s, n),
                Offset::Middle => middle_query(s, n),
                Offset::End => end_query(s, n),
            };
            let out = index.query(&q, &opts);
            let rank = source_rank(&out, s.body_hash, &hash_of);
            per_offset.entry(off).or_default().push(rank);
            if off == Offset::Start {
                contiguous.push(rank);
            }
        }
        for (off, o) in per_offset {
            by_kind.entry((s.kind, off)).or_default().extend(&o);
        }
        let out = index.query(&random_query(s, &mut rng), &opts);
        random.push(source_rank(&out, s.body_hash, &hash_of));
    }
    let mut by_kind: Vec<(UnitKind, Offset, Outcomes)> = by_kind
        .into_iter()
        .map(|((kind, off), o)| (kind, off, o))
        .collect();
    by_kind.sort_by_key(|(kind, off, _)| {
        (kind_name(*kind).to_string(), offset_name(*off).to_string())
    });
    let coverage = coverage(index, lang);
    Evaluation {
        units: coverage.units,
        contiguous,
        random,
        by_kind,
        coverage,
    }
}

/// Stable lower-case name of a `UnitKind`, for report lines and deterministic ordering.
fn kind_name(k: UnitKind) -> &'static str {
    match k {
        UnitKind::Method => "method",
        UnitKind::Constructor => "constructor",
        UnitKind::Trigger => "trigger",
        UnitKind::Accessor => "accessor",
        UnitKind::Initializer => "initializer",
        UnitKind::Snippet => "snippet",
        UnitKind::Template => "template",
        UnitKind::MarkupBlock => "markup_block",
        UnitKind::WireMember => "wire_member",
        UnitKind::FlowElement => "flow_element",
        UnitKind::Field => "field",
        UnitKind::ValidationRule => "validation_rule",
        UnitKind::MetadataRecord => "metadata_record",
        UnitKind::Page => "page",
        UnitKind::Component => "component",
    }
}

fn offset_name(o: Offset) -> &'static str {
    match o {
        Offset::Start => "start",
        Offset::Middle => "middle",
        Offset::End => "end",
    }
}

/// Tag appended to a per-kind line when the kind has fewer than 100 queries (not gated).
fn gated_tag(n: usize) -> &'static str {
    if n >= GATED_KIND_QUERIES {
        ""
    } else {
        " reported (n < 100), not gated"
    }
}

fn at_least(failures: &mut Vec<String>, what: &str, value: f64, min: f64) {
    if value < min {
        failures.push(format!("{what} {value:.4} < {min:.2}"));
    }
}

struct Config {
    v11: bool,
    v12: bool,
    v13: bool,
    sfmeta_mode: SfmetaMode,
    java: bool,
    apex: bool,
    java_repos: usize,
    samples: usize,
    seed: u64,
    repo: Option<PathBuf>,
    lang: Option<Lang>,
}

fn parse_args(args: &[String]) -> Result<Config, String> {
    let mut c = Config {
        v11: false,
        v12: false,
        v13: false,
        sfmeta_mode: SfmetaMode::BUILD_DEFAULT,
        java: true,
        apex: true,
        java_repos: DEFAULT_JAVA_REPOS,
        samples: DEFAULT_SAMPLES,
        seed: DEFAULT_SEED,
        repo: None,
        lang: None,
    };
    let mut it = args.iter();
    let mut mode_seen = false;
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--suite" => match it.next().map(String::as_str) {
                Some("v1.1") if !c.v11 && !c.v12 && !c.v13 => c.v11 = true,
                Some("v1.2") if !c.v11 && !c.v12 && !c.v13 => c.v12 = true,
                Some("v1.3") if !c.v11 && !c.v12 && !c.v13 => c.v13 = true,
                _ => return Err(format!("--suite needs one of v1.1, v1.2 or v1.3\n{USAGE}")),
            },
            "--sfmeta-mode" => {
                c.sfmeta_mode = parse_sfmeta_mode(it.next())?;
                mode_seen = true;
            }
            "--java-repos" => c.java_repos = num(it.next(), arg)?,
            "--samples" => c.samples = num(it.next(), arg)?,
            "--seed" => c.seed = num(it.next(), arg)?,
            "--only" => match it.next().map(String::as_str) {
                Some("java") => c.apex = false,
                Some("apex") => c.java = false,
                _ => return Err(format!("--only needs java or apex\n{USAGE}")),
            },
            "--repo" => c.repo = Some(PathBuf::from(it.next().ok_or("--repo needs a path")?)),
            "--lang" => {
                let s = it.next().ok_or("--lang needs a language")?;
                let lang =
                    Lang::parse(s).ok_or_else(|| format!("unknown language {s:?}\n{USAGE}"))?;
                if !Lang::SPECCED.contains(&lang) {
                    return Err(format!(
                        "language {s:?} is known but not specced yet (no adapter landed)\n{USAGE}"
                    ));
                }
                c.lang = Some(lang);
            }
            other => return Err(format!("unknown recall option {other:?}\n{USAGE}")),
        }
    }
    if mode_seen && !c.v12 {
        return Err("--sfmeta-mode requires --suite v1.2".into());
    }
    if (c.v11 || c.v12 || c.v13)
        && (c.repo.is_some()
            || c.lang.is_some()
            || !c.java
            || !c.apex
            || c.java_repos != DEFAULT_JAVA_REPOS)
    {
        return Err(
            "--suite cannot be combined with --repo, --lang, --only or --java-repos".into(),
        );
    }
    Ok(c)
}

fn print_eval(label: &str, e: &Evaluation, seed: u64) {
    println!("recall {label} ({} units)", e.units);
    println!("  contiguous     {}", summarize(&e.contiguous, seed));
    println!("  non-contiguous {}", summarize(&e.random, seed));
}

/// Java and Apex keep today's two-line report byte-identical; the per-kind breakdown and
/// extraction-coverage lines are printed only for the newer languages (D43, items 2-3).
fn is_legacy(lang: Lang) -> bool {
    matches!(lang, Lang::Java | Lang::Apex)
}

/// Extraction coverage + per-kind breakdown for languages with no legacy gate (D43).
fn print_extended(label: &str, e: &Evaluation, seed: u64) {
    let Coverage {
        units,
        files,
        per_kind,
    } = &e.coverage;
    println!("recall {label} coverage: {units} units across {files} files routed to this language");
    for (k, n) in per_kind {
        println!("  {:<16} {n} units", kind_name(*k));
    }
    println!("recall {label} per-kind:");
    for (kind, off, o) in &e.by_kind {
        let r = summarize(o, seed);
        println!(
            "  kind={:<16} offset={:<6} n={} Recall@1 {:.3} (bootstrap {:.3} ± {:.3}) Recall@100 {:.3} (bootstrap {:.3} ± {:.3}){}",
            kind_name(*kind),
            offset_name(*off),
            r.n,
            r.at1,
            r.boot_at1.0,
            r.boot_at1.1,
            r.at100,
            r.boot_at100.0,
            r.boot_at100.1,
            gated_tag(r.n)
        );
    }
}

#[derive(Default)]
struct LwcPool {
    start: Outcomes,
    random: Outcomes,
    by_kind: HashMap<(UnitKind, Offset), Outcomes>,
}

impl LwcPool {
    fn extend(&mut self, evaluation: &Evaluation) {
        self.start.extend(&evaluation.contiguous);
        self.random.extend(&evaluation.random);
        for (kind, offset, outcomes) in &evaluation.by_kind {
            self.by_kind
                .entry((*kind, *offset))
                .or_default()
                .extend(outcomes);
        }
    }

    fn print(&self, lang: Lang, seed: u64) {
        let lang = lang.as_str();
        println!(
            "recall POOLED {lang} start {}",
            summarize(&self.start, seed)
        );
        println!(
            "recall POOLED {lang} non-contiguous {} (reported, not gated)",
            summarize(&self.random, seed)
        );
        let mut entries: Vec<_> = self.by_kind.iter().collect();
        entries.sort_by_key(|((kind, offset), _)| (kind_name(*kind), offset_name(*offset)));
        for ((kind, offset), outcomes) in entries {
            let precision = if lang == "sfmeta" { 4 } else { 3 };
            println!(
                "recall POOLED {lang} kind={} offset={} {:.precision$}{}{}",
                kind_name(*kind),
                offset_name(*offset),
                summarize(outcomes, seed),
                gated_tag(outcomes.at1.len()),
                if *offset == Offset::Start
                    && (lang != "sfmeta"
                        || matches!(
                            kind,
                            UnitKind::FlowElement | UnitKind::ValidationRule | UnitKind::Field
                        ))
                {
                    ""
                } else {
                    " (reported, not gated)"
                }
            );
        }
    }
}

/// D38 gates language-wide pooled start offsets. D43 per-kind and other-offset
/// lines remain visible, including kinds with too few samples to gate on their own.
fn v11_gate_line(js: &Outcomes, html: &Outcomes, seed: u64) -> (String, bool) {
    let js = summarize(js, seed);
    let html = summarize(html, seed);
    let mut failures = Vec::new();
    at_least(&mut failures, "lwcjs pooled start R@1", js.at1, 0.95);
    at_least(&mut failures, "lwcjs pooled start R@100", js.at100, 0.99);
    at_least(
        &mut failures,
        "lwchtml pooled start R@100",
        html.at100,
        0.98,
    );
    let passed = failures.is_empty();
    let mut line = format!(
        "v1.1 recall gates {}: lwcjs pooled start n={} R@1 {:.4} (gate 0.95) R@100 {:.4} (gate 0.99); lwchtml pooled start n={} R@100 {:.4} (gate 0.98)",
        if passed { "passed" } else { "failed" }, js.n, js.at1, js.at100, html.n, html.at100
    );
    if !passed {
        line.push_str(&format!("; {}", failures.join("; ")));
    }
    (line, passed)
}

fn run_v11(c: &Config) -> Result<(), String> {
    let corpus = crate::corpus::load()?;
    let mut repos: Vec<_> = corpus
        .stage(Some("v1.1"))
        .map(|r| Ok((r.name.clone(), r.fetch()?)))
        .collect::<Result<_, String>>()?;
    for (name, _, _) in PILOT_REPOS {
        repos.push((name.into(), pilot_repo(name)?));
    }
    let (mut js, mut html) = (LwcPool::default(), LwcPool::default());
    for (name, root) in &repos {
        // The product searches one repository. Never index the corpus parent.
        build(root, &[Lang::LwcJs, Lang::LwcHtml])?;
        let index = Index::open(root).map_err(|e| format!("open {}: {e}", root.display()))?;
        for (lang, pool) in [(Lang::LwcJs, &mut js), (Lang::LwcHtml, &mut html)] {
            let e = evaluate_index(&index, lang, c.samples, c.seed);
            let label = format!("{} {name}", lang.as_str());
            print_eval(&label, &e, c.seed);
            print_extended(&label, &e, c.seed);
            pool.extend(&e);
        }
    }
    js.print(Lang::LwcJs, c.seed);
    html.print(Lang::LwcHtml, c.seed);
    let (line, passed) = v11_gate_line(&js.start, &html.start, c.seed);
    println!("{line}");
    if passed {
        Ok(())
    } else {
        Err(line)
    }
}

/// D39 gates only pooled start slices with enough queries (D43). Middle/end
/// outcomes and sparse kinds are still printed, but cannot fail a recall gate.
fn v12_gate_line(pool: &LwcPool) -> (String, bool) {
    let mut parts = Vec::new();
    let mut passed = true;
    for kind in [
        UnitKind::FlowElement,
        UnitKind::ValidationRule,
        UnitKind::Field,
    ] {
        let outcomes = pool.by_kind.get(&(kind, Offset::Start));
        let n = outcomes.map_or(0, |o| o.at100.len());
        let at100 = outcomes.map_or(0.0, |o| mean(&o.at100));
        if n >= GATED_KIND_QUERIES && at100 < 0.95 {
            passed = false;
        }
        parts.push(format!(
            "{} pooled start n={n} R@100 {at100:.4} (gate 0.95){}",
            kind_name(kind),
            gated_tag(n)
        ));
    }
    (
        format!(
            "v1.2 recall gates {}: {}",
            if passed { "passed" } else { "failed" },
            parts.join("; ")
        ),
        passed,
    )
}

fn run_v12(c: &Config) -> Result<(), String> {
    let repos = crate::corpus::load()?.with_pilots("v1.2");
    let mut pool = LwcPool::default();
    let mut counts = std::collections::BTreeMap::new();
    let mut seconds = 0.0;
    for repo in repos {
        let root = repo.fetch()?;
        let start = Instant::now();
        let report = build_with_mode(&root, &[Lang::SfMeta], c.sfmeta_mode)?;
        let elapsed = start.elapsed().as_secs_f64();
        seconds += elapsed;
        println!("index sfmeta ({}) {}: {elapsed:.3}s, {} units, {} files reparsed, {} unchanged, {} failed, {} large, {} damaged units skipped",
            mode_name(c.sfmeta_mode), repo.name, report.units, report.files_reparsed,
            report.files_unchanged, report.files_failed, report.files_skipped_large,
            report.units_skipped_damaged);
        let index = Index::open(&root).map_err(|e| format!("open {}: {e}", root.display()))?;
        let evaluation = evaluate_index(&index, Lang::SfMeta, c.samples, c.seed);
        let label = format!("sfmeta ({}) {}", mode_name(c.sfmeta_mode), repo.name);
        print_eval(&label, &evaluation, c.seed);
        print_extended(&label, &evaluation, c.seed);
        for (kind, n) in &evaluation.coverage.per_kind {
            *counts.entry(kind_name(*kind)).or_insert(0u32) += n;
        }
        pool.extend(&evaluation);
    }
    println!(
        "index sfmeta ({}) TOTAL: {seconds:.3}s",
        mode_name(c.sfmeta_mode)
    );
    for (kind, n) in counts {
        println!("recall POOLED sfmeta coverage kind={kind} {n} units");
    }
    pool.print(Lang::SfMeta, c.seed);
    let (line, passed) = v12_gate_line(&pool);
    println!("{line}");
    if passed {
        Ok(())
    } else {
        Err(line)
    }
}

/// D46 gates each v1.3 language present at run time on its pooled start-slice R@100
/// at or above 0.98 (D43 sampling). R@1 is printed but not gated. The floor (Aura
/// bundles and VF pages) must also hold.
fn v13_gate_line(pools: &HashMap<Lang, LwcPool>, floors: &V13Floors, seed: u64) -> (String, bool) {
    let mut parts = Vec::new();
    let mut passed = true;
    for lang in v13_languages() {
        let pool = pools.get(&lang).expect("v1.3 language was evaluated");
        let r = summarize(&pool.start, seed);
        // Gate only when the pool cleared the D43 100-query floor; otherwise report.
        let gate = r.n >= GATED_KIND_QUERIES;
        if gate && r.at100 < 0.98 {
            passed = false;
        }
        parts.push(format!(
            "{} pooled start n={} R@1 {:.4} (reported) R@100 {:.4} (gate 0.98){}",
            lang.as_str(),
            r.n,
            r.at1,
            r.at100,
            if gate {
                ""
            } else {
                " reported (n < 100), not gated"
            }
        ));
    }
    parts.push(format!(
        "floor Aura bundles {} (gate 300)",
        floors.aura_bundles.len()
    ));
    parts.push(format!(
        "floor VF pages {} (gate 300)",
        floors.vf_pages.len()
    ));
    if floors.aura_bundles.len() < 300 {
        passed = false;
    }
    if floors.vf_pages.len() < 300 {
        passed = false;
    }
    (
        format!(
            "v1.3 recall gates {}: {}",
            if passed { "passed" } else { "failed" },
            parts.join("; ")
        ),
        passed,
    )
}

fn run_v13(c: &Config) -> Result<(), String> {
    let langs = v13_languages();
    if langs.is_empty() {
        return Err("no v1.3 language adapter has landed yet".into());
    }
    let repos = crate::corpus::load()?.with_pilots("v1.3");
    let mut pools: HashMap<Lang, LwcPool> = HashMap::new();
    let mut floors = V13Floors::default();
    let mut seconds = 0.0;
    // (repo, lang, kind) -> units, for the per-repository and per-kind tables.
    let mut per_repo: Vec<(String, Vec<(Lang, u32)>)> = Vec::new();
    let mut kind_totals: HashMap<UnitKind, u32> = HashMap::new();
    for repo in &repos {
        let root = repo.fetch()?;
        let start = Instant::now();
        let report = match build_index(&BuildOptions {
            root: root.to_path_buf(),
            langs: langs.clone(),
            profile: ProfileName::Reference,
            compat: false,
            lock_timeout: Duration::from_secs(60),
            jobs: None,
        }) {
            Ok(report) => report,
            // A pilot or pinned repo with no v1.3 markup contributes nothing; skip it.
            Err(BuildError::Empty(_)) => {
                println!("index v1.3 {}: no v1.3 source files, skipped", repo.name);
                continue;
            }
            Err(e) => return Err(format!("index {}: {e}", root.display())),
        };
        let elapsed = start.elapsed().as_secs_f64();
        seconds += elapsed;
        println!("index v1.3 {}: {elapsed:.3}s, {} units, {} files reparsed, {} unchanged, {} failed, {} large, {} damaged units skipped",
            repo.name, report.units, report.files_reparsed,
            report.files_unchanged, report.files_failed, report.files_skipped_large,
            report.units_skipped_damaged);
        let index = Index::open(&root).map_err(|e| format!("open {}: {e}", root.display()))?;
        floors.add_index(&index);
        let mut lang_counts = Vec::new();
        for lang in &langs {
            let evaluation = evaluate_index(&index, *lang, c.samples, c.seed);
            let label = format!("{} {}", lang.as_str(), repo.name);
            print_eval(&label, &evaluation, c.seed);
            print_extended(&label, &evaluation, c.seed);
            for (kind, n) in &evaluation.coverage.per_kind {
                *kind_totals.entry(*kind).or_insert(0) += n;
            }
            lang_counts.push((*lang, evaluation.coverage.units));
            pools.entry(*lang).or_default().extend(&evaluation);
        }
        per_repo.push((repo.name.clone(), lang_counts));
    }
    println!("index v1.3 TOTAL: {seconds:.3}s");
    println!("recall POOLED v1.3 floors: Aura bundles {} (gate 300); VF pages {} (gate 300); VF .component files {} (reported)",
        floors.aura_bundles.len(), floors.vf_pages.len(), floors.vf_components.len());
    for lang in &langs {
        pools.get(lang).unwrap().print(*lang, c.seed);
    }
    let mut kind_totals: Vec<(UnitKind, u32)> = kind_totals.into_iter().collect();
    kind_totals.sort_by_key(|(k, n)| (kind_name(*k).to_string(), *n));
    println!("recall POOLED v1.3 coverage per kind:");
    for (k, n) in &kind_totals {
        println!("  {:<16} {n} units", kind_name(*k));
    }
    println!("recall POOLED v1.3 coverage per repository:");
    for (name, counts) in &per_repo {
        let parts: Vec<String> = counts
            .iter()
            .map(|(lang, n)| format!("{}={}", lang.as_str(), n))
            .collect();
        println!("  {name}: {}", parts.join(", "));
    }
    let (line, passed) = v13_gate_line(&pools, &floors, c.seed);
    println!("{line}");
    if passed {
        Ok(())
    } else {
        Err(line)
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let c = parse_args(args)?;
    if c.v11 {
        return run_v11(&c);
    }
    if c.v12 {
        return run_v12(&c);
    }
    if c.v13 {
        return run_v13(&c);
    }
    if let Some(repo) = &c.repo {
        let lang = c.lang.ok_or("--repo needs --lang <specced language>")?;
        let e = evaluate(repo, lang, c.samples, c.seed)?;
        print_eval(&repo.display().to_string(), &e, c.seed);
        if !is_legacy(lang) {
            print_extended(&repo.display().to_string(), &e, c.seed);
        }
        return Ok(());
    }
    let mut failures = Vec::new();
    if c.java {
        let root = java_corpus(c.java_repos)?;
        let e = evaluate(&root, Lang::Java, c.samples, c.seed)?;
        print_eval(
            &format!("java, first {} repositories", c.java_repos),
            &e,
            c.seed,
        );
        let r = summarize(&e.contiguous, c.seed);
        at_least(
            &mut failures,
            "java contiguous Recall@1",
            r.at1,
            JAVA_MIN_AT1,
        );
        at_least(
            &mut failures,
            "java contiguous Recall@100",
            r.at100,
            JAVA_MIN_AT100,
        );
    }
    if c.apex {
        let mut pooled = Outcomes::default();
        for (name, _, _) in PILOT_REPOS {
            let root = pilot_repo(name)?;
            let e = evaluate(&root, Lang::Apex, c.samples, c.seed)?;
            print_eval(&format!("apex {name}"), &e, c.seed);
            pooled.extend(&e.contiguous);
        }
        let r = summarize(&pooled, c.seed);
        println!("recall apex pooled contiguous {r}");
        at_least(
            &mut failures,
            "apex pooled contiguous Recall@100",
            r.at100,
            APEX_MIN_AT100,
        );
    }
    if failures.is_empty() {
        println!("recall gates passed");
        Ok(())
    } else {
        Err(format!("recall gates failed: {}", failures.join("; ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aroma_core::unit::UnitKind;
    use aroma_lang::index::{OutDiagnostics, SimilarOut};

    const STATS: &str = r#"package sample;

import java.util.HashMap;
import java.util.List;
import java.util.Map;

public class Stats {
    public static Map<String, Integer> countWords(List<String> words) {
        Map<String, Integer> counts = new HashMap<>();
        for (String w : words) {
            String key = w.toLowerCase();
            Integer old = counts.get(key);
            if (old == null) {
                counts.put(key, 1);
            } else {
                counts.put(key, old + 1);
            }
        }
        return counts;
    }

    public static String joinAll(List<String> parts, String sep) {
        StringBuilder sb = new StringBuilder();
        boolean first = true;
        for (String p : parts) {
            if (!first) {
                sb.append(sep);
            }
            sb.append(p);
            first = false;
        }
        String result = sb.toString();
        return result.trim();
    }
}
"#;

    const FILES: &str = r#"package sample;

import java.io.File;

public class Files {
    public static long totalSize(File dir) {
        long total = 0;
        File[] children = dir.listFiles();
        if (children == null) {
            return 0;
        }
        for (File child : children) {
            if (child.isDirectory()) {
                total += totalSize(child);
            } else {
                total += child.length();
            }
        }
        return total;
    }
}
"#;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = target_dir()
            .join("xtask-scratch")
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn similar(path: &str, first: u32, score: f64) -> SimilarOut {
        SimilarOut {
            path: path.to_string(),
            lines: [first, first + 12],
            kind: UnitKind::Method,
            score,
            text: String::new(),
            kept_spans: Vec::new(),
            stale_source: false,
            explain: None,
            structural_rank: 1,
            lexical_rank: None,
            bm25: None,
            matched_terms: Vec::new(),
        }
    }

    fn output(similar: Vec<SimilarOut>) -> QueryOutput {
        QueryOutput {
            schema_version: 1,
            generation: 1,
            profile: ProfileName::Reference,
            similar,
            recommendations: Vec::new(),
            diagnostics: OutDiagnostics::default(),
        }
    }

    #[test]
    fn rng_is_the_pilot_xorshift() {
        let mut rng = Rng::new(119);
        assert_eq!(rng.next_u64(), 128771489207);
        assert_eq!(rng.next_u64(), 8078813625525449100);
        assert_eq!(rng.next_u64(), 15915136011018722799);
        // seed 0 would be a fixed point of xorshift; it is mapped to 1.
        assert_ne!(Rng::new(0).next_u64(), 0);
    }

    #[test]
    fn shuffle_is_a_seeded_permutation() {
        let mut a: Vec<u32> = (0..50).collect();
        let mut b = a.clone();
        Rng::new(119).shuffle(&mut a);
        Rng::new(119).shuffle(&mut b);
        assert_eq!(a, b);
        assert_ne!(a, (0..50).collect::<Vec<_>>());
        let mut sorted = a.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..50).collect::<Vec<_>>());
    }

    #[test]
    fn bootstrap_of_constant_outcomes_has_no_spread() {
        let mut rng = Rng::new(120);
        assert_eq!(bootstrap(&[true; 40], 30, &mut rng), (1.0, 0.0));
        assert_eq!(bootstrap(&[false; 40], 30, &mut rng), (0.0, 0.0));
        assert_eq!(bootstrap(&[], 30, &mut rng), (0.0, 0.0));
        let mixed: Vec<bool> = (0..100).map(|i| i % 4 != 0).collect();
        let (mean, half) = bootstrap(&mixed, 30, &mut rng);
        assert!((mean - 0.75).abs() < 0.05, "{mean}");
        assert!(half > 0.0 && half < 0.05, "{half}");
    }

    #[test]
    fn repository_csv_rows_are_parsed_in_order() {
        let csv = "repository,url,commit_sha,commit_date\n\
                   bumptech/glide,https://github.com/bumptech/glide/tree/b7c2b1360c777794c50351a10c30c08a3390b753,b7c2b1360c777794c50351a10c30c08a3390b753,2018-07-03T21:37:39Z\n\
                   broken-row\n\
                   airbnb/lottie-android,https://github.com/airbnb/lottie-android/tree/9ce2cc88edd35925d0e3a9be21970fabb2047231,9ce2cc88edd35925d0e3a9be21970fabb2047231,2018-07-20T21:15:49Z\n";
        let rows = parse_repo_csv(csv);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "bumptech/glide");
        assert_eq!(rows[0].sha, "b7c2b1360c777794c50351a10c30c08a3390b753");
        assert_eq!(rows[1].name, "airbnb/lottie-android");
    }

    #[test]
    fn body_lines_skip_comment_only_lines() {
        // A query of comment lines carries no code (recall miss, 2026-11-20).
        let src = "class C {\n  void m(int x) {\n    // explain\n    /* block\n     * more\n     */\n    int y = x; // keep: code first\n    /** doc */\n    y++;\n  }\n}\n";
        let spec = spec_for(Lang::Java).unwrap();
        let tree = parse(spec, src, &Limits::DEFAULT).unwrap();
        let units = extract_units(&tree, src, spec, &Limits::DEFAULT);
        assert_eq!(
            body_lines(src, Lang::Java, &units[0].bytes).unwrap(),
            ["    int y = x; // keep: code first", "    y++;"]
        );
    }

    #[test]
    fn body_lines_are_the_non_blank_lines_inside_the_braces() {
        let spec = spec_for(Lang::Java).unwrap();
        let tree = parse(spec, STATS, &Limits::DEFAULT).unwrap();
        let units = extract_units(&tree, STATS, spec, &Limits::DEFAULT);
        assert_eq!(units[0].lines, 8..21);
        let lines = body_lines(STATS, Lang::Java, &units[0].bytes).unwrap();
        assert_eq!(
            lines,
            [
                "        Map<String, Integer> counts = new HashMap<>();",
                "        for (String w : words) {",
                "            String key = w.toLowerCase();",
                "            Integer old = counts.get(key);",
                "            if (old == null) {",
                "                counts.put(key, 1);",
                "            } else {",
                "                counts.put(key, old + 1);",
                "            }",
                "        }",
                "        return counts;",
            ]
        );
        assert_eq!(body_lines(STATS, Lang::Java, &(0..5)), None);
        let sample = Sample {
            body_hash: 7,
            kind: UnitKind::Method,
            lines,
        };
        assert_eq!(
            contiguous_query(&sample),
            "        Map<String, Integer> counts = new HashMap<>();\n        for (String w : words) {\n            String key = w.toLowerCase();\n            Integer old = counts.get(key);\n            if (old == null) {\n"
        );
        let mut rng = Rng::new(119);
        let random = random_query(&sample, &mut rng);
        let picked: Vec<&str> = random.lines().collect();
        assert_eq!(picked.len(), 5);
        let positions: Vec<usize> = picked
            .iter()
            .map(|l| sample.lines.iter().position(|s| s == l).unwrap())
            .collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]), "{positions:?}");
        assert_eq!(random, random_query(&sample, &mut Rng::new(119)));
    }

    #[test]
    fn source_rank_uses_displayed_position_without_tie_promotion() {
        let mut hash_of = HashMap::new();
        hash_of.insert(("A.java".to_string(), 1), 10);
        hash_of.insert(("B.java".to_string(), 1), 20);
        hash_of.insert(("C.java".to_string(), 5), 20);
        // Source at index 0.
        let first = output(vec![similar("B.java", 1, 0.5), similar("A.java", 1, 0.9)]);
        assert_eq!(source_rank(&first, 20, &hash_of), Some(0));
        // Source at index 1, lower score: rank 1.
        let second = output(vec![similar("A.java", 1, 0.9), similar("C.java", 5, 0.8)]);
        assert_eq!(source_rank(&second, 20, &hash_of), Some(1));
        assert_eq!(source_rank(&second, 30, &hash_of), None);
        assert_eq!(source_rank(&output(Vec::new()), 20, &hash_of), None);
    }

    #[test]
    fn source_rank_does_not_promote_on_score_tie_or_win() {
        let mut hash_of = HashMap::new();
        hash_of.insert(("A.java".to_string(), 1), 10);
        hash_of.insert(("B.java".to_string(), 1), 20);
        hash_of.insert(("C.java".to_string(), 5), 30);
        // Source sits at index 2 with score equal to similar[0].score -> rank 2.
        let tie = output(vec![
            similar("A.java", 1, 1.0),
            similar("B.java", 1, 0.7),
            similar("C.java", 5, 1.0),
        ]);
        assert_eq!(source_rank(&tie, 30, &hash_of), Some(2));
        // Source sits at index 2 with score greater than similar[0].score -> rank 2.
        let win = output(vec![
            similar("A.java", 1, 0.4),
            similar("B.java", 1, 0.6),
            similar("C.java", 5, 0.9),
        ]);
        assert_eq!(source_rank(&win, 30, &hash_of), Some(2));
    }

    #[test]
    fn sampling_policy_table_matches_d43() {
        for code in [Lang::Java, Lang::Apex] {
            let p = policy_for(code);
            assert_eq!(p.min_unit_lines, 12);
            assert_eq!(p.min_interior_lines, 5);
            assert_eq!(p.offsets, &[Offset::Start][..]);
        }
        for markup in [
            Lang::LwcJs,
            Lang::LwcHtml,
            Lang::SfMeta,
            Lang::Aura,
            Lang::AuraJs,
            Lang::Vf,
        ] {
            let p = policy_for(markup);
            assert_eq!(p.min_unit_lines, 4);
            assert_eq!(p.min_interior_lines, 3);
            assert_eq!(p.offsets, &[Offset::Start, Offset::Middle, Offset::End][..]);
        }
    }

    #[test]
    fn non_contiguous_queries_accept_short_units() {
        let sample = Sample {
            body_hash: 1,
            kind: UnitKind::Template,
            lines: vec!["a".into(), "b".into(), "c".into()],
        };
        assert_eq!(random_query(&sample, &mut Rng::new(119)), "a\nb\nc\n");
    }

    #[test]
    fn recall_default_and_v11_argument_paths() {
        let c = parse_args(&[]).unwrap();
        assert!(c.java && c.apex && !c.v11);
        assert_eq!((c.samples, c.seed, c.java_repos), (1000, 119, 20));
        assert!(c.repo.is_none() && c.lang.is_none());
        let c = parse_args(&["--only".into(), "apex".into()]).unwrap();
        assert!(!c.java && c.apex && !c.v11);
        let c = parse_args(&["--suite".into(), "v1.1".into()]).unwrap();
        assert!(c.v11);
        assert!(parse_args(&["--suite".into()]).is_err());
        assert!(parse_args(&["--suite".into(), "unknown".into()]).is_err());
        assert!(parse_args(&[
            "--suite".into(),
            "v1.1".into(),
            "--only".into(),
            "apex".into()
        ])
        .is_err());
    }

    #[test]
    fn v13_selects_only_specced_languages_and_rejects_conflicts() {
        let c = parse_args(&["--suite".into(), "v1.3".into()]).unwrap();
        assert!(c.v13 && !c.v11 && !c.v12);
        assert!(v13_languages().contains(&Lang::Vf));
        assert!(v13_languages().contains(&Lang::Aura));
        // aurajs joins automatically because its adapter is specced.
        assert!(v13_languages().contains(&Lang::AuraJs));
        for args in [
            vec!["--suite", "v1.3", "--suite", "v1.1"],
            vec!["--suite", "v1.3", "--only", "apex"],
            vec!["--suite", "v1.3", "--sfmeta-mode", "lexical"],
            vec!["--suite", "v1.3", "--repo", "x", "--lang", "vf"],
            vec!["--suite", "v1.1", "--suite", "v1.3"],
        ] {
            assert!(parse_args(&args.into_iter().map(String::from).collect::<Vec<_>>()).is_err());
        }
    }

    #[test]
    fn aura_bundle_extracts_the_folder_after_aura() {
        assert_eq!(
            aura_bundle("force-app/main/default/aura/myBundle/myBundle.cmp"),
            Some("myBundle".into())
        );
        assert_eq!(aura_bundle("aura/app/app.app"), Some("app".into()));
        assert_eq!(
            aura_bundle("src/main/AURA/MyComp/MyComp.cmp"),
            Some("MyComp".into())
        );
        assert_eq!(aura_bundle("pages/Account.page"), None);
        assert_eq!(aura_bundle("aura/myBundle"), None);
        assert_eq!(aura_bundle(""), None);
    }

    #[test]
    fn v13_gate_gates_languages_and_floor_with_4_decimal_report() {
        fn outcomes(at100: usize) -> Outcomes {
            Outcomes {
                at1: vec![false; 100],
                at100: (0..100).map(|n| n < at100).collect(),
            }
        }
        let mut pools = HashMap::new();
        for lang in v13_languages() {
            pools.insert(lang, LwcPool::default());
            pools.get_mut(&lang).unwrap().start = outcomes(98);
        }
        let floors = V13Floors {
            aura_bundles: (0..300).map(|i| format!("b{i}")).collect(),
            vf_pages: (0..300).map(|i| format!("p{i}")).collect(),
            vf_components: (0..5).map(|i| format!("c{i}")).collect(),
        };
        let (line, passed) = v13_gate_line(&pools, &floors, 119);
        assert!(passed, "{line}");
        assert!(line.starts_with("v1.3 recall gates passed:"));
        assert!(line.contains("R@100 0.9800 (gate 0.98)"));
        assert!(line.contains("R@1 0.0000 (reported)"));
        assert!(line.contains("floor Aura bundles 300 (gate 300)"));
        assert!(line.contains("floor VF pages 300 (gate 300)"));
        // A language below 0.98 fails.
        pools.get_mut(&Lang::Vf).unwrap().start.at100[97] = false;
        let (line, passed) = v13_gate_line(&pools, &floors, 119);
        assert!(!passed);
        assert!(line.starts_with("v1.3 recall gates failed:"));
        assert!(line.contains("vf pooled start n=100 R@1 0.0000 (reported) R@100 0.9700"));
        // A floor shortfall fails the suite.
        pools.get_mut(&Lang::Vf).unwrap().start.at100[97] = true;
        let short = V13Floors {
            aura_bundles: (0..299).map(|i| format!("b{i}")).collect(),
            vf_pages: (0..300).map(|i| format!("p{i}")).collect(),
            vf_components: Default::default(),
        };
        let (line, passed) = v13_gate_line(&pools, &short, 119);
        assert!(!passed);
        assert!(line.contains("floor Aura bundles 299 (gate 300)"));
        // Sparse pool (< 100 queries) is reported, not gated.
        let mut sparse = HashMap::new();
        for lang in v13_languages() {
            let mut pool = LwcPool::default();
            pool.start.at100 = vec![false; 40];
            sparse.insert(lang, pool);
        }
        let (line, passed) = v13_gate_line(&sparse, &floors, 119);
        assert!(passed, "{line}");
        assert!(line.contains("reported (n < 100), not gated"));
    }

    #[test]
    fn v13_floors_count_bundles_and_pages_from_the_index() {
        let root = scratch("v13-floors");
        std::fs::create_dir_all(root.join("force-app/main/default/aura/a")).unwrap();
        std::fs::create_dir_all(root.join("force-app/main/default/aura/c")).unwrap();
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("components")).unwrap();
        std::fs::write(
            root.join("force-app/main/default/aura/a/b.cmp"),
            "<aura:component>\n<aura:attribute name=\"x\" type=\"String\"/>\n</aura:component>\n",
        )
        .unwrap();
        std::fs::write(
            root.join("force-app/main/default/aura/c/d.app"),
            "<aura:application>\n<aura:attribute name=\"y\" type=\"String\"/>\n</aura:application>\n",
        )
        .unwrap();
        std::fs::write(
            root.join("pages/Home.page"),
            "<apex:page>\n<apex:outputText value=\"hi\"/>\n</apex:page>\n",
        )
        .unwrap();
        std::fs::write(
            root.join("components/Header.component"),
            "<apex:component>\n<apex:outputText value=\"h\"/>\n</apex:component>\n",
        )
        .unwrap();
        build(&root, &[Lang::Aura, Lang::Vf]).unwrap();
        let index = Index::open(&root).unwrap();
        let mut floors = V13Floors::default();
        floors.add_index(&index);
        assert_eq!(floors.aura_bundles.len(), 2);
        assert_eq!(floors.vf_pages.len(), 1);
        assert_eq!(floors.vf_components.len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn v12_selects_metadata_and_accepts_both_modes_only_for_that_suite() {
        for mode in ["lexical", "structural"] {
            let args = ["--suite", "v1.2", "--sfmeta-mode", mode].map(String::from);
            let c = parse_args(&args).unwrap();
            assert!(c.v12 && !c.v11);
            assert_eq!(mode_name(c.sfmeta_mode), mode);
        }
        for args in [
            vec!["--suite", "v1.2", "--only", "apex"],
            vec!["--suite", "v1.1", "--sfmeta-mode", "structural"],
            vec!["--sfmeta-mode", "lexical"],
            vec!["--suite", "v1.2", "--sfmeta-mode", "unknown"],
            vec!["--suite", "v1.2", "--sfmeta-mode"],
            vec!["--suite", "v1.1", "--suite", "v1.2"],
        ] {
            assert!(parse_args(&args.into_iter().map(String::from).collect::<Vec<_>>()).is_err());
        }
        let c = parse_args(&["--suite".into(), "v1.2".into()]).unwrap();
        assert!(c.v12 && !c.v11);
        assert_eq!(c.sfmeta_mode, SfmetaMode::Structural);
    }

    #[test]
    fn v12_gates_each_kind_at_95_percent_and_only_gates_100_start_queries() {
        let mut pool = LwcPool::default();
        for kind in [
            UnitKind::FlowElement,
            UnitKind::ValidationRule,
            UnitKind::Field,
        ] {
            pool.by_kind.insert(
                (kind, Offset::Start),
                Outcomes {
                    at1: vec![false; 100],
                    at100: (0..100).map(|i| i < 95).collect(),
                },
            );
            pool.by_kind.insert(
                (kind, Offset::Middle),
                Outcomes {
                    at1: vec![false; 100],
                    at100: vec![false; 100],
                },
            );
        }
        let (line, passed) = v12_gate_line(&pool);
        assert!(passed, "{line}");
        assert!(line.starts_with("v1.2 recall gates passed:"));
        assert_eq!(line.matches("R@100 0.9500 (gate 0.95)").count(), 3);
        for kind in [
            UnitKind::FlowElement,
            UnitKind::ValidationRule,
            UnitKind::Field,
        ] {
            pool.by_kind.get_mut(&(kind, Offset::Start)).unwrap().at100[94] = false;
            let (line, passed) = v12_gate_line(&pool);
            assert!(!passed, "{line}");
            assert!(line.starts_with("v1.2 recall gates failed:"));
            assert!(line.contains(&format!(
                "{} pooled start n=100 R@100 0.9400",
                kind_name(kind)
            )));
            pool.by_kind.get_mut(&(kind, Offset::Start)).unwrap().at100 = vec![false; 99];
            let (line, passed) = v12_gate_line(&pool);
            assert!(passed, "{line}");
            assert!(line.contains("n=99 R@100 0.0000 (gate 0.95) reported (n < 100), not gated"));
            pool.by_kind.remove(&(kind, Offset::Start));
            assert!(v12_gate_line(&pool).1);
        }
    }

    #[test]
    fn metadata_sample_budget_is_per_kind_and_short_units_are_excluded() {
        let root = scratch("metadata-samples");
        std::fs::create_dir_all(&root).unwrap();
        for i in 0..4 {
            std::fs::write(root.join(format!("F{i}.field-meta.xml")), format!(
                "<CustomField>\n<fullName>F{i}</fullName>\n<label>Field</label>\n<type>Text</type>\n</CustomField>\n"
            )).unwrap();
        }
        std::fs::write(root.join("R.validationRule-meta.xml"),
            "<ValidationRule>\n<fullName>R</fullName>\n<active>true</active>\n<errorMessage>Required</errorMessage>\n</ValidationRule>\n").unwrap();
        std::fs::write(
            root.join("Short.md-meta.xml"),
            "<CustomMetadata><label>Short</label></CustomMetadata>\n",
        )
        .unwrap();
        build(&root, &[Lang::SfMeta]).unwrap();
        let index = Index::open(&root).unwrap();
        let picked = sample_units(&index, Lang::SfMeta, 1, &mut Rng::new(119));
        assert_eq!(picked.len(), 2);
        assert_eq!(
            picked.iter().filter(|s| s.kind == UnitKind::Field).count(),
            1
        );
        assert_eq!(
            picked
                .iter()
                .filter(|s| s.kind == UnitKind::ValidationRule)
                .count(),
            1
        );
        assert!(picked.iter().all(|s| s.lines.len() == 3));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cached_metadata_interiors_preserve_individual_unit_slices() {
        let src = "<CustomObject>\n<fields>\n<fullName>A</fullName>\n<label>Alpha</label>\n<type>Text</type>\n</fields>\n<fields>\n<fullName>B</fullName>\n<label>Beta</label>\n<type>Number</type>\n</fields>\n</CustomObject>\n";
        let cached = metadata_body_lines(src);
        assert_eq!(cached.len(), 2);
        let mut entries: Vec<_> = cached.into_iter().collect();
        entries.sort_by_key(|(bytes, _)| *bytes);
        assert_eq!(
            entries[0].1,
            [
                "<fullName>A</fullName>",
                "<label>Alpha</label>",
                "<type>Text</type>"
            ]
        );
        assert_eq!(
            entries[1].1,
            [
                "<fullName>B</fullName>",
                "<label>Beta</label>",
                "<type>Number</type>"
            ]
        );
        for ((start, end), lines) in entries {
            assert_eq!(Some(lines), body_lines(src, Lang::SfMeta, &(start..end)));
        }
    }

    #[test]
    fn v11_gate_thresholds_are_inclusive_and_empty_is_failure() {
        fn outcomes(at1: usize, at100: usize) -> Outcomes {
            Outcomes {
                at1: (0..100).map(|n| n < at1).collect(),
                at100: (0..100).map(|n| n < at100).collect(),
            }
        }
        let (line, passed) = v11_gate_line(&outcomes(95, 99), &outcomes(0, 98), 119);
        assert!(passed);
        assert!(line.starts_with("v1.1 recall gates passed:"));
        for (js, html, expected) in [
            (
                outcomes(94, 99),
                outcomes(0, 98),
                "lwcjs pooled start R@1 0.9400 < 0.95",
            ),
            (
                outcomes(95, 98),
                outcomes(0, 98),
                "lwcjs pooled start R@100 0.9800 < 0.99",
            ),
            (
                outcomes(95, 99),
                outcomes(0, 97),
                "lwchtml pooled start R@100 0.9700 < 0.98",
            ),
        ] {
            let (line, passed) = v11_gate_line(&js, &html, 119);
            assert!(!passed);
            assert!(line.starts_with("v1.1 recall gates failed:"));
            assert!(line.contains(expected), "{line}");
        }
        assert!(!v11_gate_line(&Outcomes::default(), &Outcomes::default(), 119).1);
    }

    #[test]
    fn pooling_weights_queries_and_keeps_kind_offsets_separate() {
        let mut pool = LwcPool::default();
        for ranks in [vec![Some(0)], vec![None, None, None]] {
            let mut start = Outcomes::default();
            for rank in ranks {
                start.push(rank);
            }
            pool.extend(&Evaluation {
                units: 1,
                contiguous: start.clone(),
                random: Outcomes::default(),
                by_kind: vec![
                    (UnitKind::Method, Offset::Start, start),
                    (
                        UnitKind::Method,
                        Offset::End,
                        Outcomes {
                            at1: vec![true],
                            at100: vec![true],
                        },
                    ),
                ],
                coverage: Coverage {
                    units: 1,
                    files: 1,
                    per_kind: vec![],
                },
            });
        }
        assert_eq!(summarize(&pool.start, 119).at1, 0.25);
        assert_eq!(
            pool.by_kind[&(UnitKind::Method, Offset::Start)].at1.len(),
            4
        );
        assert_eq!(
            pool.by_kind[&(UnitKind::Method, Offset::End)].at1,
            vec![true; 2]
        );
    }

    #[test]
    fn middle_and_end_slices_are_centred_and_trailing() {
        let nine: Vec<String> = (1..=9).map(|i| i.to_string()).collect();
        let s9 = Sample {
            body_hash: 1,
            kind: UnitKind::Method,
            lines: nine,
        };
        assert_eq!(query_len(s9.lines.len()), 5);
        assert_eq!(start_query(&s9, 5), "1\n2\n3\n4\n5\n");
        assert_eq!(middle_query(&s9, 5), "3\n4\n5\n6\n7\n");
        assert_eq!(end_query(&s9, 5), "5\n6\n7\n8\n9\n");

        let three: Vec<String> = (1..=3).map(|i| i.to_string()).collect();
        let s3 = Sample {
            body_hash: 2,
            kind: UnitKind::Template,
            lines: three,
        };
        let n = query_len(3);
        assert_eq!(n, 3);
        assert_eq!(middle_query(&s3, n), "1\n2\n3\n");
        assert_eq!(end_query(&s3, n), "1\n2\n3\n");
    }

    #[test]
    fn kinds_below_100_queries_are_not_gated() {
        assert_eq!(gated_tag(99), " reported (n < 100), not gated");
        assert_eq!(gated_tag(100), "");
        assert_eq!(gated_tag(1000), "");
    }

    #[test]
    fn java_and_apex_sampling_is_unchanged() {
        // Java/Apex keep the historical policy: same units, same queries, same seed as the
        // pre-D43 functions. The `Sample` list below is the output the old `sample_units`
        // produced on this small repository at DEFAULT_SEED (captured before the change):
        // same body hashes, same interior lines, same order. `contiguous_query` and
        // `random_query` still start from the same 5-line slice.
        let root = scratch("recall-sampling");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/Stats.java"), STATS).unwrap();
        std::fs::write(root.join("src/Files.java"), FILES).unwrap();
        let _ = build(&root, &[Lang::Java]);
        let index = Index::open(&root).unwrap();
        let mut rng = Rng::new(DEFAULT_SEED);
        let picked = sample_units(&index, Lang::Java, 10, &mut rng);
        let hashes: Vec<u64> = picked.iter().map(|s| s.body_hash).collect();
        assert_eq!(
            hashes,
            [
                12236975177678856691,
                2531585278182224711,
                15007083082163781276
            ]
        );
        for s in &picked {
            assert!(s.lines.len() >= 5);
            assert_eq!(
                contiguous_query(s),
                start_query(s, query_len(s.lines.len()))
            );
        }
        // Apex samples the same way (same policy object).
        assert_eq!(
            policy_for(Lang::Java).min_unit_lines,
            policy_for(Lang::Apex).min_unit_lines
        );
        assert_eq!(
            policy_for(Lang::Java).offsets,
            policy_for(Lang::Apex).offsets
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn every_source_is_recalled_first_in_a_small_repository() {
        let root = scratch("recall");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/Stats.java"), STATS).unwrap();
        std::fs::write(root.join("src/Files.java"), FILES).unwrap();
        let e = evaluate(&root, Lang::Java, 10, DEFAULT_SEED).unwrap();
        assert_eq!(e.units, 3);
        assert_eq!(e.contiguous.at1, vec![true; 3]);
        assert_eq!(e.contiguous.at100, vec![true; 3]);
        assert_eq!(e.random.at100.len(), 3);
        let r = summarize(&e.contiguous, DEFAULT_SEED);
        assert_eq!((r.n, r.at1, r.at100), (3, 1.0, 1.0));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
