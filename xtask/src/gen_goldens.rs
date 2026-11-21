//! `gen-goldens` (D13, task T14): differential goldens from the Python reference.
//!
//! ```text
//!  xtask/goldens/inputs/*.java (40 methods, ours)
//!     │ Java adapter, VarMode::Compat ──> Spt ──spt_to_reference_json──> corpus.jsonl
//!     │ 13 source mutations each (rename, drop, swap, prefix) ───────────> queries.jsonl
//!     ▼
//!  target/aroma-venv            python3 -m venv + pinned numpy/scipy/scikit-learn wheels
//!  target/aroma-goldens/scratch-<pid>/
//!     similar.py  (copy from AROMA_REFERENCE_DIR; driver.py checks its sha256)
//!     driver.py   (copy of xtask/goldens/driver.py) ── venv python -I ──┐
//!                                                                       ▼
//!  crates/aroma-core/tests/fixtures/goldens/{meta,corpus,cases}.json   (checked in)
//! ```
//!
//! similar.py (facebookresearch/aroma-paper-artifacts, CC BY-NC-SA 4.0) is never copied into
//! this repository and never run in place: it is copied into a fresh scratch directory under
//! `target/` and only its outputs on our own inputs are committed. CI never runs Python; it
//! replays the checked-in goldens in `crates/aroma-lang/tests/differential.rs` (T16).
use aroma_core::lang::Lang;
use aroma_core::spt::{Elem, NodeId, Spt};
use aroma_lang::classify::{unit_spt, VarMode};
use aroma_lang::globals::Globals;
use aroma_lang::kraw::KRaw;
use aroma_lang::spec::spec_for;
use aroma_lang::table::{declared_types, extract_units, parse, Limits};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;

/// sha256 of `reference/src/main/python/similar.py` at `REFERENCE_COMMIT`. driver.py
/// refuses to execute any other file.
const SIMILAR_PY_SHA256: &str = "7203c86b1fca6cf21373cca2d9fc2f0a23a1685cc559887b02dbc52b9f411065";
const SIMILAR_PY: &str = "reference/src/main/python/similar.py";
/// Installed with `--only-binary=:all:` (wheels only: no setup.py ever runs). Versions
/// support CPython 3.10–3.13 and similar.py's imports (DBSCAN, AffinityPropagation,
/// CountVectorizer, linear_kernel).
const PY_PACKAGES: &[&str] = &[
    "numpy==2.1.3",
    "scipy==1.14.1",
    "scikit-learn==1.5.2",
    "joblib==1.4.2",
    "threadpoolctl==3.5.0",
];
const INPUTS_DIR: &str = "xtask/goldens/inputs";
const DRIVER: &str = "xtask/goldens/driver.py";
const GOLDENS_DIR: &str = "crates/aroma-core/tests/fixtures/goldens";
/// 40 inputs x 13 mutations.
const EXPECTED_CASES: usize = 520;
const USAGE: &str = "usage: cargo run -p xtask -- gen-goldens [--keep-scratch]";

