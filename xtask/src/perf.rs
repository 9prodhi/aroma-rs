//! `perf`: the plan §6 / definition-of-done performance gates.
//!
//! ```text
//! cargo run --release -p xtask -- perf [--java-repos N] [--queries N] [--component-queries N] [--seed S] [--skip-index] [--skip-query] [--skip-component]
//! ```
//!
//! 1. NPSP pilot checkout: delete `.aroma/`, time a full `build_index` with every
//!    specced language (`Lang::SPECCED`), printing units per language. Gate <= 60 s.
//! 2. Query fixture = the recall Java corpus (first `--java-repos`, default 100,
//!    repositories; the DoD's 200k-unit fixture; fewer units print a warning). Build the
//!    release `aroma` binary, write `--queries` (100) contiguous first-5-line queries
//!    (seed 119), run one warm-up, then time each as its own `aroma query` process
//!    (one-shot latency, D23c). Gates: p50 <= 100 ms, p95 <= 500 ms (nearest rank).
//! 3. Component phase (skippable with `--skip-component`): index the v1.1 pinned
//!    `trailheadapps/lwc-recipes` with all specced languages, choose up to
//!    `--component-queries` (50) LWC component folders deterministically (seed) that have
//!    both a `.js` and a `.html` file, and after one warm-up time each
//!    `aroma query --component <folder> --root <repo> --json` as its own process (same
//!    one-shot method as the query phase). Gate p50 <= 200 ms (a component query runs two
//!    searches); p95 and max are reported. If lwc-recipes is missing, the phase fails with
//!    a clear message pointing at `cargo run -p xtask -- fetch-corpus --stage v1.1`.
//! 4. RSS (Linux): the peak after the builds is reported; the same queries then run in
//!    a fresh `xtask perf --rss-probe` child on a freshly opened index, which reports its
//!    own `VmHWM` (this process still holds the build's heap, so measuring here would
//!    count index-build memory as query memory). Gate <= 1 GiB. Skipped where `/proc` is
//!    missing.

use crate::recall::{
    build, contiguous_query, java_corpus, num, pilot_repo, sample_units, target_dir, Rng,
    DEFAULT_SEED,
};
use aroma_core::lang::Lang;
use aroma_core::unit::UnitStore;
use aroma_lang::index::{Index, QueryOptions};
use aroma_lang::lang_for_path;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

const NPSP_INDEX_SECS: f64 = 60.0;
const QUERY_P50_MS: f64 = 100.0;
const QUERY_P95_MS: f64 = 500.0;
const MAX_RSS_MIB: f64 = 1024.0;
const FIXTURE_UNITS: u32 = 200_000;
const DEFAULT_JAVA_REPOS: usize = 100;
const DEFAULT_QUERIES: usize = 100;
const DEFAULT_COMPONENT_QUERIES: usize = 50;
const COMPONENT_P50_MS: f64 = 200.0;
const USAGE: &str = "usage: cargo run --release -p xtask -- perf [--java-repos N] [--queries N] [--component-queries N] [--seed S] [--skip-index] [--skip-query] [--skip-component]";

/// Nearest-rank percentile of an ascending slice (`0.0` when empty).
pub(crate) fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (q * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// `VmHWM` (peak resident set) of a `/proc/<pid>/status` text, in bytes.
pub(crate) fn parse_vm_hwm(status: &str) -> Option<u64> {
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))
        .and_then(|v| v.trim().strip_suffix("kB"))
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|kb| kb * 1024)
}

fn peak_rss_mib() -> Option<f64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    parse_vm_hwm(&status).map(|b| b as f64 / (1024.0 * 1024.0))
}

const PROBE_MARKER: &str = "aroma-perf-rss-bytes";

/// Peak RSS reported by a `--rss-probe` child on its marker line.
pub(crate) fn parse_probe_output(stdout: &str) -> Option<u64> {
    stdout
        .lines()
        .find_map(|l| l.strip_prefix(PROBE_MARKER))
        .and_then(|v| v.trim().parse().ok())
}

