//! `idiom`: D17 Salesforce idiom suite.
//!
//! ```text
//! cargo run --release -p xtask -- idiom [--tasks DIR] [--only ID]
//! cargo run --release -p xtask -- idiom --suite lwc
//! cargo run --release -p xtask -- idiom --suite sfmeta --sfmeta-mode lexical|structural
//! cargo run --release -p xtask -- idiom --suite lwc --check
//! ```
//!
//! Each `xtask/tasks/apex/*.toml` task (T19) names a repository at a pinned commit, the
//! source method (`path`, `lines = [first, last]`), a partial `query` and the `required`
//! fragments a useful answer contains. Per task: the source unit and its exact clones
//! (same body_hash) are blanked out of a copy of the checkout, the copy is indexed, and
//! the query runs with top-5 similar results and `experimental_recommend`.
//!
//! * similar hit@5 (D29): one of the top-5 similar methods whose FULL source lines
//!   contain every required fragment. **Gate: >= 60% of all tasks (D29).** The stricter
//!   "pruned text" hit@5 (`SimilarOut::text`) is reported, not gated.
//! * recommendation hit: a recommendation's text contains every required fragment the
//!   query lacks (D17: scored on recommendation minus query). Win = recommendation hit
//!   without similar hit, loss = the reverse, tie otherwise; win-rate is reported, not gated (D27, D29).
//!
//! Fragments match ignoring whitespace and case (Apex is case-insensitive).
//! The LWC suite uses the same repository-only holdout protocol. Component tasks
//! hold out all parts and score full matched unit lines within one top-5 component.
//! sfmeta instead pools all pinned v1.2 repositories and pilots, holds out source
//! units and exact clones across that pool, and reports the D42 relative comparison
//! without applying the Apex/LWC absolute hit-rate gate. Metadata defaults to structural.

use crate::recall::{
    build, build_with_mode, clone_at, corpus_dir, mode_name, parse_sfmeta_mode, target_dir,
};
use aroma_core::lang::Lang;
use aroma_core::unit::UnitStore;
use aroma_lang::build::SfmetaMode;
use aroma_lang::index::{
    ComponentOutput, ComponentPart, ComponentQuery, Index, QueryOptions, SimilarOut,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

const HIT_AT_5_GATE: f64 = 0.60;
const TOP: usize = 5;
const USAGE: &str =
    "usage: cargo run --release -p xtask -- idiom [--suite lwc|sfmeta] [--sfmeta-mode structural|lexical (sfmeta only)] [--tasks DIR] [--only ID] [--check]";

fn default_lang() -> String {
    "apex".to_string()
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct Task {
    /// Defaults to the file stem.
    #[serde(default)]
    pub(crate) id: String,
    pub(crate) repo: String,
    pub(crate) commit: String,
    #[serde(default, alias = "file", alias = "source_file")]
    pub(crate) path: String,
    /// Source method, 1-based inclusive.
    #[serde(default, alias = "source_lines", alias = "method_lines")]
    pub(crate) lines: [u32; 2],
    #[serde(default)]
    pub(crate) query: String,
    #[serde(
        alias = "required_elements",
        alias = "must_contain",
        alias = "expected"
    )]
    pub(crate) required: Vec<String>,
    #[serde(default = "default_lang")]
    pub(crate) lang: String,
    #[serde(default)]
    pub(crate) include_tests: bool,
    #[serde(default)]
    pub(crate) parts: Vec<TaskPart>,
    #[serde(default)]
    pub(crate) answer_witness: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct TaskPart {
    lang: String,
    #[serde(alias = "source_file")]
    path: String,
    #[serde(alias = "source_lines")]
    lines: [u32; 2],
    query: String,
}

impl Task {
    fn source_parts(&self) -> Vec<TaskPart> {
        if self.lang == "component" {
            self.parts.clone()
        } else {
            vec![TaskPart {
                lang: self.lang.clone(),
                path: self.path.clone(),
                lines: self.lines,
                query: self.query.clone(),
            }]
        }
    }

    fn langs(&self) -> Vec<Lang> {
        self.source_parts()
            .iter()
            .filter_map(|p| Lang::parse(&p.lang))
            .collect()
    }
}

pub(crate) fn parse_task(text: &str, stem: &str) -> Result<Task, String> {
    let mut t: Task = toml::from_str(text).map_err(|e| e.to_string())?;
    if t.id.is_empty() {
        t.id = stem.to_string();
    }
    if t.required.is_empty() {
        return Err("required must list at least one fragment".into());
    }
    if t.lang == "component" {
        if t.parts.len() < 2
            || t.parts
                .iter()
                .any(|p| !matches!(p.lang.as_str(), "lwcjs" | "lwchtml"))
        {
            return Err("component tasks need at least two LWC parts".into());
        }
    } else if !t.parts.is_empty() {
        return Err("only component tasks may contain parts".into());
    }
    for part in t.source_parts() {
        if part.lines[0] == 0 || part.lines[0] > part.lines[1] {
            return Err(format!(
                "lines {:?} must be a 1-based [first, last] range",
                part.lines
            ));
        }
        if part.path.is_empty() || part.query.trim().is_empty() {
            return Err("source file and query must not be empty".into());
        }
        if !Lang::parse(&part.lang).is_some_and(|lang| Lang::SPECCED.contains(&lang)) {
            return Err(format!("unknown lang {:?} (or not specced)", part.lang));
        }
    }
    Ok(t)
}

pub(crate) fn load_tasks(dir: &Path) -> Result<Vec<Task>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|p| {
            let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            parse_task(&text, &stem).map_err(|e| format!("{}: {e}", p.display()))
        })
        .collect()
}

/// Lower-cased, whitespace removed.
pub(crate) fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) fn contains_all(text: &str, required: &[String]) -> bool {
    let t = squash(text);
    required.iter().all(|r| t.contains(&squash(r)))
}

/// Lines `first..=last` (1-based) replaced by empty lines with their original endings.
pub(crate) fn blank_lines(src: &str, first: u32, last: u32) -> String {
    let mut out = String::with_capacity(src.len());
    for (i, line) in src.split_inclusive('\n').enumerate() {
        if (first..=last).contains(&(i as u32 + 1)) {
            if line.ends_with("\r\n") {
                out.push_str("\r\n");
            } else if line.ends_with('\n') {
                out.push('\n');
            }
        } else {
            out.push_str(line);
        }
    }
    out
}

/// Recursive copy without `.git` and `.aroma` (symlinks skipped).
pub(crate) fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("create {}: {e}", to.display()))?;
    let entries = std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        if name == ".git" || name == ".aroma" {
            continue;
        }
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        let target = to.join(&name);
        if ty.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if ty.is_file() {
            std::fs::copy(entry.path(), &target)
                .map_err(|e| format!("copy {}: {e}", entry.path().display()))?;
        }
    }
    Ok(())
}

/// `owner__name@sha12` from a GitHub URL.
pub(crate) fn repo_slug(url: &str, commit: &str) -> String {
    let trimmed = url.trim_end_matches('/').trim_end_matches(".git");
    let mut parts: Vec<&str> = trimmed.rsplit('/').take(2).collect();
    parts.reverse();
    format!("{}@{}", parts.join("__"), &commit[..commit.len().min(12)])
}

/// `(path, first, last)` of the task's source unit (the smallest unit of `path`
/// covering `lines`) and of every unit with the same body hash, sorted and deduplicated
/// across all component parts.
pub(crate) fn held_out(index: &Index, task: &Task) -> Result<Vec<(String, u32, u32)>, String> {
    let mut held = Vec::new();
    for part in task.source_parts() {
        held.extend(held_out_part(index, &part)?);
    }
    held.sort();
    held.dedup();
    Ok(held)
}

/// Units of `part`'s language and file to hold out: the smallest unit covering the
/// declared lines; when none covers them (a component part may span several members),
/// every unit overlapping them. Exact clones (same body hash) of those units are held
/// out too, so the source can never answer its own task.
fn held_out_part(index: &Index, part: &TaskPart) -> Result<Vec<(String, u32, u32)>, String> {
    let path = Path::new(&part.path);
    let lang = Lang::parse(&part.lang).ok_or_else(|| format!("unknown lang {}", part.lang))?;
    let [first, last] = part.lines;
    let in_file: Vec<_> = (0..index.n_units())
        .map(|id| index.record(id))
        .filter(|r| r.lang == lang && index.file_path(r.file) == path)
        .collect();
    let covering = in_file
        .iter()
        .filter(|r| r.lines.start <= first && r.lines.end > last)
        .min_by_key(|r| r.lines.end - r.lines.start);
    let sources: Vec<_> = match covering {
        Some(r) => vec![r],
        None => in_file
            .iter()
            .filter(|r| r.lines.start <= last && r.lines.end > first)
            .collect(),
    };
    if sources.is_empty() {
        return Err(format!(
            "no unit of {} covers or overlaps lines {first}-{last}",
            part.path
        ));
    }
    let hashes: std::collections::HashSet<u64> = sources.iter().map(|r| r.body_hash).collect();
    Ok((0..index.n_units())
        .map(|id| index.record(id))
        .filter(|r| r.lang == lang && hashes.contains(&r.body_hash))
        .map(|r| {
            (
                index.file_path(r.file).to_string_lossy().into_owned(),
                r.lines.start,
                r.lines.end - 1,
            )
        })
        .collect())
}