pub fn run(args: &[String]) -> Result<(), String> {
    let keep_scratch = match args {
        [] => false,
        [flag] if flag == "--keep-scratch" => true,
        _ => return Err(USAGE.into()),
    };
    let root = crate::workspace_root();
    let reference = std::env::var_os("AROMA_REFERENCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("ref").join("aroma-paper-artifacts"));
    let similar = reference.join(SIMILAR_PY);
    if !similar.is_file() {
        return Err(format!(
            "{} not found: run `cargo run -p xtask -- fetch-reference` or set AROMA_REFERENCE_DIR",
            similar.display()
        ));
    }
    let (corpus, queries) = build_records(&root.join(INPUTS_DIR))?;
    if queries.len() != EXPECTED_CASES {
        return Err(format!(
            "{} query cases, expected {EXPECTED_CASES} (40 inputs x 13 mutations)",
            queries.len()
        ));
    }
    eprintln!(
        "gen-goldens: {} corpus methods, {} query cases",
        corpus.len(),
        queries.len()
    );

    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let python = ensure_venv(&target.join("aroma-venv"))?;
    let scratch = target
        .join("aroma-goldens")
        .join(format!("scratch-{}", std::process::id()));
    if scratch.exists() {
        std::fs::remove_dir_all(&scratch)
            .map_err(|e| format!("remove {}: {e}", scratch.display()))?;
    }
    std::fs::create_dir_all(scratch.join("work"))
        .map_err(|e| format!("create {}: {e}", scratch.display()))?;
    copy(&similar, &scratch.join("similar.py"))?;
    copy(&root.join(DRIVER), &scratch.join("driver.py"))?;
    write_jsonl(&scratch.join("corpus.jsonl"), &corpus)?;
    write_jsonl(&scratch.join("queries.jsonl"), &queries)?;
    let out = root.join(GOLDENS_DIR);
    std::fs::create_dir_all(&out).map_err(|e| format!("create {}: {e}", out.display()))?;

    run_cmd(
        Command::new(&python)
            .arg("-I")
            .arg(scratch.join("driver.py"))
            .arg("--similar")
            .arg(scratch.join("similar.py"))
            .args(["--sha256", SIMILAR_PY_SHA256])
            .args(["--reference-commit", crate::REFERENCE_COMMIT])
            .arg("--corpus")
            .arg(scratch.join("corpus.jsonl"))
            .arg("--queries")
            .arg(scratch.join("queries.jsonl"))
            .arg("--work")
            .arg(scratch.join("work"))
            .arg("--out")
            .arg(&out)
            .current_dir(&scratch),
    )?;
    if keep_scratch {
        eprintln!("gen-goldens: scratch kept at {}", scratch.display());
    } else {
        std::fs::remove_dir_all(&scratch)
            .map_err(|e| format!("remove {}: {e}", scratch.display()))?;
    }
    eprintln!("gen-goldens: wrote {}", out.display());
    Ok(())
}

/// Create the venv once (`python3 -m venv`, interpreter from `AROMA_PYTHON`), then make
/// sure the pinned wheels are installed (a no-op when they already are).
fn ensure_venv(venv: &Path) -> Result<PathBuf, String> {
    let python = if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    };
    if !python.is_file() {
        let base = std::env::var("AROMA_PYTHON").unwrap_or_else(|_| "python3".into());
        run_cmd(Command::new(base).args(["-I", "-m", "venv"]).arg(venv))?;
    }
    run_cmd(
        Command::new(&python)
            .args(["-I", "-m", "pip", "install", "--disable-pip-version-check"])
            .args(["--quiet", "--only-binary=:all:"])
            .args(PY_PACKAGES),
    )?;
    Ok(python)
}

fn run_cmd(cmd: &mut Command) -> Result<(), String> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let status = cmd
        .status()
        .map_err(|e| format!("cannot start {program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with {status}"))
    }
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| format!("copy {} -> {}: {e}", from.display(), to.display()))
}

fn write_jsonl(path: &Path, records: &[Value]) -> Result<(), String> {
    let mut text = String::new();
    for r in records {
        text.push_str(&r.to_string());
        text.push('\n');
    }
    std::fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))
}

/// Corpus records (one per input file, in file-name order: record i is unit i) and query
/// records (every applicable mutation of every input).
fn build_records(inputs: &Path) -> Result<(Vec<Value>, Vec<Value>), String> {
    let entries =
        std::fs::read_dir(inputs).map_err(|e| format!("read {}: {e}", inputs.display()))?;
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "java"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("no .java inputs in {}", inputs.display()));
    }
    let mut corpus = Vec::new();
    let mut queries = Vec::new();
    for (base, file) in files.iter().enumerate() {
        let name = file
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| format!("{}: non-UTF-8 file name", file.display()))?;
        let path = format!("{INPUTS_DIR}/{name}.java");
        let src =
            std::fs::read_to_string(file).map_err(|e| format!("read {}: {e}", file.display()))?;
        let adapted = adapt(&src).map_err(|e| format!("{path}: {e}"))?;
        corpus.push(record(&path, &src, &adapted));
        for (mutation, source, a) in mutations(&src).map_err(|e| format!("{path}: {e}"))? {
            let mut r = record(&path, &source, &a);
            r["id"] = json!(format!("{name}/{mutation}"));
            r["base"] = json!(base);
            r["mutation"] = json!(mutation);
            queries.push(r);
        }
    }
    Ok((corpus, queries))
}

fn record(path: &str, src: &str, a: &Adapted) -> Value {
    json!({
        "path": path,
        "beginline": a.lines.start,
        "endline": a.lines.end - 1,
        "source": src,
        "ast": spt_to_reference_json(&a.spt),
    })
}

/// One method run through the Java adapter in Compat mode, plus the byte spans of the
/// top-level statements of its body (the mutation targets).
#[derive(Debug)]
struct Adapted {
    spt: Spt,
    stmts: Vec<Range<usize>>,
    lines: Range<u32>,
}