/// Hidden mode `perf --rss-probe ROOT FILE...`: open the index and run every query in
/// this (fresh) process, then print the process's peak RSS. The parent cannot measure
/// in-process because the index build it just ran leaves its heap resident, and resetting
/// `VmHWM` only lowers the peak to the current (still inflated) RSS.
pub(crate) fn rss_probe(args: &[String]) -> Result<(), String> {
    let [root, files @ ..] = args else {
        return Err("--rss-probe needs ROOT and at least one query file".into());
    };
    if files.is_empty() {
        return Err("--rss-probe needs ROOT and at least one query file".into());
    }
    let index = Index::open(Path::new(root)).map_err(|e| format!("open {root}: {e}"))?;
    let opts = QueryOptions::new(Lang::Java);
    for f in files {
        let text = std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))?;
        let _ = index.query(&text, &opts);
    }
    let status = std::fs::read_to_string("/proc/self/status").map_err(|e| e.to_string())?;
    let peak = parse_vm_hwm(&status).ok_or("no VmHWM in /proc/self/status")?;
    println!("{PROBE_MARKER} {peak}");
    Ok(())
}

/// Peak RSS (MiB) of a fresh `xtask perf --rss-probe` process running `files` on `root`.
fn probe_query_rss_mib(root: &Path, files: &[PathBuf]) -> Result<f64, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let out = Command::new(&exe)
        .arg("perf")
        .arg("--rss-probe")
        .arg(root)
        .args(files)
        .output()
        .map_err(|e| format!("cannot run {}: {e}", exe.display()))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        return Err(format!(
            "rss probe failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    parse_probe_output(&stdout)
        .map(|b| b as f64 / (1024.0 * 1024.0))
        .ok_or_else(|| format!("rss probe printed no {PROBE_MARKER} line"))
}

pub(crate) fn at_most(failures: &mut Vec<String>, what: &str, value: f64, max: f64, unit: &str) {
    if value > max {
        failures.push(format!("{what} {value:.1} {unit} > {max:.1} {unit}"));
    }
}

pub(crate) struct Config {
    pub(crate) java_repos: usize,
    pub(crate) queries: usize,
    pub(crate) component_queries: usize,
    pub(crate) seed: u64,
    pub(crate) index: bool,
    pub(crate) query: bool,
    pub(crate) component: bool,
}

pub(crate) fn parse_args(args: &[String]) -> Result<Config, String> {
    let mut c = Config {
        java_repos: DEFAULT_JAVA_REPOS,
        queries: DEFAULT_QUERIES,
        component_queries: DEFAULT_COMPONENT_QUERIES,
        seed: DEFAULT_SEED,
        index: true,
        query: true,
        component: true,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--java-repos" => c.java_repos = num(it.next(), arg)?,
            "--queries" => c.queries = num(it.next(), arg)?,
            "--component-queries" => c.component_queries = num(it.next(), arg)?,
            "--seed" => c.seed = num(it.next(), arg)?,
            "--skip-index" => c.index = false,
            "--skip-query" => c.query = false,
            "--skip-component" => c.component = false,
            other => return Err(format!("unknown perf option {other:?}\n{USAGE}")),
        }
    }
    Ok(c)
}

/// `cargo build --release -p aroma-cli --bin aroma`; returns the binary path.
fn build_cli() -> Result<PathBuf, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let status = Command::new(&cargo)
        .args(["build", "--release", "-p", "aroma-cli", "--bin", "aroma"])
        .current_dir(crate::workspace_root())
        .status()
        .map_err(|e| format!("cannot run {cargo}: {e}"))?;
    if !status.success() {
        return Err("cargo build --release -p aroma-cli failed".into());
    }
    let bin = target_dir()
        .join("release")
        .join(format!("aroma{}", std::env::consts::EXE_SUFFIX));
    if bin.is_file() {
        Ok(bin)
    } else {
        Err(format!("{} not found after the build", bin.display()))
    }
}