pub(crate) struct Outcome {
    pub(crate) held_out: usize,
    /// 0-based rank of the first top-5 similar method containing every fragment.
    pub(crate) similar_rank: Option<usize>,
    pub(crate) pruned_hit: bool,
    pub(crate) has_recommendation: bool,
    pub(crate) rec_hit: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Win,
    Tie,
    Loss,
}

pub(crate) fn verdict(o: &Outcome) -> Option<Verdict> {
    if !o.has_recommendation {
        return None;
    }
    Some(match (o.rec_hit, o.similar_rank.is_some()) {
        (true, false) => Verdict::Win,
        (false, true) => Verdict::Loss,
        _ => Verdict::Tie,
    })
}

/// Full source lines of a similar unit in the work copy.
fn unit_text(root: &Path, s: &SimilarOut) -> Option<String> {
    let bytes = std::fs::read(root.join(&s.path)).ok()?;
    let src = String::from_utf8_lossy(&bytes);
    let first = s.lines[0].max(1) as usize;
    let count = (s.lines[1] + 1).saturating_sub(s.lines[0]) as usize;
    Some(
        src.lines()
            .skip(first - 1)
            .take(count)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Fragments may span matched units of one component, never separate components.
/// Use full source lines, not the pruned `SimilarOut::text` (D29).
fn component_rank(root: &Path, out: &ComponentOutput, required: &[String]) -> Option<usize> {
    out.components.iter().take(TOP).position(|component| {
        let texts: Vec<_> = component
            .parts
            .iter()
            .flatten()
            .filter_map(|part| unit_text(root, part))
            .map(|text| squash(&text))
            .collect();
        required
            .iter()
            .all(|fragment| texts.iter().any(|text| text.contains(&squash(fragment))))
    })
}

type Location = (String, u32, u32);
type UnitEvidence = Vec<(Location, String)>;

/// Shared by search and validation: physically blank source units and exact-body
/// clones, then reindex, including any enclosing units changed by the blanking.
fn prepare_holdout(
    clone: &Path,
    index: &Index,
    task: &Task,
    work: &Path,
) -> Result<(Index, Vec<Location>), String> {
    let langs = task.langs();
    let held = held_out(index, task)?;
    if work.exists() {
        std::fs::remove_dir_all(work).map_err(|e| format!("{}: {e}", work.display()))?;
    }
    copy_tree(clone, work)?;
    let mut by_file: BTreeMap<&str, Vec<(u32, u32)>> = BTreeMap::new();
    for (path, first, last) in &held {
        by_file
            .entry(path.as_str())
            .or_default()
            .push((*first, *last));
    }
    for (path, ranges) in by_file {
        let file = work.join(path);
        let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut src = String::from_utf8_lossy(&bytes).into_owned();
        for (first, last) in ranges {
            src = blank_lines(&src, first, last);
        }
        std::fs::write(&file, src).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    build(work, &langs)?;
    let idx = Index::open(work).map_err(|e| format!("open {}: {e}", work.display()))?;
    Ok((idx, held))
}

struct Answer {
    path: String,
    units: Vec<Location>,
}

struct CheckOutcome {
    held_out: usize,
    answers: Vec<Answer>,
    witness_errors: Vec<String>,
    query_errors: Vec<String>,
}

impl CheckOutcome {
    fn passes(&self) -> bool {
        self.answers.len() >= 2 && self.witness_errors.is_empty() && self.query_errors.is_empty()
    }
}

/// Enumerate possible answers without querying. A full-unit answer contains every
/// fragment itself; component evidence may span indexed units of the requested
/// languages in one folder. Each fragment must occur inside a unit, never across
/// concatenated unit boundaries. This is answerability, not selected-region scoring.
fn check_task(
    clone: &Path,
    index: &Index,
    task: &Task,
    work: &Path,
) -> Result<CheckOutcome, String> {
    let (idx, held) = prepare_holdout(clone, index, task, work)?;
    check_answers(work, &idx, task, &held, false)
}

fn check_answers(
    work: &Path,
    idx: &Index,
    task: &Task,
    held: &[Location],
    pooled: bool,
) -> Result<CheckOutcome, String> {
    let langs = task.langs();
    let mut files = BTreeMap::new();
    let mut groups: BTreeMap<(String, Option<u32>), UnitEvidence> = BTreeMap::new();
    for id in 0..idx.n_units() {
        let r = idx.record(id);
        if !langs.contains(&r.lang) || (!task.include_tests && r.is_test) {
            continue;
        }
        let path = idx.file_path(r.file).to_string_lossy().replace('\\', "/");
        if !files.contains_key(&path) {
            let text =
                std::fs::read_to_string(work.join(&path)).map_err(|e| format!("{path}: {e}"))?;
            files.insert(path.clone(), text);
        }
        let text = files[&path]
            .lines()
            .skip(r.lines.start.saturating_sub(1) as usize)
            .take((r.lines.end - r.lines.start) as usize)
            .collect::<Vec<_>>()
            .join("\n");
        let path_key = if task.lang == "component" {
            path.rsplit_once('/')
                .map_or(".", |(dir, _)| dir)
                .to_string()
        } else {
            format!("{path}:{}-{}", r.lines.start, r.lines.end - 1)
        };
        groups
            // Full-unit tasks count records, even when two units share a line.
            .entry((path_key, (task.lang != "component").then_some(id)))
            .or_default()
            .push(((path, r.lines.start, r.lines.end - 1), squash(&text)));
    }
    let answers: Vec<_> = groups
        .into_iter()
        .filter(|(_, units)| {
            task.required.iter().all(|fragment| {
                let fragment = squash(fragment);
                units.iter().any(|(_, text)| text.contains(&fragment))
            })
        })
        .map(|((path, _), units)| Answer {
            path,
            units: units.into_iter().map(|(location, _)| location).collect(),
        })
        .collect();
    let repo = task.repo.trim_end_matches('/').trim_end_matches(".git");
    let repo = repo.strip_prefix("https://github.com/").unwrap_or(repo);
    let mut witness_errors = Vec::new();
    for witness in &task.answer_witness {
        let Some((owner_repo, location)) = witness.split_once(':') else {
            witness_errors.push(format!("{witness}: expected owner/repo:path:line"));
            continue;
        };
        if !pooled && owner_repo != repo {
            witness_errors.push(format!("{witness}: outside task repository {repo}"));
            continue;
        }
        let Some((path, line)) = location
            .rsplit_once(':')
            .and_then(|(p, l)| l.parse::<u32>().ok().map(|line| (p, line)))
        else {
            witness_errors.push(format!("{witness}: expected owner/repo:path:line"));
            continue;
        };
        let path = if pooled {
            format!("{}/{}", owner_repo.replace('/', "__"), path)
        } else {
            path.to_owned()
        };
        let covers = |(p, first, last): &Location| *p == path && (*first..=*last).contains(&line);
        if held.iter().any(covers) {
            witness_errors.push(format!("{witness}: held out (source or exact-body clone)"));
        } else if !answers.iter().any(|a| a.units.iter().any(covers)) {
            witness_errors.push(format!("{witness}: not a post-holdout answer"));
        }
    }
    let mut query_errors = Vec::new();
    for (part, query) in std::iter::once(&task.query)
        .chain(task.parts.iter().map(|p| &p.query))
        .enumerate()
    {
        let query = squash(query);
        for fragment in &task.required {
            if query.contains(&squash(fragment)) {
                let label = if part == 0 {
                    "query".to_string()
                } else {
                    format!("part {part} query")
                };
                query_errors.push(format!("{label} contains required fragment {fragment:?}"));
            }
        }
    }
    Ok(CheckOutcome {
        held_out: held.len(),
        answers,
        witness_errors,
        query_errors,
    })
}

/// Copy `clone` to `work`, blank the held-out units, index the copy and score the task.
pub(crate) fn run_task(
    clone: &Path,
    index: &Index,
    task: &Task,
    work: &Path,
) -> Result<Outcome, String> {
    let langs = task.langs();
    let (idx, held) = prepare_holdout(clone, index, task, work)?;
    let mut opts = QueryOptions::new(langs[0]);
    opts.top_similar = TOP;
    opts.experimental_recommend = true;
    opts.include_tests = task.include_tests;
    if task.lang == "component" {
        let query = ComponentQuery {
            parts: task
                .parts
                .iter()
                .map(|p| ComponentPart {
                    lang: Lang::parse(&p.lang).expect("validated task part"),
                    source: p.query.clone(),
                    path: Some(p.path.clone()),
                })
                .collect(),
            top: TOP,
            options: opts,
        };
        let out = idx.query_component(&query)?;
        return Ok(Outcome {
            held_out: held.len(),
            similar_rank: component_rank(work, &out, &task.required),
            pruned_hit: false,
            has_recommendation: false,
            rec_hit: false,
        });
    }
    let out = idx.query(&task.query, &opts);
    let similar_rank = out
        .similar
        .iter()
        .take(TOP)
        .position(|s| unit_text(work, s).is_some_and(|t| contains_all(&t, &task.required)));
    let pruned_hit = out
        .similar
        .iter()
        .take(TOP)
        .any(|s| contains_all(&s.text, &task.required));
    let query = squash(&task.query);
    let missing: Vec<String> = task
        .required
        .iter()
        .filter(|r| !query.contains(&squash(r)))
        .cloned()
        .collect();
    let need = if missing.is_empty() {
        &task.required
    } else {
        &missing
    };
    let rec_hit = out
        .recommendations
        .iter()
        .take(TOP)
        .any(|r| contains_all(&r.text, need));
    Ok(Outcome {
        held_out: held.len(),
        similar_rank,
        pruned_hit,
        has_recommendation: !out.recommendations.is_empty(),
        rec_hit,
    })
}

/// One metadata-only pool with a repository prefix to disambiguate equal paths.
/// Walk each checkout with the indexer's ignore policy before linking, so pooling
/// cannot admit ignored files. Held-out files are replaced, never written through
/// their hard links. Cross-device sources fall back to copying metadata files only.
fn prepare_sfmeta_pool(repos: &[(String, PathBuf)], root: &Path) -> Result<(), String> {
    if root.exists() {
        std::fs::remove_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
    }
    std::fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
    for (name, path) in repos {
        let prefix = root.join(name.replace('/', "__"));
        let walker = ignore::WalkBuilder::new(path)
            .hidden(true)
            .parents(false)
            .require_git(false)
            .follow_links(false)
            .build();
        for entry in walker {
            let entry = entry.map_err(|e| format!("{}: {e}", path.display()))?;
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let rel = entry.path().strip_prefix(path).map_err(|e| e.to_string())?;
            if aroma_lang::spec::lang_for_path(&rel.to_string_lossy(), &[Lang::SfMeta]).is_none() {
                continue;
            }
            let dest = prefix.join(rel);
            std::fs::create_dir_all(dest.parent().expect("prefixed path"))
                .map_err(|e| format!("{}: {e}", dest.display()))?;
            if std::fs::hard_link(entry.path(), &dest).is_err() {
                std::fs::copy(entry.path(), &dest)
                    .map_err(|e| format!("{}: {e}", dest.display()))?;
            }
        }
    }
    Ok(())
}

fn pooled_task(task: &Task) -> Task {
    let mut pooled = task.clone();
    let slug = repo_slug(&task.repo, &task.commit);
    let prefix = slug.split('@').next().expect("repository slug");
    pooled.path = format!("{prefix}/{}", task.path);
    for part in &mut pooled.parts {
        part.path = format!("{prefix}/{}", part.path);
    }
    pooled
}

fn replace_pool_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("aroma-heldout-tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Reuse one pool/index, incrementally indexing this task's holdout. Compute all
/// holdouts against the intact baseline first, then restore files even on errors.
/// The next build picks up both the restored files and the next task's holdout.
fn with_pooled_sfmeta_holdout<T>(
    root: &Path,
    held: &[(String, u32, u32)],
    mode: SfmetaMode,
    evaluate: impl FnOnce(&Index) -> Result<T, String>,
) -> Result<(T, f64), String> {
    let mut originals = BTreeMap::new();
    for (path, _, _) in held {
        if !originals.contains_key(path) {
            let bytes = std::fs::read(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
            originals.insert(path.clone(), bytes);
        }
    }
    let result = (|| {
        for (path, bytes) in &originals {
            let mut text = String::from_utf8_lossy(bytes).into_owned();
            for (_, first, last) in held.iter().filter(|(p, _, _)| p == path) {
                text = blank_lines(&text, *first, *last);
            }
            replace_pool_file(&root.join(path), text.as_bytes())?;
        }
        let start = Instant::now();
        build_with_mode(root, &[Lang::SfMeta], mode)?;
        let seconds = start.elapsed().as_secs_f64();
        let index = Index::open(root).map_err(|e| format!("{}: {e}", root.display()))?;
        Ok((evaluate(&index)?, seconds))
    })();
    // Attempt every restoration, even when an earlier restoration fails.
    let mut restore_error = None;
    for (path, bytes) in originals {
        if let Err(e) = replace_pool_file(&root.join(path), &bytes) {
            restore_error = Some(e);
        }
    }
    match restore_error {
        Some(e) => Err(format!("restore pooled source: {e}")),
        None => result,
    }
}

fn run_pooled_sfmeta_task(
    root: &Path,
    task: &Task,
    held: &[Location],
    mode: SfmetaMode,
) -> Result<(Option<usize>, f64), String> {
    with_pooled_sfmeta_holdout(root, held, mode, |index| {
        let mut opts = QueryOptions::new(Lang::SfMeta);
        opts.top_similar = TOP;
        opts.include_tests = task.include_tests;
        let out = index.query(&task.query, &opts);
        Ok(out.similar.iter().take(TOP).position(|s| {
            unit_text(root, s).is_some_and(|text| contains_all(&text, &task.required))
        }))
    })
}

/// Checks and evaluation use exactly the same pinned population and routing.
fn sfmeta_sources(tasks: &[Task]) -> Result<Vec<(String, PathBuf)>, String> {
    let selected = crate::corpus::load()?.with_pilots("v1.2");
    for task in tasks {
        if task.lang != "sfmeta"
            || !selected
                .iter()
                .any(|r| r.url == task.repo && r.commit == task.commit)
        {
            return Err(format!(
                "{}: sfmeta task must name a pinned v1.2 repository or pilot",
                task.id
            ));
        }
    }
    selected
        .iter()
        .map(|repo| Ok((repo.name.clone(), repo.fetch()?)))
        .collect::<Result<Vec<_>, String>>()
}

fn run_sfmeta(tasks: &[Task], mode: SfmetaMode) -> Result<(), String> {
    let sources = sfmeta_sources(tasks)?;
    let root = target_dir().join("idiom-sfmeta").join(mode_name(mode));
    prepare_sfmeta_pool(&sources, &root)?;
    let start = Instant::now();
    let report = build_with_mode(&root, &[Lang::SfMeta], mode)?;
    let baseline_seconds = start.elapsed().as_secs_f64();
    println!("index sfmeta ({}) POOLED: {baseline_seconds:.3}s, {} repositories, {} units, {} files reparsed, {} failed, {} large, {} damaged units skipped",
        mode_name(mode), sources.len(), report.units, report.files_reparsed, report.files_failed,
        report.files_skipped_large, report.units_skipped_damaged);
    let baseline = Index::open(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    let prepared = tasks
        .iter()
        .map(|task| {
            let pooled = pooled_task(task);
            let held = held_out(&baseline, &pooled).map_err(|e| format!("{}: {e}", task.id))?;
            Ok((pooled, held))
        })
        .collect::<Result<Vec<_>, String>>()?;
    drop(baseline);
    let (mut hits, mut holdout_seconds) = (0, 0.0);
    for (task, held) in prepared {
        let (rank, seconds) = run_pooled_sfmeta_task(&root, &task, &held, mode)
            .map_err(|e| format!("{}: {e}", task.id))?;
        holdout_seconds += seconds;
        hits += usize::from(rank.is_some());
        println!(
            "{} {} similar {} (held out {}; index {seconds:.3}s)",
            if rank.is_some() { "HIT " } else { "MISS" },
            task.id,
            rank.map_or_else(|| "-".into(), |r| format!("rank {}", r + 1)),
            held.len()
        );
    }
    println!(
        "index sfmeta ({}) holdout TOTAL: {holdout_seconds:.3}s",
        mode_name(mode)
    );
    println!(
        "idiom sfmeta ({}): similar hit@5 {hits}/{} = {:.1}%",
        mode_name(mode),
        tasks.len(),
        100.0 * hits as f64 / tasks.len() as f64
    );
    // D42 compares the two modes; the Apex/LWC absolute 60% gate does not apply.
    Ok(())
}

struct Config {
    dir: PathBuf,
    only: Option<String>,
    lwc: bool,
    sfmeta: bool,
    sfmeta_mode: SfmetaMode,
    check: bool,
}

fn parse_args(args: &[String]) -> Result<Config, String> {
    let mut dir = None;
    let mut only: Option<String> = None;
    let mut lwc = false;
    let mut sfmeta = false;
    let mut mode = None;
    let mut check = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--check" => check = true,
            "--suite" => match it.next().map(String::as_str) {
                Some("lwc") if !lwc && !sfmeta => lwc = true,
                Some("sfmeta") if !lwc && !sfmeta => sfmeta = true,
                _ => return Err(format!("--suite needs one of lwc or sfmeta\n{USAGE}")),
            },
            "--sfmeta-mode" => mode = Some(parse_sfmeta_mode(it.next())?),
            "--tasks" => dir = Some(PathBuf::from(it.next().ok_or("--tasks needs a directory")?)),
            "--only" => only = Some(it.next().ok_or("--only needs a task id")?.clone()),
            other => return Err(format!("unknown idiom option {other:?}\n{USAGE}")),
        }
    }
    if mode.is_some() && !sfmeta {
        return Err("--sfmeta-mode requires --suite sfmeta".into());
    }
    Ok(Config {
        dir: dir.unwrap_or_else(|| {
            crate::workspace_root().join("xtask/tasks").join(if sfmeta {
                "sfmeta"
            } else if lwc {
                "lwc"
            } else {
                "apex"
            })
        }),
        only,
        lwc,
        sfmeta,
        sfmeta_mode: mode.unwrap_or(SfmetaMode::BUILD_DEFAULT),
        check,
    })
}

fn run_checks(c: &Config, tasks: &[Task]) -> Result<(), String> {
    if c.sfmeta {
        return run_sfmeta_checks(tasks, c.sfmeta_mode);
    }
    let mut indexes = BTreeMap::new();
    // Build each repository with all languages its tasks need, as in a real run.
    for task in tasks {
        let key = (task.repo.clone(), task.commit.clone());
        if indexes.contains_key(&key) {
            continue;
        }
        let slug = repo_slug(&task.repo, &task.commit);
        let clone = if c.lwc {
            corpus_dir()
                .join("sf")
                .join(slug.split('@').next().expect("repo slug"))
        } else {
            corpus_dir().join("idiom").join(slug)
        };
        let prepared = clone_at(&task.repo, &task.commit, &clone).and_then(|()| {
            let mut langs: Vec<_> = tasks
                .iter()
                .filter(|t| t.repo == task.repo && t.commit == task.commit)
                .flat_map(Task::langs)
                .collect();
            langs.sort();
            langs.dedup();
            build(&clone, &langs)?;
            Index::open(&clone).map_err(|e| format!("open {}: {e}", clone.display()))
        });
        indexes.insert(key, (clone, prepared));
    }
    let mut failed = 0;
    for task in tasks {
        let (clone, index) = &indexes[&(task.repo.clone(), task.commit.clone())];
        let work = target_dir().join("idiom-work").join(&task.id);
        let result = match index {
            Ok(index) => check_task(clone, index, task, &work),
            Err(e) => Err(e.clone()),
        };
        let _ = std::fs::remove_dir_all(&work);
        failed += usize::from(report_check(task, result));
    }
    println!("idiom check: {}/{} tasks pass (>= 2 post-holdout answers in own repository; valid declared witnesses; leak-free queries)", tasks.len() - failed, tasks.len());
    if failed == 0 {
        Ok(())
    } else {
        Err(format!(
            "{failed} idiom tasks failed post-holdout validation"
        ))
    }
}

fn report_check(task: &Task, result: Result<CheckOutcome, String>) -> bool {
    match result {
        Ok(out) => {
            let passes = out.passes();

            println!(
                "{} {}: {} post-holdout answers (held out {}; witness errors {}; query leaks {})",
                if passes { "PASS" } else { "FAIL" },
                task.id,
                out.answers.len(),
                out.held_out,
                out.witness_errors.len(),
                out.query_errors.len()
            );
            for answer in out.answers {
                println!("  answer {}", answer.path);
                if task.lang == "component" {
                    for (path, first, last) in answer.units {
                        println!("    unit {path}:{first}-{last}");
                    }
                }
            }
            for error in out.witness_errors {
                println!("  witness {error}");
            }
            for error in out.query_errors {
                println!("  leak {error}");
            }
            !passes
        }
        Err(e) => {
            println!("FAIL {}: {e}", task.id);
            true
        }
    }
}

fn run_sfmeta_checks(tasks: &[Task], mode: SfmetaMode) -> Result<(), String> {
    let sources = sfmeta_sources(tasks)?;
    let root = target_dir().join("idiom-sfmeta").join("check");
    prepare_sfmeta_pool(&sources, &root)?;
    build_with_mode(&root, &[Lang::SfMeta], mode)?;
    let baseline = Index::open(&root).map_err(|e| e.to_string())?;
    // Compute every holdout on the intact pool, exactly as in run_sfmeta.
    let prepared = tasks
        .iter()
        .map(|task| {
            let pooled = pooled_task(task);
            let held = held_out(&baseline, &pooled)?;
            Ok((pooled, held))
        })
        .collect::<Result<Vec<_>, String>>()?;
    drop(baseline);
    let mut failed = 0;
    for (task, held) in prepared {
        let result = with_pooled_sfmeta_holdout(&root, &held, mode, |index| {
            check_answers(&root, index, &task, &held, true)
        })
        .map(|(checked, _)| checked);
        failed += usize::from(report_check(&task, result));
    }
    println!("idiom check: {}/{} tasks pass (>= 2 post-holdout answers in v1.2 pool; valid declared witnesses; leak-free queries)", tasks.len() - failed, tasks.len());
    if failed == 0 {
        Ok(())
    } else {
        Err(format!(
            "{failed} idiom tasks failed post-holdout validation"
        ))
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let c = parse_args(args)?;
    let tasks: Vec<Task> = load_tasks(&c.dir)?
        .into_iter()
        .filter(|t| c.only.as_ref().is_none_or(|o| *o == t.id))
        .collect();
    if tasks.is_empty() {
        return Err(format!("no idiom tasks under {}", c.dir.display()));
    }
    if c.check {
        return run_checks(&c, &tasks);
    }
    if c.sfmeta {
        return run_sfmeta(&tasks, c.sfmeta_mode);
    }
    let mut groups: BTreeMap<(String, String), Vec<&Task>> = BTreeMap::new();
    for t in &tasks {
        groups
            .entry((t.repo.clone(), t.commit.clone()))
            .or_default()
            .push(t);
    }
    let mut results: Vec<(&Task, Result<Outcome, String>)> = Vec::new();
    for ((repo, commit), group) in &groups {
        let slug = repo_slug(repo, commit);
        let clone = if c.lwc {
            corpus_dir()
                .join("sf")
                .join(slug.split('@').next().expect("repo slug"))
        } else {
            corpus_dir().join("idiom").join(slug)
        };
        let prepared = clone_at(repo, commit, &clone).and_then(|()| {
            let mut langs: Vec<Lang> = group.iter().flat_map(|t| t.langs()).collect();
            langs.sort();
            langs.dedup();
            build(&clone, &langs)?;
            Index::open(&clone).map_err(|e| format!("open {}: {e}", clone.display()))
        });
        match prepared {
            Ok(index) => {
                for t in group {
                    let work = target_dir().join("idiom-work").join(&t.id);
                    let r = run_task(&clone, &index, t, &work);
                    let _ = std::fs::remove_dir_all(&work);
                    results.push((t, r));
                }
            }
            Err(e) => {
                for t in group {
                    results.push((t, Err(e.clone())));
                }
            }
        }
    }

    let (mut hits, mut pruned, mut wins, mut ties, mut losses) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    for (t, r) in &results {
        match r {
            Ok(o) => {
                if o.similar_rank.is_some() {
                    hits += 1;
                }
                if o.pruned_hit {
                    pruned += 1;
                }
                let v = verdict(o);
                match v {
                    Some(Verdict::Win) => wins += 1,
                    Some(Verdict::Tie) => ties += 1,
                    Some(Verdict::Loss) => losses += 1,
                    None => {}
                }
                println!(
                    "{} {:<36} similar {:<8} pruned-text {:<3} recommendation {:<6} (held out {})",
                    if o.similar_rank.is_some() {
                        "HIT "
                    } else {
                        "MISS"
                    },
                    t.id,
                    o.similar_rank
                        .map_or_else(|| "-".to_string(), |r| format!("rank {}", r + 1)),
                    if o.pruned_hit { "yes" } else { "no" },
                    match v {
                        Some(Verdict::Win) => "win",
                        Some(Verdict::Tie) => "tie",
                        Some(Verdict::Loss) => "loss",
                        None => "none",
                    },
                    o.held_out
                );
            }
            Err(e) if c.lwc => println!("MISS {:<36} similar - (error: {e})", t.id),
            Err(e) => println!("ERR  {:<36} {e}", t.id),
        }
    }
    let n = results.len();
    let rate = hits as f64 / n as f64;
    if c.lwc {
        println!(
            "idiom lwc: similar hit@5 {hits}/{n} = {:.1}% (gate >= 60%)",
            100.0 * rate
        );
    } else {
        println!(
        "idiom: similar hit@5 {hits}/{n} = {:.1}% (gate >= {:.0}%); pruned-text hit@5 {pruned}/{n}; \
         recommendations on {} tasks: {wins} wins, {ties} ties, {losses} losses, win-rate {:.1}% of all tasks (reported, not gated)",
        100.0 * rate,
        100.0 * HIT_AT_5_GATE,
        wins + ties + losses,
        100.0 * wins as f64 / n as f64
    );
    }
    if rate < HIT_AT_5_GATE {
        Err(format!(
            "similar hit@5 {:.1}% is below the {:.0}% gate",
            100.0 * rate,
            100.0 * HIT_AT_5_GATE
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDER_SERVICE: &str = r#"public with sharing class OrderService {
    public static void saveOrders(List<Order> orders) {
        Database.SaveResult[] results = Database.insert(orders, false);
        for (Database.SaveResult sr : results) {
            if (!sr.isSuccess()) {
                for (Database.Error err : sr.getErrors()) {
                    System.debug(LoggingLevel.ERROR, err.getMessage());
                }
            }
        }
    }
}
"#;

    const INVOICE_SERVICE: &str = r#"public with sharing class InvoiceService {
    public static void saveInvoices(List<Invoice__c> invoices) {
        Database.SaveResult[] results = Database.insert(invoices, false);
        for (Database.SaveResult sr : results) {
            if (!sr.isSuccess()) {
                for (Database.Error err : sr.getErrors()) {
                    System.debug(LoggingLevel.ERROR, err.getMessage());
                }
            }
        }
    }
}
"#;

    const GREETER: &str = r#"public with sharing class Greeter {
    public static String greet(String name) {
        if (String.isBlank(name)) {
            return 'Hello, stranger';
        }
        return 'Hello, ' + name;
    }
}
"#;

    const TASK: &str = r#"
repo = "https://github.com/example/orders"
commit = "0123456789abcdef0123456789abcdef01234567"
file = "force-app/main/default/classes/OrderService.cls"
source_lines = [2, 11]
query = """
Database.SaveResult[] results = Database.insert(orders, false);
for (Database.SaveResult sr : results) {
"""
required_elements = ["isSuccess()", "getErrors()"]
notes = "unknown keys are ignored"
"#;

    fn scratch(name: &str) -> PathBuf {
        let dir = target_dir()
            .join("xtask-scratch")
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn outcome(similar: bool, has_rec: bool, rec_hit: bool) -> Outcome {
        Outcome {
            held_out: 1,
            similar_rank: similar.then_some(0),
            pruned_hit: false,
            has_recommendation: has_rec,
            rec_hit,
        }
    }

    #[test]
    fn check_flag_is_accepted_for_lwc_and_default_suites() {
        assert!(parse_args(&["--check".into()]).unwrap().check);
        assert!(
            parse_args(&["--suite".into(), "lwc".into(), "--check".into()])
                .unwrap()
                .check
        );
    }

    #[test]
    fn check_counts_full_units_after_clone_holdout_and_validates_witnesses() {
        let root = scratch("check-units");
        let js = "class Card {\n    fire() {\n        this.dispatchEvent(new CustomEvent('done', { detail: 1 }));\n    }\n}\n";
        for name in ["source", "clone", "answer", "second", "tests/__tests__"] {
            let path = root.join(format!("lwc/{name}/card.js"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let text = match name {
                "answer" => js.replace("detail: 1", "detail: 2"),
                "second" => js.replace("detail: 1", "detail: 3"),
                "tests/__tests__" => js.replace("detail: 1", "detail: 4"),
                _ => js.to_string(),
            };
            std::fs::write(path, text).unwrap();
        }
        // Whole-file presence must not combine fragments across separate JS units.
        std::fs::write(root.join("lwc/answer/split.js"), "class Split {\n first() { this.dispatchEvent(event); }\n second() { return new CustomEvent('x', { detail: 5 }); }\n}\n").unwrap();
        let text = r#"
repo = "https://github.com/example/cards"
commit = "0123456789ab"
lang = "lwcjs"
source_file = "lwc/source/card.js"
source_lines = [2, 4]
query = "fire() {"
required = ["this.dispatchEvent(", "new CustomEvent(", "detail:"]
answer_witness = ["elsewhere/cards:lwc/answer/card.js:2", "example/cards:lwc/clone/card.js:2", "example/cards:lwc/answer/card.js:2"]
"#;
        let task = parse_task(text, "check").unwrap();
        build(&root, &[Lang::LwcJs]).unwrap();
        let index = Index::open(&root).unwrap();
        let work = root.with_extension("work");
        let checked = check_task(&root, &index, &task, &work).unwrap();
        assert_eq!(checked.held_out, 2);
        let paths: Vec<_> = checked.answers.iter().map(|a| a.path.as_str()).collect();
        assert_eq!(paths, ["lwc/answer/card.js:2-4", "lwc/second/card.js:2-4"]);
        assert_eq!(checked.witness_errors.len(), 2);
        assert!(checked.witness_errors[0].contains("outside"));
        assert!(checked.witness_errors[1].contains("held out"));
        assert!(!checked.passes());

        let valid = parse_task(
            &text
                .replace(
                    "elsewhere/cards:lwc/answer/card.js:2",
                    "example/cards:lwc/second/card.js:2",
                )
                .replace("\"example/cards:lwc/clone/card.js:2\", ", ""),
            "valid",
        )
        .unwrap();
        let checked = check_task(&root, &index, &valid, &work).unwrap();
        assert!(checked.passes());
        let mut with_tests = valid.clone();
        with_tests.include_tests = true;
        assert_eq!(
            check_task(&root, &index, &with_tests, &work)
                .unwrap()
                .answers
                .len(),
            3
        );
        let mut one = valid.clone();
        one.required.push("detail: 2".into());
        assert!(!check_task(&root, &index, &one, &work).unwrap().passes());
        assert_eq!(
            std::fs::read_to_string(root.join("lwc/source/card.js")).unwrap(),
            js
        );
        std::fs::remove_dir_all(work).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn check_counts_distinct_indexed_units_even_when_their_line_ranges_match() {
        let root = scratch("check-same-line");
        let dir = root.join("lwc/card");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("source.js"), "class Card {\n fire() {\n this.dispatchEvent(new CustomEvent('source', { detail: 1 }));\n }\n}\n").unwrap();
        std::fs::write(dir.join("answers.js"), "class Answers { first() { this.dispatchEvent(new CustomEvent('one', { detail: 2 })); } second() { this.dispatchEvent(new CustomEvent('two', { detail: 3 })); } }\n").unwrap();
        let task = parse_task(
            r#"
repo = "https://github.com/example/cards"
commit = "0123456789ab"
lang = "lwcjs"
source_file = "lwc/card/source.js"
source_lines = [2, 4]
query = "fire() {"
required = ["this.dispatchEvent(", "new CustomEvent(", "detail:"]
"#,
            "same-line",
        )
        .unwrap();
        build(&root, &[Lang::LwcJs]).unwrap();
        let index = Index::open(&root).unwrap();
        let work = root.with_extension("work");
        let checked = check_task(&root, &index, &task, &work).unwrap();
        assert_eq!(checked.answers.len(), 2);
        assert!(checked.passes());
        std::fs::remove_dir_all(work).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn check_components_combine_indexed_units_only_within_one_folder() {
        let root = scratch("check-components");
        for (name, marker) in [
            ("source", 1),
            ("clone", 1),
            ("answer", 2),
            ("second", 3),
            ("htmlOnly", 4),
            ("jsOnly", 5),
        ] {
            let dir = root.join(format!("lwc/{name}"));
            std::fs::create_dir_all(&dir).unwrap();
            if name != "jsOnly" {
                std::fs::write(dir.join("card.html"), format!("<template>\n<lightning-card title=\"{marker}\">\n<template if:true={{open}}>\n<p>evidence {marker}</p>\n</template>\n</lightning-card>\n</template>\n")).unwrap();
            }
            if name != "htmlOnly" {
                std::fs::write(dir.join("card.js"), format!("class Card {{\n @api get value() {{ return {marker}; }}\n show() {{ this.open = true; }}\n}}\n")).unwrap();
            }
        }
        let task = parse_task(
            r#"
repo = "https://github.com/example/cards"
commit = "0123456789ab"
lang = "component"
required = ["<lightning-card", "evidence", "@api"]
[[parts]]
lang = "lwchtml"
source_file = "lwc/source/card.html"
source_lines = [3, 5]
query = "<template if:true={open}>"
[[parts]]
lang = "lwcjs"
source_file = "lwc/source/card.js"
source_lines = [2, 2]
query = "get value()"
"#,
            "component",
        )
        .unwrap();
        build(&root, &[Lang::LwcJs, Lang::LwcHtml]).unwrap();
        let index = Index::open(&root).unwrap();
        let work = root.with_extension("work");
        let checked = check_task(&root, &index, &task, &work).unwrap();
        let paths: Vec<_> = checked.answers.iter().map(|a| a.path.as_str()).collect();
        // Blanking the nested template removes its evidence from the surviving parent too.
        assert_eq!(paths, ["lwc/answer", "lwc/second"]);
        assert!(checked.passes());
        std::fs::remove_dir_all(work).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn check_rejects_required_fragments_in_full_or_component_queries() {
        let root = scratch("check-query-leaks");
        for (name, marker) in [("source", 1), ("answer", 2), ("second", 3)] {
            let dir = root.join(format!("lwc/{name}"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("card.js"), format!("class Card {{\n fire() {{\n this.dispatchEvent(new CustomEvent('done', {{ detail: {marker} }}));\n }}\n}}\n")).unwrap();
            std::fs::write(dir.join("card.html"), format!("<template>\n<lightning-card title=\"{marker}\"></lightning-card>\n</template>\n")).unwrap();
        }
        let task = parse_task(
            r#"
repo = "https://github.com/example/cards"
commit = "0123456789ab"
lang = "lwcjs"
source_file = "lwc/source/card.js"
source_lines = [2, 4]
query = "fire() {"
required = ["new CustomEvent(", "detail:"]
"#,
            "query-leaks",
        )
        .unwrap();
        build(&root, &[Lang::LwcJs, Lang::LwcHtml]).unwrap();
        let index = Index::open(&root).unwrap();
        let work = root.with_extension("work");
        assert!(check_task(&root, &index, &task, &work).unwrap().passes());
        // Leaking any single required fragment is enough; whitespace and case
        // follow the same normalization as answer matching.
        let mut leaked = task.clone();
        leaked.query = "new \n CUSTOMEvent \t (".into();
        let out = check_task(&root, &index, &leaked, &work).unwrap();
        assert_eq!(out.answers.len(), 2);
        assert!(
            !out.passes(),
            "a full-unit query already contains its answer"
        );

        let mut component = task.clone();
        component.lang = "component".into();
        component.query.clear();
        component.parts = vec![
            TaskPart {
                lang: "lwcjs".into(),
                path: task.path.clone(),
                lines: task.lines,
                query: task.query.clone(),
            },
            TaskPart {
                lang: "lwchtml".into(),
                path: "lwc/source/card.html".into(),
                lines: [1, 3],
                query: "title=\"1\"".into(),
            },
        ];
        component.required.push("<lightning-card".into());
        assert!(check_task(&root, &index, &component, &work)
            .unwrap()
            .passes());
        for (part, query) in [(0, "detail \n :"), (1, "<lightning-\ncard")] {
            let mut leaked = component.clone();
            leaked.parts[part].query = query.into();
            let out = check_task(&root, &index, &leaked, &work).unwrap();
            assert_eq!(out.answers.len(), 2);
            assert!(
                !out.passes(),
                "component part {part} already contains an answer fragment"
            );
        }
        std::fs::remove_dir_all(work).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn task_files_accept_aliases_and_defaults() {
        let t = parse_task(TASK, "save-result").unwrap();
        assert_eq!(t.id, "save-result");
        assert_eq!(t.path, "force-app/main/default/classes/OrderService.cls");
        assert_eq!(t.lines, [2, 11]);
        assert_eq!(
            t.query,
            "Database.SaveResult[] results = Database.insert(orders, false);\nfor (Database.SaveResult sr : results) {\n"
        );
        assert_eq!(t.required, ["isSuccess()", "getErrors()"]);
        assert_eq!(t.lang, "apex");
        assert!(!t.include_tests);
        let explicit = format!("id = \"named\"\nlang = \"java\"\n{TASK}");
        let t = parse_task(&explicit, "ignored").unwrap();
        assert_eq!((t.id.as_str(), t.lang.as_str()), ("named", "java"));
    }

    #[test]
    fn task_files_are_validated() {
        let bad_lines = TASK.replace("[2, 11]", "[11, 2]");
        assert!(parse_task(&bad_lines, "t").unwrap_err().contains("1-based"));
        let no_required = TASK.replace(r#"["isSuccess()", "getErrors()"]"#, "[]");
        assert!(parse_task(&no_required, "t")
            .unwrap_err()
            .contains("required"));
        let bad_lang = format!("lang = \"cobol\"\n{TASK}");
        assert!(parse_task(&bad_lang, "t").unwrap_err().contains("cobol"));
        assert!(parse_task("repo = 1", "t").is_err());
    }

    #[test]
    fn component_task_files_parse_all_parts() {
        let text = include_str!("../tasks/lwc/lwc16-lms-subscribe.toml");
        let task = parse_task(text, "ignored").unwrap();
        assert_eq!(task.lang, "component");
        assert_eq!(task.parts.len(), 2);
        assert_eq!(task.parts[0].lang, "lwchtml");
        assert_eq!(task.parts[0].lines, [1, 39]);
        assert!(task.parts[0].path.ends_with(".html"));
        assert!(task.parts[1].query.contains("handlePropertySelected"));
        assert!(parse_task(&text.replace("[1, 39]", "[0, 39]"), "t").is_err());
        assert!(parse_task(&text.replace("lang = \"lwcjs\"", "lang = \"apex\""), "t").is_err());
        assert!(parse_task(&text[..text.find("[[parts]]").unwrap()], "t").is_err());
        let tasks = load_tasks(&crate::workspace_root().join("xtask/tasks/lwc")).unwrap();
        assert_eq!(tasks.len(), 20);
        assert_eq!(tasks.iter().filter(|t| t.lang == "component").count(), 6);
    }

    #[test]
    fn idiom_default_and_lwc_argument_paths() {
        let c = parse_args(&[]).unwrap();
        assert!(!c.lwc);
        assert!(c.dir.ends_with("xtask/tasks/apex"));
        let args = ["--only", "task", "--suite", "lwc", "--tasks", "custom"].map(String::from);
        let c = parse_args(&args).unwrap();
        assert!(c.lwc);
        assert_eq!(c.dir, PathBuf::from("custom"));
        assert_eq!(c.only.as_deref(), Some("task"));
        assert!(parse_args(&["--suite".into()]).is_err());
        assert!(parse_args(&["--suite".into(), "unknown".into()]).is_err());
    }

    #[test]
    fn sfmeta_selects_its_tasks_and_rejects_mode_flags_on_other_suites() {
        for mode in ["lexical", "structural"] {
            let args = ["--sfmeta-mode", mode, "--suite", "sfmeta"].map(String::from);
            let c = parse_args(&args).unwrap();
            assert!(c.dir.ends_with("xtask/tasks/sfmeta"));
            assert!(c.sfmeta && !c.lwc);
            assert_eq!(mode_name(c.sfmeta_mode), mode);
        }
        assert_eq!(
            parse_args(&["--suite".into(), "sfmeta".into()])
                .unwrap()
                .sfmeta_mode,
            SfmetaMode::Structural
        );
        assert!(parse_args(&["--suite".into(), "sfmeta".into()])
            .unwrap()
            .dir
            .ends_with("xtask/tasks/sfmeta"));
        for args in [
            vec!["--sfmeta-mode", "lexical"],
            vec!["--suite", "lwc", "--sfmeta-mode", "structural"],
            vec!["--suite", "sfmeta", "--sfmeta-mode", "unknown"],
            vec!["--suite", "sfmeta", "--sfmeta-mode"],
            vec!["--suite", "sfmeta", "--suite", "lwc"],
        ] {
            assert!(parse_args(&args.into_iter().map(String::from).collect::<Vec<_>>()).is_err());
        }
        let tasks = load_tasks(&crate::workspace_root().join("xtask/tasks/sfmeta")).unwrap();
        assert_eq!(tasks.len(), 15);
        assert!(tasks.iter().all(|t| t.lang == "sfmeta"));
    }

    #[test]
    fn pooled_check_accepts_cross_repo_answers_rejects_clones_absent_witnesses_and_leaks() {
        let base = scratch("pooled-check");
        let a = base.join("a");
        let b = base.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let source = "<ValidationRule>\n<fullName>Original</fullName>\n<errorConditionFormula>ISBLANK(Name)</errorConditionFormula>\n<errorMessage>Required</errorMessage>\n</ValidationRule>\n";
        std::fs::write(a.join("Source.validationRule-meta.xml"), source).unwrap();
        std::fs::write(b.join("Clone.validationRule-meta.xml"), source).unwrap();
        for name in ["First", "Second", "Ignored"] {
            std::fs::write(
                b.join(format!("{name}.validationRule-meta.xml")),
                source.replace("Original", name),
            )
            .unwrap();
        }
        std::fs::write(b.join(".gitignore"), "Ignored*\n").unwrap();
        let root = base.join("pool");
        prepare_sfmeta_pool(
            &[("owner/a".into(), a.clone()), ("owner/b".into(), b.clone())],
            &root,
        )
        .unwrap();
        let task = Task {
            id: "pooled-check".into(),
            repo: "https://github.com/owner/a".into(),
            commit: "unused".into(),
            lang: "sfmeta".into(),
            path: "Source.validationRule-meta.xml".into(),
            lines: [1, 5],
            query: "<ValidationRule><errorMessage>Required</errorMessage></ValidationRule>".into(),
            required: vec!["ISBLANK(Name)".into(), "<errorConditionFormula>".into()],
            include_tests: false,
            parts: vec![],
            answer_witness: vec![
                "owner/b:First.validationRule-meta.xml:3".into(),
                "owner/b:Second.validationRule-meta.xml:3".into(),
            ],
        };
        for mode in [SfmetaMode::Lexical, SfmetaMode::Structural] {
            build_with_mode(&root, &[Lang::SfMeta], mode).unwrap();
            let index = Index::open(&root).unwrap();
            assert_eq!(index.n_units(), 4);
            let pooled = pooled_task(&task);
            let held = held_out(&index, &pooled).unwrap();
            assert_eq!(held.len(), 2);
            let (checked, _) = with_pooled_sfmeta_holdout(&root, &held, mode, |index| {
                check_answers(&root, index, &pooled, &held, true)
            })
            .unwrap();
            assert_eq!(checked.answers.len(), 2);
            assert!(checked.passes());
            let mut invalid = pooled.clone();
            invalid.answer_witness.extend([
                "owner/b:Clone.validationRule-meta.xml:3".into(),
                "outside/pool:First.validationRule-meta.xml:3".into(),
                "owner/b:Ignored.validationRule-meta.xml:3".into(),
                "owner/b:Missing.validationRule-meta.xml:3".into(),
                "malformed".into(),
            ]);
            invalid.query.push_str("ISBLANK( Name )");
            let (checked, _) = with_pooled_sfmeta_holdout(&root, &held, mode, |index| {
                check_answers(&root, index, &invalid, &held, true)
            })
            .unwrap();
            assert_eq!(checked.answers.len(), 2);
            assert_eq!(checked.witness_errors.len(), 5);
            assert!(checked.witness_errors[0].contains("held out"));
            assert_eq!(checked.query_errors.len(), 1);
            assert!(!checked.passes());
            // Restoration also runs when answer enumeration fails.
            assert!(
                with_pooled_sfmeta_holdout::<()>(&root, &held, mode, |_| Err("probe".into()))
                    .is_err()
            );
            for path in [
                a.join("Source.validationRule-meta.xml"),
                b.join("Clone.validationRule-meta.xml"),
                root.join("owner__a/Source.validationRule-meta.xml"),
            ] {
                assert_eq!(std::fs::read_to_string(path).unwrap(), source);
            }
        }
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn pooled_metadata_holds_out_cross_repo_clones_and_restores_each_task_in_both_modes() {
        let base = scratch("pooled-metadata");
        let a = base.join("a");
        let b = base.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let source = "<ValidationRule>\n<fullName>Original</fullName>\n<active>true</active>\n<errorConditionFormula>ISBLANK(Name)</errorConditionFormula>\n<errorMessage>Required</errorMessage>\n</ValidationRule>\n";
        let answer = source.replace("Original", "Different");
        std::fs::write(a.join("X.validationRule-meta.xml"), source).unwrap();
        std::fs::write(b.join("X.validationRule-meta.xml"), source).unwrap();
        std::fs::write(b.join("Answer.validationRule-meta.xml"), &answer).unwrap();
        // The pooled population must obey the same ignore and routing rules as a
        // repository index, and must never include non-metadata or symlink files.
        std::fs::write(a.join("Ignored.validationRule-meta.xml"), &answer).unwrap();
        std::fs::write(a.join(".gitignore"), "Ignored*\n").unwrap();
        std::fs::write(a.join("not-metadata.xml"), source).unwrap();
        let task = Task {
            id: "pooled".into(), repo: "https://github.com/owner/a".into(),
            commit: "unused".into(), lang: "sfmeta".into(),
            path: "X.validationRule-meta.xml".into(), lines: [1, 6],
            query: "<ValidationRule>\n<active>true</active>\n<errorMessage>Required</errorMessage>\n</ValidationRule>".into(),
            required: vec!["ISBLANK(Name)".into(), "<errorConditionFormula>".into()],
            include_tests: false, parts: vec![], answer_witness: vec![],
        };
        for mode in [SfmetaMode::Lexical, SfmetaMode::Structural] {
            let root = base.join(mode_name(mode));
            prepare_sfmeta_pool(
                &[("owner/a".into(), a.clone()), ("owner/b".into(), b.clone())],
                &root,
            )
            .unwrap();
            build_with_mode(&root, &[Lang::SfMeta], mode).unwrap();
            let index = Index::open(&root).unwrap();
            assert_eq!(index.n_units(), 3);
            let mut pooled = pooled_task(&task);
            let held = held_out(&index, &pooled).unwrap();
            assert_eq!(
                held,
                vec![
                    ("owner__a/X.validationRule-meta.xml".into(), 1, 6),
                    ("owner__b/X.validationRule-meta.xml".into(), 1, 6),
                ]
            );
            let (rank, _) = run_pooled_sfmeta_task(&root, &pooled, &held, mode).unwrap();
            assert_eq!(rank, Some(0));
            assert_eq!(Index::open(&root).unwrap().n_units(), 1);
            for path in [
                a.join("X.validationRule-meta.xml"),
                b.join("X.validationRule-meta.xml"),
                root.join("owner__a/X.validationRule-meta.xml"),
            ] {
                assert_eq!(std::fs::read_to_string(path).unwrap(), source);
            }
            // Reuse the same pooled index: prior task holdouts must be restored.
            pooled.path = "owner__b/Answer.validationRule-meta.xml".into();
            let held = held_out(&index, &pooled).unwrap();
            run_pooled_sfmeta_task(&root, &pooled, &held, mode).unwrap();
            assert_eq!(Index::open(&root).unwrap().n_units(), 2);
        }
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn pooled_metadata_overlap_fallback_holds_every_intersecting_unit() {
        let root = scratch("metadata-overlap");
        std::fs::create_dir_all(root.join("owner__a")).unwrap();
        std::fs::create_dir_all(root.join("owner__b")).unwrap();
        let field = "<fields>\n<fullName>A</fullName>\n<label>Alpha</label>\n<type>Text</type>\n</fields>\n";
        std::fs::write(
            root.join("owner__a/Test.object"),
            format!(
                "<CustomObject>\n{field}{}{}</CustomObject>\n",
                field.replace('A', "B"),
                field.replace('A', "C")
            ),
        )
        .unwrap();
        std::fs::write(
            root.join("owner__b/Clone.object"),
            format!("<CustomObject>\n{field}</CustomObject>\n"),
        )
        .unwrap();
        build(&root, &[Lang::SfMeta]).unwrap();
        let index = Index::open(&root).unwrap();
        let task = Task {
            id: "overlap".into(),
            repo: "https://github.com/owner/a".into(),
            commit: "unused".into(),
            path: "Test.object".into(),
            lines: [3, 8],
            query: "<fields><label>Alpha</label>".into(),
            lang: "sfmeta".into(),
            required: vec!["no such fragment".into()],
            include_tests: false,
            parts: vec![],
            answer_witness: vec![],
        };
        let pooled = pooled_task(&task);
        let held = held_out(&index, &pooled).unwrap();
        assert_eq!(
            held,
            vec![
                ("owner__a/Test.object".into(), 2, 6),
                ("owner__a/Test.object".into(), 7, 11),
                ("owner__b/Clone.object".into(), 2, 6),
            ]
        );
        assert_eq!(
            run_pooled_sfmeta_task(&root, &pooled, &held, SfmetaMode::Lexical)
                .unwrap()
                .0,
            None
        );
        assert_eq!(Index::open(&root).unwrap().n_units(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn component_hits_use_full_matched_units_within_one_top_five_component() {
        use aroma_core::unit::UnitKind;
        use aroma_lang::index::ComponentHit;
        let root = scratch("component-hit");
        std::fs::write(
            root.join("part.js"),
            "outside\nsubscribe(channel);\nthis.messageContext;\noutside\n",
        )
        .unwrap();
        std::fs::write(
            root.join("part.html"),
            "outside\n<template>\n<lightning-card>\n</template>\n",
        )
        .unwrap();
        let js = SimilarOut {
            path: "part.js".into(),
            lines: [2, 3],
            kind: UnitKind::Method,
            score: 1.0,
            text: "pruned text lacks required fragments".into(),
            kept_spans: vec![],
            stale_source: false,
            explain: None,
            structural_rank: 1,
            lexical_rank: None,
            bm25: None,
            matched_terms: vec![],
        };
        let html = SimilarOut {
            path: "part.html".into(),
            kind: UnitKind::Template,
            ..js.clone()
        };
        let required = vec!["subscribe(".into(), "<lightning-card>".into()];
        let mut out = ComponentOutput {
            schema_version: 1,
            generation: 1,
            parts: vec![],
            components: vec![
                ComponentHit {
                    dir: "a".into(),
                    score: 1.0,
                    parts: vec![Some(js.clone()), None],
                },
                ComponentHit {
                    dir: "b".into(),
                    score: 0.9,
                    parts: vec![None, Some(html.clone())],
                },
            ],
        };
        assert_eq!(component_rank(&root, &out, &required), None);
        out.components.push(ComponentHit {
            dir: "c".into(),
            score: 0.8,
            parts: vec![Some(js), Some(html)],
        });
        assert_eq!(component_rank(&root, &out, &required), Some(2));
        assert_eq!(component_rank(&root, &out, &["outside".into()]), None);
        let hit = out.components.pop().unwrap();
        out.components.resize(5, out.components[0].clone());
        out.components.push(hit);
        assert_eq!(component_rank(&root, &out, &required), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_range_spanning_several_units_holds_out_every_overlapping_unit() {
        let root = scratch("span-holdout");
        let js = "class Card {\n    show() {\n        this.open = true;\n        this.render();\n    }\n    hide() {\n        this.open = false;\n        this.render();\n    }\n    other() {\n        return 1;\n    }\n}\n";
        let dir = root.join("lwc/card");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("card.js"), js).unwrap();
        build(&root, &[Lang::LwcJs]).unwrap();
        let index = Index::open(&root).unwrap();
        let part = TaskPart {
            lang: "lwcjs".into(),
            path: "lwc/card/card.js".into(),
            lines: [2, 9],
            query: "this.open = true;".into(),
        };
        let held = held_out_part(&index, &part).unwrap();
        let firsts: Vec<u32> = held.iter().map(|h| h.1).collect();
        assert_eq!(
            firsts,
            vec![2, 6],
            "show() and hide() held out, other() kept: {held:?}"
        );
        let outside = TaskPart {
            lines: [13, 13],
            ..part
        };
        assert!(held_out_part(&index, &outside).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn component_holdout_blanks_every_parts_unit_and_exact_clones() {
        let root = scratch("component-holdout");
        let js = "class Card {\n    show() {\n        this.open = true;\n        this.title = 'Card';\n        this.render();\n    }\n}\n";
        let html = "<template>\n    <lightning-card>\n        <p>{title}</p>\n        <p>{open}</p>\n    </lightning-card>\n</template>\n";
        for name in ["source", "clone"] {
            let dir = root.join(format!("lwc/{name}"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("{name}.js")), js).unwrap();
            std::fs::write(dir.join(format!("{name}.html")), html).unwrap();
        }
        build(&root, &[Lang::LwcJs, Lang::LwcHtml]).unwrap();
        let index = Index::open(&root).unwrap();
        let task = Task {
            id: "component".into(),
            repo: "unused".into(),
            commit: "unused".into(),
            path: String::new(),
            lines: [0, 0],
            query: String::new(),
            lang: "component".into(),
            include_tests: false,
            required: vec!["not present".into()],
            answer_witness: vec![],
            parts: vec![
                TaskPart {
                    lang: "lwcjs".into(),
                    path: "lwc/source/source.js".into(),
                    lines: [2, 6],
                    query: "this.open = true;".into(),
                },
                TaskPart {
                    lang: "lwchtml".into(),
                    path: "lwc/source/source.html".into(),
                    lines: [1, 6],
                    query: "<lightning-card>".into(),
                },
            ],
        };
        let held = held_out(&index, &task).unwrap();
        assert_eq!(held.len(), 4);
        let work = root.with_extension("work");
        let out = run_task(&root, &index, &task, &work).unwrap();
        assert_eq!(out.held_out, 4);
        assert_eq!(out.similar_rank, None);
        for (path, first, last) in held {
            let text = std::fs::read_to_string(work.join(path)).unwrap();
            assert!(text
                .lines()
                .skip(first as usize - 1)
                .take((last - first + 1) as usize)
                .all(str::is_empty));
        }
        std::fs::remove_dir_all(work).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fragments_match_ignoring_whitespace_and_case() {
        let text =
            "Database.SaveResult[] rs = Database.insert(x, false);\nif (!rs[0].isSuccess ()) {";
        fn req(v: &[&str]) -> Vec<String> {
            v.iter().map(|s| s.to_string()).collect()
        }
        assert!(contains_all(
            text,
            &req(&["database.insert(x,false)", "isSuccess()"])
        ));
        assert!(contains_all(text, &req(&["DATABASE . SAVERESULT"])));
        assert!(!contains_all(text, &req(&["isSuccess()", "getErrors()"])));
        assert_eq!(squash(" With  USER_MODE\n"), "withuser_mode");
    }

    #[test]
    fn blanking_keeps_line_count_and_endings() {
        assert_eq!(blank_lines("a\nb\r\nc\nd", 2, 3), "a\n\r\n\nd");
        assert_eq!(blank_lines("a\nb\n", 1, 1), "\nb\n");
        assert_eq!(blank_lines("a\nb\n", 5, 6), "a\nb\n");
    }

    #[test]
    fn verdicts_compare_recommendations_with_similar_code() {
        assert_eq!(verdict(&outcome(false, true, true)), Some(Verdict::Win));
        assert_eq!(verdict(&outcome(true, true, false)), Some(Verdict::Loss));
        assert_eq!(verdict(&outcome(true, true, true)), Some(Verdict::Tie));
        assert_eq!(verdict(&outcome(false, true, false)), Some(Verdict::Tie));
        assert_eq!(verdict(&outcome(true, false, false)), None);
    }

    #[test]
    fn repository_slugs_name_owner_repo_and_commit() {
        assert_eq!(
            repo_slug(
                "https://github.com/SalesforceFoundation/NPSP",
                "f5072b44781e7f31cb396c6919cfa7d0ead0ec6a"
            ),
            "SalesforceFoundation__NPSP@f5072b44781e"
        );
        assert_eq!(
            repo_slug("https://github.com/jongpie/NebulaLogger.git/", "b792ac8"),
            "jongpie__NebulaLogger@b792ac8"
        );
    }

    #[test]
    fn a_task_holds_out_source_and_clones_and_finds_the_sibling() {
        let base = scratch("idiom");
        let clone = base.join("clone");
        let classes = clone.join("force-app/main/default/classes");
        std::fs::create_dir_all(&classes).unwrap();
        std::fs::write(classes.join("OrderService.cls"), ORDER_SERVICE).unwrap();
        std::fs::write(
            classes.join("OrderServiceCopy.cls"),
            ORDER_SERVICE.replace("class OrderService", "class OrderServiceCopy"),
        )
        .unwrap();
        std::fs::write(classes.join("InvoiceService.cls"), INVOICE_SERVICE).unwrap();
        std::fs::write(classes.join("Greeter.cls"), GREETER).unwrap();
        let task = parse_task(TASK, "save-result").unwrap();

        build(&clone, &[Lang::Apex]).unwrap();
        let index = Index::open(&clone).unwrap();
        assert_eq!(
            held_out(&index, &task).unwrap(),
            vec![
                (
                    "force-app/main/default/classes/OrderService.cls".to_string(),
                    2,
                    11
                ),
                (
                    "force-app/main/default/classes/OrderServiceCopy.cls".to_string(),
                    2,
                    11
                ),
            ]
        );

        let work = base.join("work");
        let o = run_task(&clone, &index, &task, &work).unwrap();
        assert_eq!(o.held_out, 2);
        assert_eq!(o.similar_rank, Some(0));
        let blanked =
            std::fs::read_to_string(work.join("force-app/main/default/classes/OrderService.cls"))
                .unwrap();
        assert_eq!(
            blanked,
            "public with sharing class OrderService {\n\n\n\n\n\n\n\n\n\n\n}\n"
        );
        // the clone itself is untouched
        assert_eq!(
            std::fs::read_to_string(classes.join("OrderService.cls")).unwrap(),
            ORDER_SERVICE
        );

        let mut missing = task.clone();
        missing.lines = [40, 41];
        assert!(held_out(&index, &missing)
            .unwrap_err()
            .contains("covers or overlaps lines 40-41"));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn checked_in_task_files_parse() {
        let tasks = load_tasks(&crate::workspace_root().join("xtask/tasks/apex")).unwrap();
        assert_eq!(tasks.len(), 40);
        assert!(tasks
            .iter()
            .all(|t| t.lang == "apex" && !t.required.is_empty()));
    }
}