/// The Compat-mode Java pipeline the index build uses (T15): parse, repository types as
/// globals, unit extraction, normalize + classify + collapse. T16's `adapt` mirrors this.
fn adapt(src: &str) -> Result<Adapted, String> {
    let spec = spec_for(Lang::Java).unwrap();
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
    if unit.damage > 0.0 {
        return Err(format!("parse damage {:.3}", unit.damage));
    }
    let spt = unit_spt(unit, spec, &globals, VarMode::Compat).ok_or("method body has no tokens")?;
    Ok(Adapted {
        spt,
        stmts: statement_spans(&unit.body),
        lines: unit.lines.clone(),
    })
}

/// Byte spans of the statement nodes directly under the body block (`{`/`}` are tokens).
fn statement_spans(body: &KRaw) -> Vec<Range<usize>> {
    match body {
        KRaw::Node { children, .. } => children
            .iter()
            .filter(|c| matches!(c, KRaw::Node { .. }))
            .filter_map(span)
            .collect(),
        KRaw::Token { .. } => Vec::new(),
    }
}

fn span(k: &KRaw) -> Option<Range<usize>> {
    match k {
        KRaw::Token { bytes, .. } => Some(bytes.start as usize..bytes.end as usize),
        KRaw::Node { children, .. } => children
            .iter()
            .filter_map(span)
            .reduce(|a, b| a.start.min(b.start)..a.end.max(b.end)),
    }
}

/// Source-level mutation primitives. Each applies to the current text and its fresh
/// adapter output; `None` when it does not apply (too few statements, nothing to rename).
#[derive(Clone, Copy, Debug)]
enum Op {
    /// Rename every lower-case `#VAR` leaf of the body to `ren<k>` (k = first-occurrence
    /// order). Upper-case vars are left alone: similar.py treats them as globals.
    Rename,
    Drop(usize),
    /// Drop the last statement (only with >= 4 statements, so it differs from drop0..2).
    DropLast,
    /// Swap statements i and i+1.
    Swap(usize),
    /// Swap the last two statements (only with >= 4 statements).
    SwapLast,
    /// Keep the first k statements (a partial-snippet query).
    Prefix(usize),
}

const MUTATIONS: &[(&str, &[Op])] = &[
    ("id", &[]),
    ("rename", &[Op::Rename]),
    ("drop0", &[Op::Drop(0)]),
    ("drop1", &[Op::Drop(1)]),
    ("drop2", &[Op::Drop(2)]),
    ("droplast", &[Op::DropLast]),
    ("swap01", &[Op::Swap(0)]),
    ("swap12", &[Op::Swap(1)]),
    ("swaplast", &[Op::SwapLast]),
    ("prefix1", &[Op::Prefix(1)]),
    ("prefix2", &[Op::Prefix(2)]),
    ("prefix3", &[Op::Prefix(3)]),
    ("rename+prefix2", &[Op::Rename, Op::Prefix(2)]),
];

/// Every applicable mutation of `src`, in `MUTATIONS` order, without duplicate texts.
fn mutations(src: &str) -> Result<Vec<(&'static str, String, Adapted)>, String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    'mutation: for &(name, ops) in MUTATIONS {
        let mut text = src.to_string();
        let mut adapted = adapt(&text)?;
        for &op in ops {
            let Some(edits) = op_edits(op, &text, &adapted) else {
                continue 'mutation;
            };
            text = apply_edits(&text, edits);
            adapted = adapt(&text).map_err(|e| format!("{name}: {e}"))?;
        }
        if seen.insert(text.clone()) {
            out.push((name, text, adapted));
        }
    }
    Ok(out)
}

fn op_edits(op: Op, src: &str, a: &Adapted) -> Option<Vec<(Range<usize>, String)>> {
    let s = &a.stmts;
    let n = s.len();
    let text = |r: &Range<usize>| src[r.clone()].to_string();
    match op {
        Op::Rename => {
            let mut names: Vec<&str> = Vec::new();
            let mut edits = Vec::new();
            for leaf in &a.spt.leaves {
                let t: &str = &leaf.text;
                let r = leaf.bytes.start as usize..leaf.bytes.end as usize;
                if !leaf.is_var
                    || !t.starts_with(|c: char| c.is_ascii_lowercase())
                    || src.get(r.clone()) != Some(t)
                {
                    continue;
                }
                let k = match names.iter().position(|&x| x == t) {
                    Some(k) => k,
                    None => {
                        names.push(t);
                        names.len() - 1
                    }
                };
                edits.push((r, format!("ren{k}")));
            }
            (!edits.is_empty()).then_some(edits)
        }
        Op::Drop(i) => (n >= 2 && i < n).then(|| vec![(s[i].clone(), String::new())]),
        Op::DropLast => (n >= 4).then(|| vec![(s[n - 1].clone(), String::new())]),
        Op::Swap(i) => (i + 1 < n).then(|| {
            vec![
                (s[i].clone(), text(&s[i + 1])),
                (s[i + 1].clone(), text(&s[i])),
            ]
        }),
        Op::SwapLast => (n >= 4).then(|| {
            vec![
                (s[n - 2].clone(), text(&s[n - 1])),
                (s[n - 1].clone(), text(&s[n - 2])),
            ]
        }),
        Op::Prefix(k) => (n > k).then(|| vec![(s[k].start..s[n - 1].end, String::new())]),
    }
}