/// Wall time of one `aroma query` process, in milliseconds.
fn one_shot(bin: &Path, root: &Path, query: &Path) -> Result<f64, String> {
    let start = Instant::now();
    let out = Command::new(bin)
        .args(["query", "--lang", "java", "--json", "--root"])
        .arg(root)
        .arg("--file")
        .arg(query)
        .output()
        .map_err(|e| format!("cannot run {}: {e}", bin.display()))?;
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    if !out.status.success() {
        return Err(format!(
            "aroma query --file {} failed: {}",
            query.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(ms)
}

/// Wall time of one `aroma query --component <dir>` process, in milliseconds.
fn one_shot_component(bin: &Path, root: &Path, component: &Path) -> Result<f64, String> {
    let start = Instant::now();
    let out = Command::new(bin)
        .args(["query", "--component"])
        .arg(component)
        .args(["--root"])
        .arg(root)
        .arg("--json")
        .output()
        .map_err(|e| format!("cannot run {}: {e}", bin.display()))?;
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    if !out.status.success() {
        return Err(format!(
            "aroma query --component {} failed: {}",
            component.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(ms)
}

/// True when `dir` holds at least one file routable as `LwcJs` and at least one routable
/// as `LwcHtml` (the adapters require an `lwc/<component>/` ancestor, so this restricts
/// candidates to real LWC component folders).
fn is_lwc_component(dir: &Path) -> Result<bool, String> {
    let read_error = |e: std::io::Error| format!("cannot read {}: {e}", dir.display());
    let mut has_js = false;
    let mut has_html = false;
    for entry in std::fs::read_dir(dir).map_err(read_error)? {
        let path = entry.map_err(read_error)?.path();
        if !path.is_file() {
            continue;
        }
        let rel = path.to_string_lossy().replace('\\', "/");
        if lang_for_path(&rel, &[Lang::LwcJs]).is_some() {
            has_js = true;
        } else if lang_for_path(&rel, &[Lang::LwcHtml]).is_some() {
            has_html = true;
        }
    }
    Ok(has_js && has_html)
}

/// All LWC component folders under `root` (recursively) that hold both a `.js` and a
/// `.html` file, in deterministic path order.
fn lwc_component_folders(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut folders = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let read_error = |e: std::io::Error| format!("cannot read {}: {e}", dir.display());
        for entry in std::fs::read_dir(&dir).map_err(read_error)? {
            let path = entry.map_err(read_error)?.path();
            if path.is_dir() {
                if is_lwc_component(&path)? {
                    folders.push(path.clone());
                }
                stack.push(path);
            }
        }
    }
    folders.sort();
    Ok(folders)
}

/// Deterministically pick up to `n` component folders (seed; Fisher–Yates as in the
/// pilot), preserving the candidate set when `n` is at least the length.
fn select_component_folders(mut folders: Vec<PathBuf>, n: usize, seed: u64) -> Vec<PathBuf> {
    let mut rng = Rng::new(seed);
    rng.shuffle(&mut folders);
    folders.truncate(n);
    folders
}

/// Units per language in `index`, for the per-language printout.
fn units_per_lang(index: &Index) -> Vec<(Lang, u32)> {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<Lang, u32> = BTreeMap::new();
    for id in 0..index.n_units() {
        *counts.entry(index.record(id).lang).or_insert(0) += 1;
    }
    counts.into_iter().collect()
}

pub fn run(args: &[String]) -> Result<(), String> {
    if args.first().map(String::as_str) == Some("--rss-probe") {
        return rss_probe(&args[1..]);
    }
    let c = parse_args(args)?;
    let mut failures = Vec::new();
    if c.index {
        let npsp = pilot_repo("NPSP")?;
        let aroma_dir = npsp.join(".aroma");
        if aroma_dir.exists() {
            std::fs::remove_dir_all(&aroma_dir)
                .map_err(|e| format!("{}: {e}", aroma_dir.display()))?;
        }
        let start = Instant::now();
        let report = build(&npsp, Lang::SPECCED)?;
        let secs = start.elapsed().as_secs_f64();
        println!(
            "perf index NPSP: {} units from {} files in {secs:.2} s (gate <= {NPSP_INDEX_SECS} s)",
            report.units, report.files_reparsed
        );
        let index = Index::open(&npsp).map_err(|e| format!("open {}: {e}", npsp.display()))?;
        let per_lang: Vec<String> = units_per_lang(&index)
            .iter()
            .map(|(lang, n)| format!("{}={}", lang.as_str(), n))
            .collect();
        println!(
            "perf index NPSP units per language: {}",
            per_lang.join(", ")
        );
        at_most(&mut failures, "NPSP full index", secs, NPSP_INDEX_SECS, "s");
    }
    let bin = if c.query || c.component {
        Some(build_cli()?)
    } else {
        None
    };
    if c.query {
        let root = java_corpus(c.java_repos)?;
        build(&root, &[Lang::Java])?;
        if let Some(peak) = peak_rss_mib() {
            println!("perf index peak RSS {peak:.0} MiB (reported, not gated)");
            if peak > MAX_RSS_MIB {
                eprintln!("perf: warning: index peak RSS is above 1 GiB");
            }
        }
        let index = Index::open(&root).map_err(|e| format!("open {}: {e}", root.display()))?;
        let units = index.n_units();
        if units < FIXTURE_UNITS {
            println!(
                "perf: warning: the fixture has {units} units (< {FIXTURE_UNITS}); raise --java-repos for the DoD fixture"
            );
        }
        let samples = sample_units(&index, Lang::Java, c.queries, &mut Rng::new(c.seed));
        drop(index);
        if samples.is_empty() {
            return Err("no query samples in the fixture".into());
        }
        let qdir = target_dir().join("perf").join("queries");
        let _ = std::fs::remove_dir_all(&qdir);
        std::fs::create_dir_all(&qdir).map_err(|e| format!("{}: {e}", qdir.display()))?;
        let mut files = Vec::with_capacity(samples.len());
        for (i, s) in samples.iter().enumerate() {
            let p = qdir.join(format!("q{i:03}.java"));
            std::fs::write(&p, contiguous_query(s)).map_err(|e| format!("{}: {e}", p.display()))?;
            files.push(p);
        }
        let bin = bin.as_ref().expect("query phase built the cli");
        one_shot(bin, &root, &files[0])?;
        let mut ms = Vec::with_capacity(files.len());
        for f in &files {
            ms.push(one_shot(bin, &root, f)?);
        }
        ms.sort_by(f64::total_cmp);
        let (p50, p95) = (percentile(&ms, 0.50), percentile(&ms, 0.95));
        println!(
            "perf one-shot aroma query on {units} units, {} queries: p50 {p50:.1} ms (gate <= {QUERY_P50_MS}), p95 {p95:.1} ms (gate <= {QUERY_P95_MS}), max {:.1} ms",
            ms.len(),
            ms[ms.len() - 1]
        );
        at_most(&mut failures, "query p50", p50, QUERY_P50_MS, "ms");
        at_most(&mut failures, "query p95", p95, QUERY_P95_MS, "ms");
        if Path::new("/proc/self/status").exists() {
            // A fresh process: this one still holds the heap of the index build above.
            let peak = probe_query_rss_mib(&root, &files)?;
            println!(
                "perf query peak RSS {peak:.0} MiB in a fresh process, {} queries (gate <= {MAX_RSS_MIB} MiB)",
                files.len()
            );
            at_most(&mut failures, "query peak RSS", peak, MAX_RSS_MIB, "MiB");
        } else {
            println!("perf: RSS gate skipped (no /proc on this platform)");
        }
    }
    if c.component {
        let bin = bin.as_ref().expect("component phase built the cli");
        let corpus = crate::corpus::load()?;
        let repo = corpus
            .stage(Some("v1.1"))
            .find(|r| r.name == "trailheadapps/lwc-recipes")
            .ok_or_else(|| {
                "trailheadapps/lwc-recipes is not pinned in the v1.1 corpus".to_string()
            })?;
        let root = repo.path();
        if !root.join(".git").exists() {
            return Err(format!(
                "lwc-recipes corpus is missing at {}; run `cargo run -p xtask -- fetch-corpus --stage v1.1`",
                root.display()
            ));
        }
        let report = build(&root, Lang::SPECCED)?;
        println!(
            "perf component index lwc-recipes: {} units from {} files",
            report.units, report.files_reparsed
        );
        let folders = lwc_component_folders(&root)?;
        let selected = select_component_folders(folders, c.component_queries, c.seed);
        if selected.is_empty() {
            return Err(
                "no LWC component folder holding both a .js and a .html was found in lwc-recipes"
                    .into(),
            );
        }
        one_shot_component(bin, &root, &selected[0])?;
        let mut ms = Vec::with_capacity(selected.len());
        for folder in &selected {
            ms.push(one_shot_component(bin, &root, folder)?);
        }
        ms.sort_by(f64::total_cmp);
        let (p50, p95) = (percentile(&ms, 0.50), percentile(&ms, 0.95));
        println!(
            "perf one-shot component query on {} components: p50 {p50:.1} ms (gate <= {COMPONENT_P50_MS} ms), p95 {p95:.1} ms, max {:.1} ms",
            ms.len(),
            ms[ms.len() - 1]
        );
        at_most(&mut failures, "component p50", p50, COMPONENT_P50_MS, "ms");
    }
    if failures.is_empty() {
        println!("perf gates passed");
        Ok(())
    } else {
        Err(format!("perf gates failed: {}", failures.join("; ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rss_probe_output_is_parsed_from_its_marker_line() {
        let out = "noise\naroma-perf-rss-bytes 52428800\n";
        assert_eq!(parse_probe_output(out), Some(52_428_800));
        assert_eq!(parse_probe_output("aroma-perf-rss-bytes x\n"), None);
        assert_eq!(parse_probe_output(""), None);
    }

    #[test]
    fn rss_probe_runs_queries_in_a_fresh_process_view() {
        // probe mode needs a root and at least one query file
        assert!(rss_probe(&["only-root".to_string()]).is_err());
    }

    #[test]
    fn percentiles_use_the_nearest_rank() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&v, 0.50), 50.0);
        assert_eq!(percentile(&v, 0.95), 95.0);
        assert_eq!(percentile(&v, 1.0), 100.0);
        assert_eq!(percentile(&v, 0.0), 1.0);
        assert_eq!(percentile(&[7.0], 0.95), 7.0);
        assert_eq!(percentile(&[], 0.5), 0.0);
    }

    #[test]
    fn peak_rss_is_read_from_vm_hwm() {
        let status = "Name:\txtask\nVmPeak:\t  999 kB\nVmHWM:\t  123456 kB\nVmRSS:\t  1 kB\n";
        assert_eq!(parse_vm_hwm(status), Some(123456 * 1024));
        assert_eq!(parse_vm_hwm("Name:\txtask\n"), None);
    }

    #[test]
    fn gates_fail_only_above_the_limit() {
        let mut failures = Vec::new();
        at_most(&mut failures, "p50", 100.0, 100.0, "ms");
        assert!(failures.is_empty());
        at_most(&mut failures, "p95", 500.5, 500.0, "ms");
        assert_eq!(failures, ["p95 500.5 ms > 500.0 ms"]);
    }

    #[test]
    fn options_parse_with_defaults() {
        let c = parse_args(&[]).unwrap();
        assert_eq!(
            (
                c.java_repos,
                c.queries,
                c.component_queries,
                c.seed,
                c.index,
                c.query,
                c.component
            ),
            (100, 100, 50, 119, true, true, true)
        );
        let args: Vec<String> = [
            "--skip-index",
            "--queries",
            "7",
            "--java-repos",
            "3",
            "--component-queries",
            "9",
            "--skip-component",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let c = parse_args(&args).unwrap();
        assert_eq!(
            (
                c.java_repos,
                c.queries,
                c.component_queries,
                c.index,
                c.query,
                c.component
            ),
            (3, 7, 9, false, true, false)
        );
        assert!(parse_args(&["--queries".to_string()]).is_err());
        assert!(parse_args(&["--component-queries".to_string()]).is_err());
        assert!(parse_args(&["--fast".to_string()]).is_err());
    }

    #[test]
    fn component_folders_are_selected_deterministically_by_seed() {
        let folders: Vec<PathBuf> = (0..6).map(|i| PathBuf::from(format!("lwc/c{i}"))).collect();
        // A count at least the candidate length keeps every folder.
        let all = select_component_folders(folders.clone(), 99, 119);
        let mut sorted = all.clone();
        sorted.sort();
        let mut expected = folders.clone();
        expected.sort();
        assert_eq!(sorted, expected);
        // A smaller count picks the same subset for the same seed, in the same order.
        let a = select_component_folders(folders.clone(), 3, 119);
        let b = select_component_folders(folders.clone(), 3, 119);
        assert_eq!(a, b);
        assert_eq!(a.len(), 3);
        // Same seed reproduces the same selection; the picks are real candidates.
        assert_eq!(a, select_component_folders(folders.clone(), 3, 119));
        assert!(a.iter().all(|f| folders.contains(f)));
        assert_eq!(
            select_component_folders(folders, 0, 119),
            Vec::<PathBuf>::new()
        );
    }
}