/// Apply non-overlapping edits back to front so earlier offsets stay valid.
fn apply_edits(src: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
    edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    let mut out = src.to_string();
    for (r, replacement) in edits {
        out.replace_range(r, &replacement);
    }
    out
}

/// An `Spt` in similar.py's record format (`record["ast"]`, the shape
/// `Spt::from_reference_json` reads): a node is `[label, child, ...]`; every token is an
/// object with `token`, `leading`, `trailing` and `line`; leaves add `"leaf": true` and
/// local variables `"var": true` (similar.py tests both keys by presence, so they are
/// omitted rather than set to false). `leading`/`trailing` are a fixed " " / "" because
/// only the reference's printers read them; a keyword takes the line of the nearest
/// preceding leaf (the first leaf's line before any leaf).
fn spt_to_reference_json(spt: &Spt) -> Value {
    let mut line = spt.leaves.first().map_or(0, |l| l.line);
    node_json(spt, spt.root, &mut line)
}

fn node_json(spt: &Spt, n: NodeId, line: &mut u32) -> Value {
    let node = &spt.nodes[n as usize];
    let mut arr = Vec::with_capacity(node.children.len() + 1);
    arr.push(Value::String(node.label.to_string()));
    for child in &node.children {
        arr.push(match child {
            Elem::Keyword(k) => token(k, *line, None),
            Elem::Leaf(l) => {
                let leaf = &spt.leaves[*l as usize];
                *line = leaf.line;
                token(&leaf.text, leaf.line, Some(leaf.is_var))
            }
            Elem::Tree(t) => node_json(spt, *t, line),
        });
    }
    Value::Array(arr)
}

fn token(text: &str, line: u32, leaf: Option<bool>) -> Value {
    let mut m = Map::new();
    m.insert("token".into(), Value::from(text));
    m.insert("leading".into(), Value::from(" "));
    m.insert("trailing".into(), Value::from(""));
    m.insert("line".into(), Value::from(line));
    if let Some(is_var) = leaf {
        m.insert("leaf".into(), Value::Bool(true));
        if is_var {
            m.insert("var".into(), Value::Bool(true));
        }
    }
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aroma_core::spt::{Leaf, Node, Raw};
    use std::collections::HashMap;

    const SMALL: &str = "class A {\n    int f(int n) {\n        int a = n;\n        int b = a + 1;\n        a = b * 2;\n        return a + b;\n    }\n}\n";

    type Shape = (Vec<Node>, Vec<(String, bool, u32)>);

    fn shape(spt: &Spt) -> Shape {
        let leaves = spt
            .leaves
            .iter()
            .map(|l| (l.text.to_string(), l.is_var, l.line))
            .collect();
        (spt.nodes.clone(), leaves)
    }

    fn leaf(text: &str, is_var: bool, line: u32) -> Raw {
        Raw::Leaf(Leaf {
            text: text.into(),
            is_var,
            binding: None,
            bytes: 0..0,
            line,
        })
    }

    fn kw(text: &str) -> Raw {
        Raw::Keyword(text.into())
    }

    #[test]
    fn spt_json_has_similar_py_shape_and_round_trips() {
        // { x = Y ; f ( ) ; }  ->  ["{##}", "{", ["#;", ["#=#", x, =, Y], ;], ["#;", ["#()", f, (, )], ;], "}"]
        let spt = Spt::build(Raw::List(vec![
            kw("{"),
            Raw::List(vec![
                Raw::List(vec![leaf("x", true, 2), kw("="), leaf("Y", true, 2)]),
                kw(";"),
            ]),
            Raw::List(vec![
                Raw::List(vec![leaf("f", false, 3), kw("("), kw(")")]),
                kw(";"),
            ]),
            kw("}"),
        ]))
        .unwrap();
        let v = spt_to_reference_json(&spt);
        let t =
            |s: &str, line: u32| json!({"token": s, "leading": " ", "trailing": "", "line": line});
        let l = |s: &str, line: u32| json!({"token": s, "leading": " ", "trailing": "", "line": line, "leaf": true});
        let var = |s: &str, line: u32| json!({"token": s, "leading": " ", "trailing": "", "line": line, "leaf": true, "var": true});
        let expected = json!([
            "{##}",
            t("{", 2),
            [
                "#;",
                ["#=#", var("x", 2), t("=", 2), var("Y", 2)],
                t(";", 2)
            ],
            ["#;", ["#()", l("f", 3), t("(", 3), t(")", 3)], t(";", 3)],
            t("}", 3)
        ]);
        assert_eq!(v, expected);
        let back = Spt::from_reference_json(&v).unwrap();
        assert_eq!(shape(&back), shape(&spt));
    }

    #[test]
    fn statements_and_mutations_of_a_small_method() {
        let a = adapt(SMALL).unwrap();
        let stmts: Vec<&str> = a.stmts.iter().map(|r| &SMALL[r.clone()]).collect();
        assert_eq!(
            stmts,
            [
                "int a = n;",
                "int b = a + 1;",
                "a = b * 2;",
                "return a + b;"
            ]
        );
        assert_eq!(a.lines, 2..8);

        // With 4 statements prefix3 produces the droplast text, so it is deduplicated.
        let m = mutations(SMALL).unwrap();
        let names: Vec<&str> = m.iter().map(|(n, _, _)| *n).collect();
        assert_eq!(
            names,
            [
                "id",
                "rename",
                "drop0",
                "drop1",
                "drop2",
                "droplast",
                "swap01",
                "swap12",
                "swaplast",
                "prefix1",
                "prefix2",
                "rename+prefix2"
            ]
        );
        let src = |name: &str| m.iter().find(|(n, _, _)| *n == name).unwrap().1.as_str();
        let wrap = |body: &str| format!("class A {{\n    int f(int n) {{\n{body}    }}\n}}\n");
        assert_eq!(src("id"), SMALL);
        assert_eq!(
            src("rename"),
            wrap("        int ren0 = ren1;\n        int ren2 = ren0 + 1;\n        ren0 = ren2 * 2;\n        return ren0 + ren2;\n")
        );
        assert_eq!(
            src("drop1"),
            wrap("        int a = n;\n        \n        a = b * 2;\n        return a + b;\n")
        );
        assert_eq!(
            src("swap01"),
            wrap("        int b = a + 1;\n        int a = n;\n        a = b * 2;\n        return a + b;\n")
        );
        assert_eq!(
            src("prefix2"),
            wrap("        int a = n;\n        int b = a + 1;\n        \n")
        );
        assert_eq!(
            src("rename+prefix2"),
            wrap("        int ren0 = ren1;\n        int ren2 = ren0 + 1;\n        \n")
        );
        // Renaming locals never changes the Compat tree shape apart from leaf texts.
        let renamed = &m.iter().find(|(n, _, _)| *n == "rename").unwrap().2;
        assert_eq!(renamed.spt.nodes, a.spt.nodes);
    }

    #[test]
    fn every_input_yields_thirteen_cases() {
        let (corpus, queries) = build_records(&crate::workspace_root().join(INPUTS_DIR)).unwrap();
        assert_eq!(corpus.len(), 40);
        assert_eq!(queries.len(), EXPECTED_CASES);
        let ids: HashSet<&str> = queries.iter().map(|q| q["id"].as_str().unwrap()).collect();
        assert_eq!(ids.len(), EXPECTED_CASES);
        for q in &queries {
            let ast = &q["ast"];
            assert!(Spt::from_reference_json(ast).is_some(), "{}", q["id"]);
        }
    }

    #[test]
    fn checked_in_goldens_are_consistent_and_fresh() {
        let dir = crate::workspace_root().join(GOLDENS_DIR);
        let read = |name: &str| -> Value {
            let path = dir.join(name);
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
                panic!(
                    "{}: {e}; run `cargo run -p xtask -- gen-goldens`",
                    path.display()
                )
            });
            serde_json::from_str(&text).unwrap()
        };
        let meta = read("meta.json");
        let corpus = read("corpus.json");
        let cases = read("cases.json");
        let corpus = corpus.as_array().unwrap();
        let cases = cases.as_array().unwrap();
        assert_eq!(meta["reference_commit"], crate::REFERENCE_COMMIT);
        assert_eq!(meta["similar_py_sha256"], SIMILAR_PY_SHA256);
        assert_eq!(meta["n_corpus"], 40);
        assert_eq!(corpus.len(), 40);
        let skipped = meta["skipped"].as_array().unwrap().len();
        assert_eq!(meta["n_cases"], cases.len());
        assert_eq!(cases.len() + skipped, EXPECTED_CASES);
        assert!(
            cases.len() >= 500,
            "{skipped} cases skipped: {}",
            meta["skipped"]
        );
        let with_clusters = cases
            .iter()
            .filter(|c| !c["clusters"].as_array().unwrap().is_empty())
            .count();
        assert_eq!(meta["n_cases_with_clusters"], with_clusters);
        assert!(
            with_clusters >= 40,
            "only {with_clusters} cases form clusters"
        );

        // Fresh: the checked-in sources are exactly what the current inputs and adapter produce.
        let (want_corpus, want_queries) =
            build_records(&crate::workspace_root().join(INPUTS_DIR)).unwrap();
        for (i, (unit, want)) in corpus.iter().zip(&want_corpus).enumerate() {
            assert_eq!(unit["unit"], i);
            assert_eq!(unit["path"], want["path"]);
            assert_eq!(unit["source"], want["source"]);
            assert_eq!(
                unit["ast"],
                strip_layout(&want["ast"]),
                "{}: stale goldens",
                unit["path"]
            );
        }
        let sources: HashMap<&str, &Value> = want_queries
            .iter()
            .map(|q| (q["id"].as_str().unwrap(), &q["source"]))
            .collect();
        let n_leaves: Vec<usize> = corpus
            .iter()
            .map(|u| u["n_leaves"].as_u64().unwrap() as usize)
            .collect();

        let mut ids = HashSet::new();
        for case in cases {
            let id = case["id"].as_str().unwrap();
            assert!(ids.insert(id), "duplicate case {id}");
            assert_eq!(
                sources.get(id).copied(),
                Some(&case["source"]),
                "{id}: stale goldens"
            );
            let hits: Vec<(u64, u64)> = case["hits"]
                .as_array()
                .unwrap()
                .iter()
                .map(|h| (h[0].as_u64().unwrap(), h[1].as_u64().unwrap()))
                .collect();
            assert!(
                hits.windows(2).all(|w| {
                    (std::cmp::Reverse(w[0].1), w[0].0) < (std::cmp::Reverse(w[1].1), w[1].0)
                }),
                "{id}: hits are not in (overlap desc, unit asc) order"
            );
            let prunes = case["prunes"].as_array().unwrap();
            assert_eq!(prunes.len(), hits.len(), "{id}");
            let mut expected: Vec<(u64, f64)> = Vec::new();
            for (p, h) in prunes.iter().zip(&hits) {
                let unit = p["unit"].as_u64().unwrap();
                assert_eq!(unit, h.0, "{id}");
                assert_eq!(
                    p["kept"].as_str().unwrap().len(),
                    n_leaves[unit as usize],
                    "{id}"
                );
                let score = p["score"].as_f64().unwrap();
                if score > 0.65 {
                    expected.push((unit, score));
                }
            }
            expected.sort_by(|a, b| b.1.total_cmp(&a.1));
            let candidates: Vec<(u64, f64)> = case["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| (c[0].as_u64().unwrap(), c[1].as_f64().unwrap()))
                .collect();
            assert_eq!(
                candidates, expected,
                "{id}: candidates are not the stable-sorted prunes > 0.65"
            );
            for c in case["clusters"].as_array().unwrap() {
                let members: Vec<u64> = c["members"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|m| m.as_u64().unwrap())
                    .collect();
                assert!(members.len() >= 2, "{id}");
                assert!(
                    members.iter().all(|m| candidates.iter().any(|c| c.0 == *m)),
                    "{id}"
                );
                assert_eq!(
                    c["kept"].as_str().unwrap().len(),
                    n_leaves[members[0] as usize],
                    "{id}"
                );
            }
        }
    }

    /// driver.py drops `leading`/`trailing` from the trees it writes.
    fn strip_layout(v: &Value) -> Value {
        match v {
            Value::Array(a) => Value::Array(a.iter().map(strip_layout).collect()),
            Value::Object(o) => Value::Object(
                o.iter()
                    .filter(|(k, _)| *k != "leading" && *k != "trailing")
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
}
