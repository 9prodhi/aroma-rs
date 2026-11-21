//! T17: end-to-end tests of the `aroma` binary on temporary repositories: every
//! command, JSON on stdout, logs on stderr, exit codes 0/2/3/4.

use aroma_core::store::lock::WriterLock;
use assert_cmd::Command;
use std::fs;
use std::path::Path;
use std::time::Duration;

const FILE_UTIL: &str = r#"package demo;

import java.io.BufferedReader;
import java.io.FileReader;
import java.io.IOException;
import java.util.ArrayList;
import java.util.List;

public class FileUtil {
    public static List<String> readLines(String path) throws IOException {
        List<String> lines = new ArrayList<>();
        BufferedReader reader = new BufferedReader(new FileReader(path));
        String line;
        while ((line = reader.readLine()) != null) {
            lines.add(line);
        }
        reader.close();
        return lines;
    }

    public static int countLines(String path) throws IOException {
        return readLines(path).size();
    }
}
"#;

const CONFIG_LOADER: &str = r#"package demo;

import java.io.BufferedReader;
import java.io.FileReader;
import java.io.IOException;
import java.util.ArrayList;
import java.util.List;

public class ConfigLoader {
    public List<String> loadEntries(String file) throws IOException {
        List<String> entries = new ArrayList<>();
        BufferedReader in = new BufferedReader(new FileReader(file));
        String entry;
        while ((entry = in.readLine()) != null) {
            entries.add(entry);
        }
        in.close();
        return entries;
    }

    public boolean isEnabled(String key) {
        return key != null && key.startsWith("enable.");
    }
}
"#;

const MATHS: &str = r#"package demo;

public class Maths {
    public static int gcd(int a, int b) {
        while (b != 0) {
            int t = a % b;
            a = b;
            b = t;
        }
        return a;
    }

    public static int sum(int[] xs) {
        int total = 0;
        for (int x : xs) {
            total += x;
        }
        return total;
    }
}
"#;

const ACCOUNT_SERVICE: &str = r#"public with sharing class AccountService {
    public static void closeAccounts(List<Account> accounts) {
        List<Account> toUpdate = new List<Account>();
        for (Account acc : accounts) {
            acc.Description = 'Closed';
            toUpdate.add(acc);
        }
        update toUpdate;
    }
}
"#;

const CONTACT_SERVICE: &str = r#"public with sharing class ContactService {
    public static void archiveContacts(List<Contact> contacts) {
        List<Contact> changed = new List<Contact>();
        for (Contact c : contacts) {
            c.Description = 'Archived';
            changed.add(c);
        }
        update changed;
    }
}
"#;

/// 3 Java files (6 units) + 2 Apex files (2 units).
const FIXTURE: &[(&str, &str)] = &[
    ("src/main/java/demo/FileUtil.java", FILE_UTIL),
    ("src/main/java/demo/ConfigLoader.java", CONFIG_LOADER),
    ("src/main/java/demo/Maths.java", MATHS),
    (
        "force-app/main/default/classes/AccountService.cls",
        ACCOUNT_SERVICE,
    ),
    (
        "force-app/main/default/classes/ContactService.cls",
        CONTACT_SERVICE,
    ),
];

const JAVA_QUERY: &str = "BufferedReader br = new BufferedReader(new FileReader(name));\nString l;\nwhile ((l = br.readLine()) != null) {\n    out.add(l);\n}\n";

fn aroma() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_aroma"));
    cmd.env_remove("RUST_LOG");
    cmd
}

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (rel, text) in FIXTURE {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    dir
}

fn indexed() -> tempfile::TempDir {
    let repo = fixture();
    aroma().arg("index").arg(repo.path()).assert().code(0);
    repo
}

fn stdout(a: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(a.get_output().stdout.clone()).unwrap()
}

fn stderr(a: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(a.get_output().stderr.clone()).unwrap()
}

fn json(a: &assert_cmd::assert::Assert) -> serde_json::Value {
    serde_json::from_slice(&a.get_output().stdout).expect("stdout is one JSON document")
}

fn root_arg(p: &Path) -> String {
    p.to_str().unwrap().to_string()
}

#[test]
fn version_and_help_exit_zero() {
    let a = aroma().arg("--version").assert().code(0);
    assert_eq!(stdout(&a), format!("aroma {}\n", env!("CARGO_PKG_VERSION")));
    let a = aroma().arg("--help").assert().code(0);
    for language in [
        "Java",
        "Apex",
        "LWC",
        "Salesforce XML metadata",
        "Aura",
        "Visualforce",
    ] {
        assert!(stdout(&a).contains(language), "help lists {language}");
    }
    for cmd in [
        "index",
        "query",
        "dump-spt",
        "dump-features",
        "feature-diff",
        "inspect-index",
    ] {
        assert!(stdout(&a).contains(cmd), "help lists {cmd}");
    }
    for cmd in ["index", "query"] {
        let a = aroma().args([cmd, "--help"]).assert().code(0);
        for lang in [
            "java", "apex", "lwcjs", "lwchtml", "sfmeta", "aura", "aurajs", "vf", "lwc",
        ] {
            assert!(stdout(&a).contains(lang), "{cmd} help lists {lang}");
        }
        assert!(stdout(&a).contains("Aliases"), "{cmd} help lists aliases");
    }
}

#[test]
fn usage_errors_exit_two() {
    aroma().assert().code(2);
    aroma().arg("frobnicate").assert().code(2);
    let a = aroma()
        .args(["dump-spt", "snippet.unknown"])
        .assert()
        .code(2);
    assert!(String::from_utf8_lossy(&a.get_output().stderr).contains("--lang java|apex|sfmeta"));
    aroma().args(["query", "--lang", "java"]).assert().code(2);
    aroma()
        .args(["query", "--lang", "java", "--stdin", "--file", "q.java"])
        .assert()
        .code(2);
    aroma().args(["query", "--stdin"]).assert().code(2);
    aroma()
        .args(["query", "--stdin", "--lang", "cobol"])
        .assert()
        .code(2);
    aroma()
        .args(["index", "--profile", "fast"])
        .assert()
        .code(2);
}

#[test]
fn index_prints_a_summary_and_reuses_unchanged_files() {
    let repo = fixture();
    let a = aroma().arg("index").arg(repo.path()).assert().code(0);
    assert_eq!(
        stdout(&a),
        "indexed generation 1: 8 units from 5 files (5 parsed, 0 unchanged, 0 too large, 0 failed), 0 units skipped as damaged\n"
    );
    let a = aroma()
        .args(["index", "--json"])
        .arg(repo.path())
        .assert()
        .code(0);
    let v = json(&a);
    assert_eq!(v["generation"], 2);
    assert_eq!(v["files_seen"], 5);
    assert_eq!(v["files_unchanged"], 5);
    assert_eq!(v["units"], 8);
    let a = aroma().arg("index").arg(repo.path()).assert().code(0);
    assert!(
        stdout(&a).ends_with("removed old generations: 1\n"),
        "{}",
        stdout(&a)
    );
}

#[test]
fn index_language_filter_and_profile() {
    let repo = fixture();
    let a = aroma()
        .args([
            "index",
            "--json",
            "--lang",
            "java",
            "--profile",
            "paper",
            "--jobs",
            "1",
        ])
        .arg(repo.path())
        .assert()
        .code(0);
    let v = json(&a);
    assert_eq!(v["files_seen"], 3);
    assert_eq!(v["units"], 6);
    let a = aroma()
        .args(["inspect-index", "--json", "--root", &root_arg(repo.path())])
        .assert()
        .code(0);
    let v = json(&a);
    assert_eq!(v["manifest"]["profile"], "paper");
    assert_eq!(v["stats"]["units_by_lang"]["java"], 6);
    assert!(v["stats"]["units_by_lang"]["apex"].is_null());
}

#[test]
fn index_of_an_empty_directory_fails_with_one() {
    let repo = tempfile::tempdir().unwrap();
    let a = aroma().arg("index").arg(repo.path()).assert().code(1);
    assert!(
        stderr(&a).contains("no supported source files"),
        "{}",
        stderr(&a)
    );
}

#[test]
fn query_from_stdin_prints_json() {
    let repo = indexed();
    let a = aroma()
        .args(["query", "--stdin", "--lang", "java", "--json", "--root"])
        .arg(repo.path())
        .write_stdin(JAVA_QUERY)
        .assert()
        .code(0);
    let v = json(&a);
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["generation"], 1);
    assert_eq!(v["profile"], "reference");
    let mut top2: Vec<String> = v["similar"].as_array().unwrap()[..2]
        .iter()
        .map(|s| s["path"].as_str().unwrap().to_string())
        .collect();
    top2.sort();
    assert_eq!(
        top2,
        [
            "src/main/java/demo/ConfigLoader.java",
            "src/main/java/demo/FileUtil.java"
        ]
    );
    assert_eq!(v["similar"][0]["lines"], serde_json::json!([10, 19]));
    assert_eq!(v["diagnostics"]["wrapper"], "statements");
    assert_eq!(v["diagnostics"]["no_evidence"], false);
    assert!(v["recommendations"].as_array().unwrap().is_empty());
}

#[test]
fn query_from_a_file_prints_text_and_finds_the_index_upwards() {
    let repo = indexed();
    let q = repo.path().join("query.txt");
    fs::write(&q, JAVA_QUERY).unwrap();
    let a = aroma()
        .current_dir(repo.path().join("src/main"))
        .args(["query", "--lang", "java", "--file"])
        .arg(&q)
        .assert()
        .code(0);
    let out = stdout(&a);
    assert!(out.starts_with("#1 src/main/java/demo/"), "{out}");
    assert!(out.contains(":10-19 method score "), "{out}");
    assert!(out.contains("readLine()) != null) {"), "{out}");
    assert!(
        out.contains("\n-- generation 1, profile reference, wrapper statements"),
        "{out}"
    );
}

#[test]
fn query_options_are_passed_through() {
    let repo = indexed();
    let root = root_arg(repo.path());
    let a = aroma()
        .args([
            "query",
            "--stdin",
            "--lang",
            "java",
            "--json",
            "--explain",
            "--experimental-recommend",
            "--top",
            "1",
            "--similar",
            "1",
            "--root",
            &root,
        ])
        .write_stdin(JAVA_QUERY)
        .assert()
        .code(0);
    let v = json(&a);
    assert_eq!(v["similar"].as_array().unwrap().len(), 1);
    assert!(v["similar"][0]["explain"]["jaccard"].as_f64().unwrap() > 0.0);
    assert!(v["recommendations"].as_array().unwrap().len() <= 1);

    let a = aroma()
        .args([
            "query", "--stdin", "--lang", "java", "--json", "--kind", "trigger", "--root", &root,
        ])
        .write_stdin(JAVA_QUERY)
        .assert()
        .code(0);
    assert!(json(&a)["similar"].as_array().unwrap().is_empty());

    let a = aroma()
        .args([
            "query",
            "--stdin",
            "--lang",
            "java",
            "--json",
            "--budget-ms",
            "0",
            "--root",
            &root,
        ])
        .write_stdin(JAVA_QUERY)
        .assert()
        .code(0);
    assert_eq!(json(&a)["diagnostics"]["budget_exhausted"], "search");
}

#[test]
fn no_lexical_turns_the_bm25_channel_off() {
    let repo = indexed();
    let root = root_arg(repo.path());
    let run = |extra: &[&str]| {
        let mut args = vec!["query", "--stdin", "--lang", "java", "--root", &root];
        args.extend_from_slice(extra);
        aroma().args(&args).write_stdin(JAVA_QUERY).assert().code(0)
    };
    let on = json(&run(&["--json"]));
    assert_eq!(on["diagnostics"]["lexical"], true);
    assert!(on["diagnostics"]["query_terms"].as_u64().unwrap() > 0);
    assert!(on["similar"][0]["bm25"].is_number());
    assert!(on["similar"][0]["matched_terms"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t == "readline"));
    let off = json(&run(&["--json", "--no-lexical"]));
    assert_eq!(off["diagnostics"]["lexical"], false);
    assert_eq!(off["diagnostics"]["query_terms"], 0);
    assert!(off["similar"][0]["bm25"].is_null());
    assert!(off["similar"][0]["lexical_rank"].is_null());
    assert_eq!(off["similar"][0]["structural_rank"], 1);

    let text = stdout(&run(&["--explain"]));
    assert!(text.contains("  lexical: structural rank "), "{text}");
    assert!(text.contains(" query terms known"), "{text}");
    let text = stdout(&run(&["--explain", "--no-lexical"]));
    assert!(!text.contains("  lexical: "), "{text}");
    assert!(!text.contains("query terms known"), "{text}");
}

#[test]
fn empty_query_prints_no_results() {
    let repo = indexed();
    let a = aroma()
        .args(["query", "--stdin", "--lang", "java", "--root"])
        .arg(repo.path())
        .write_stdin("")
        .assert()
        .code(0);
    assert!(
        stdout(&a).starts_with("no similar code found\n"),
        "{}",
        stdout(&a)
    );
}

#[test]
fn query_without_an_index_exits_three() {
    let repo = fixture();
    let a = aroma()
        .args(["query", "--stdin", "--lang", "java", "--root"])
        .arg(repo.path())
        .write_stdin(JAVA_QUERY)
        .assert()
        .code(3);
    assert_eq!(
        stderr(&a).matches("aroma index").count(),
        1,
        "{}",
        stderr(&a)
    );
    aroma()
        .args(["inspect-index", "--root"])
        .arg(repo.path())
        .assert()
        .code(3);
}

#[test]
fn incompatible_index_exits_three() {
    let repo = indexed();
    let path = repo.path().join(".aroma/gen-1/manifest.json");
    let mut v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    v["feature_schema"] = serde_json::json!(999);
    fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    let a = aroma()
        .args(["query", "--stdin", "--lang", "java", "--root"])
        .arg(repo.path())
        .write_stdin(JAVA_QUERY)
        .assert()
        .code(3);
    assert!(stderr(&a).contains("incompatible index"), "{}", stderr(&a));
}

#[test]
fn held_writer_lock_exits_four() {
    let repo = fixture();
    let _held = WriterLock::acquire(&repo.path().join(".aroma"), Duration::ZERO).unwrap();
    let a = aroma()
        .args(["index", "--lock-timeout", "0"])
        .arg(repo.path())
        .assert()
        .code(4);
    assert!(
        stderr(&a).contains(&format!("pid {}", std::process::id())),
        "{}",
        stderr(&a)
    );
}

#[test]
fn dump_spt_prints_unit_trees() {
    let repo = fixture();
    let a = aroma()
        .args(["dump-spt", "--lang", "java"])
        .arg(repo.path().join("src/main/java/demo/Maths.java"))
        .assert()
        .code(0);
    let out = stdout(&a);
    assert!(
        out.starts_with("== method lines 4-11 (damage 0.000)\n"),
        "{out}"
    );
    assert!(
        out.contains("== method lines 13-19 (damage 0.000)\n"),
        "{out}"
    );
    assert!(out.contains("  #VAR"), "{out}");
    // Language from the extension; a snippet file falls back to the wrappers.
    let snippet = repo.path().join("snippet.cls");
    fs::write(&snippet, "update records;\n").unwrap();
    let a = aroma().arg("dump-spt").arg(&snippet).assert().code(0);
    assert!(
        stdout(&a).starts_with("== snippet (wrapper statements, damage 0.000)\n"),
        "{}",
        stdout(&a)
    );
    let unknown = repo.path().join("notes.txt");
    fs::write(&unknown, "x").unwrap();
    aroma().arg("dump-spt").arg(&unknown).assert().code(2);
}

#[test]
fn dump_features_counts_and_origins() {
    let repo = fixture();
    let file = repo.path().join("src/main/java/demo/Maths.java");
    let a = aroma().arg("dump-features").arg(&file).assert().code(0);
    let out = stdout(&a);
    assert!(out.starts_with("== method lines 4-11"), "{out}");
    assert!(
        out.lines()
            .filter(|l| !l.starts_with("=="))
            .all(|l| l.trim_start().split_once("  ").is_some()),
        "{out}"
    );
    let a = aroma()
        .args(["dump-features", "--with-origins"])
        .arg(&file)
        .assert()
        .code(0);
    assert!(
        stdout(&a).contains("leaf 0 line 5 \"b\"\n"),
        "{}",
        stdout(&a)
    );
}

#[test]
fn feature_diff_reports_jaccard() {
    let repo = fixture();
    let a_path = repo.path().join("src/main/java/demo/FileUtil.java");
    let b_path = repo.path().join("src/main/java/demo/ConfigLoader.java");
    let a = aroma()
        .arg("feature-diff")
        .arg(&a_path)
        .arg(&a_path)
        .assert()
        .code(0);
    assert!(stdout(&a).starts_with("jaccard 1.000 ("), "{}", stdout(&a));
    let a = aroma()
        .arg("feature-diff")
        .arg(&a_path)
        .arg(&b_path)
        .assert()
        .code(0);
    let out = stdout(&a);
    let first = out.lines().next().unwrap();
    let j: f64 = first
        .strip_prefix("jaccard ")
        .unwrap()
        .split(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(j > 0.0 && j < 1.0, "{first}");
    assert!(out.contains("only in "), "{out}");
}

#[test]
fn inspect_index_prints_manifest_and_counts() {
    let repo = indexed();
    let root = root_arg(repo.path());
    let a = aroma()
        .args(["inspect-index", "--json", "--root", &root])
        .assert()
        .code(0);
    let v = json(&a);
    assert_eq!(v["manifest"]["generation"], 1);
    assert_eq!(v["manifest"]["n_units"], 8);
    assert_eq!(v["stats"]["n_files"], 5);
    assert_eq!(v["stats"]["units_by_lang"]["java"], 6);
    assert_eq!(v["stats"]["units_by_lang"]["apex"], 2);
    assert_eq!(v["stats"]["build"]["files_reparsed"], 5);
    let a = aroma()
        .args(["inspect-index", "--root", &root])
        .assert()
        .code(0);
    let out = stdout(&a);
    assert!(out.starts_with("index generation 1 at "), "{out}");
    assert!(out.contains("units 8 (0 tests) in 5 files"), "{out}");
    assert!(out.contains("integrity ok"), "{out}");
    assert_eq!(v["integrity"], "ok");
}

#[test]
fn inspect_index_detects_corrupt_postings_that_queries_skip() {
    // Queries open postings.bin in O(1); only inspect-index scans the arrays.
    let repo = indexed();
    let root = root_arg(repo.path());
    let aroma_dir = repo.path().join(".aroma");
    let current = std::fs::read_to_string(aroma_dir.join("CURRENT")).unwrap();
    let postings = aroma_dir
        .join(format!("gen-{}", current.trim()))
        .join("postings.bin");
    let mut bytes = std::fs::read(&postings).unwrap();
    bytes[48..56].copy_from_slice(&u64::MAX.to_le_bytes()); // first feature id: unsorted
    std::fs::write(&postings, bytes).unwrap();
    aroma()
        .args(["query", "--stdin", "--lang", "java", "--root", &root])
        .write_stdin(JAVA_QUERY)
        .assert()
        .code(0);
    let a = aroma()
        .args(["inspect-index", "--root", &root])
        .assert()
        .code(3);
    assert!(stderr(&a).contains("postings.bin"), "{}", stderr(&a));
}

#[test]
fn logs_go_to_stderr_and_never_into_json() {
    let repo = fixture();
    let a = aroma()
        .env("RUST_LOG", "debug")
        .args(["index", "--json"])
        .arg(repo.path())
        .assert()
        .code(0);
    assert_eq!(json(&a)["generation"], 1);
    let a = aroma()
        .env("RUST_LOG", "debug")
        .args(["query", "--stdin", "--lang", "java", "--json", "--root"])
        .arg(repo.path())
        .write_stdin(JAVA_QUERY)
        .assert()
        .code(0);
    assert_eq!(json(&a)["schema_version"], 1);
}

#[test]
fn unknown_languages_fail_before_opening_an_index() {
    // Every `Lang` now has an adapter; an unknown name is still a usage error.
    let dir = tempfile::tempdir().unwrap();
    let result = aroma()
        .current_dir(dir.path())
        .args(["query", "--stdin", "--lang", "cobol"])
        .write_stdin("MOVE A TO B.")
        .assert()
        .code(2);
    assert!(!stderr(&result).is_empty());
    aroma()
        .current_dir(dir.path())
        .args(["index", "--lang", "cobol"])
        .assert()
        .code(2);
    assert!(!dir.path().join(".aroma").exists());
}

#[test]
fn component_directory_json_and_text_include_each_matched_part() {
    let repo = indexed();
    let query_dir = repo.path().join("component-query");
    fs::create_dir_all(query_dir.join("__tests__")).unwrap();
    fs::create_dir_all(query_dir.join("nested")).unwrap();
    fs::write(query_dir.join("X.java"), MATHS).unwrap();
    fs::write(query_dir.join("Y.cls"), ACCOUNT_SERVICE).unwrap();
    fs::write(query_dir.join("ignored.txt"), "unroutable").unwrap();
    fs::write(query_dir.join("__tests__/Test.java"), FILE_UTIL).unwrap();
    fs::write(query_dir.join("nested/Nested.java"), FILE_UTIL).unwrap();
    let a = aroma()
        .args(["query", "--component"])
        .arg(&query_dir)
        .args(["--root", &root_arg(repo.path()), "--json"])
        .assert()
        .code(0);
    let v = json(&a);
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["parts"].as_array().unwrap().len(), 2);
    assert_eq!(v["parts"][0]["lang"], "java");
    assert_eq!(v["parts"][1]["lang"], "apex");
    assert!(!v["components"].as_array().unwrap().is_empty());
    assert!(v.get("similar").is_none());
    let a = aroma()
        .args(["query", "--component"])
        .arg(&query_dir)
        .args(["--root", &root_arg(repo.path())])
        .assert()
        .code(0);
    assert!(stdout(&a).contains("score"));
    assert!(stdout(&a).contains("src/main/java/demo"));
    assert!(stdout(&a).contains("force-app/main/default/classes"));
    assert!(stdout(&a).contains("total += x"));
    assert!(stdout(&a).contains("update toUpdate"));
}

#[test]
fn component_directory_without_routable_files_is_usage_error() {
    let repo = indexed();
    let dir = repo.path().join("empty-component");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("notes.txt"), "nothing to query").unwrap();
    let a = aroma()
        .args(["query", "--component"])
        .arg(&dir)
        .args(["--root", &root_arg(repo.path())])
        .assert()
        .code(2);
    assert!(stderr(&a).contains("no routable files"));
    assert!(stderr(&a).contains(&root_arg(&dir)));
}

#[test]
fn repeated_files_infer_each_language_and_return_components() {
    let repo = indexed();
    let a = aroma()
        .args(["query", "--file"])
        .arg(repo.path().join(FIXTURE[0].0))
        .arg("--file")
        .arg(repo.path().join(FIXTURE[3].0))
        .args(["--root", &root_arg(repo.path()), "--json"])
        .assert()
        .code(0);
    let v = json(&a);
    assert_eq!(v["parts"].as_array().unwrap().len(), 2);
    assert_eq!(v["parts"][0]["lang"], "java");
    assert_eq!(v["parts"][1]["lang"], "apex");
    assert!(!v["components"].as_array().unwrap().is_empty());
}

#[test]
fn repeated_files_reject_unroutable_path() {
    let repo = indexed();
    let unknown = repo.path().join("unknown.txt");
    fs::write(&unknown, MATHS).unwrap();
    let a = aroma()
        .args(["query", "--file"])
        .arg(repo.path().join(FIXTURE[0].0))
        .arg("--file")
        .arg(&unknown)
        .args(["--root", &root_arg(repo.path()), "--lang", "java"])
        .assert()
        .code(2);
    assert!(stderr(&a).contains("cannot infer the language"));
    assert!(stderr(&a).contains("unknown.txt"));
}

#[test]
fn component_and_stdin_are_mutually_exclusive() {
    aroma()
        .args(["query", "--component", ".", "--stdin"])
        .assert()
        .code(2);
}

#[test]
fn single_file_keeps_normal_query_json_schema() {
    let repo = indexed();
    let q = repo.path().join("query.txt");
    fs::write(&q, JAVA_QUERY).unwrap();
    let a = aroma()
        .args(["query", "--file"])
        .arg(&q)
        .args(["--lang", "java", "--json", "--root", &root_arg(repo.path())])
        .assert()
        .code(0);
    let v = json(&a);
    assert_eq!(v["schema_version"], 1);
    assert!(v["similar"].is_array());
    assert!(v["recommendations"].is_array());
    assert!(v["diagnostics"].is_object());
    assert!(v.get("components").is_none());
    assert!(v.get("parts").is_none());
    let stdin = aroma()
        .args([
            "query",
            "--stdin",
            "--lang",
            "java",
            "--json",
            "--root",
            &root_arg(repo.path()),
        ])
        .write_stdin(JAVA_QUERY)
        .assert()
        .code(0);
    assert_eq!(v["similar"], json(&stdin)["similar"]);
}

#[test]
fn component_directory_skips_languages_disabled_in_index() {
    let repo = fixture();
    aroma()
        .args(["index", "--lang", "java"])
        .arg(repo.path())
        .assert()
        .code(0);
    let dir = repo.path().join("mixed");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("X.java"), MATHS).unwrap();
    fs::write(dir.join("Y.cls"), ACCOUNT_SERVICE).unwrap();
    let a = aroma()
        .args(["query", "--component"])
        .arg(&dir)
        .args(["--root", &root_arg(repo.path()), "--json"])
        .assert()
        .code(0);
    let v = json(&a);
    assert_eq!(v["parts"].as_array().unwrap().len(), 1);
    assert_eq!(v["parts"][0]["lang"], "java");
}

#[test]
fn lwcjs_index_and_query_explicit_and_alias_find_expected_unit_first() {
    let repo = tempfile::tempdir().unwrap();
    let dir = repo.path().join("force-app/main/default/lwc/card");
    fs::create_dir_all(dir.join("__tests__")).unwrap();
    let query = r#"{
        const selected = this.records.filter(row => row.active);
        const names = selected.map(row => row.Name);
        this.dispatchEvent(new CustomEvent('select', { detail: names }));
        return names.join(',');
    }"#;
    fs::write(
        dir.join("card.js"),
        format!("export default class Card extends LightningElement {{ select() {query} }}"),
    )
    .unwrap();
    fs::write(dir.join("other.js"), "function total(items) { let sum = 0; for (const item of items) { sum += item.amount; } return sum; }").unwrap();
    fs::write(
        dir.join("__tests__/card.test.js"),
        "function testCard() { expect(card.value).toBe(1); }",
    )
    .unwrap();
    aroma()
        .args(["index", "--lang", "lwcjs", "--json"])
        .arg(repo.path())
        .assert()
        .code(0);
    for lang in ["lwcjs", "lwc"] {
        let result = aroma()
            .args(["query", "--lang", lang, "--stdin", "--json", "--root"])
            .arg(repo.path())
            .write_stdin(query)
            .assert()
            .code(0);
        let v = json(&result);
        assert_eq!(
            v["similar"][0]["path"], "force-app/main/default/lwc/card/card.js",
            "{v}"
        );
    }
}

#[test]
fn lwc_html_index_query_alias_repair_and_original_rendering() {
    let repo = tempfile::tempdir().unwrap();
    let dir = repo.path().join("force-app/main/default/lwc/cards");
    fs::create_dir_all(&dir).unwrap();
    let source = "<template>\n<c-account_card title={heading}>\n<lightning-input value={account.Name} onchange={handleChange}></lightning-input>\n<p>Hello {account.Owner.Name}</p>\n</c-account_card>\n</template>\n";
    fs::write(dir.join("cards.html"), source).unwrap();
    let other = repo.path().join("lwc/other");
    fs::create_dir_all(&other).unwrap();
    fs::write(
        other.join("other.html"),
        "<template><span>Other</span></template>",
    )
    .unwrap();
    let a = aroma()
        .args(["index", "--json", "--lang", "lwchtml", "--jobs", "1"])
        .arg(repo.path())
        .assert()
        .code(0);
    assert_eq!(json(&a)["units"], 2);
    let snippets = [source, "<template>\n<c-account_card title={heading}>\n<lightning-input value={account.Name} onchange={handleChange}></lightning-input>\n<p>Hello {account.Owner.Name}</p>"];
    for lang in ["lwchtml", "lwc"] {
        for snippet in snippets {
            let a = aroma()
                .args(["query", "--stdin", "--json", "--lang", lang, "--root"])
                .arg(repo.path())
                .write_stdin(snippet)
                .assert()
                .code(0);
            let v = json(&a);
            assert_eq!(
                v["similar"][0]["path"],
                "force-app/main/default/lwc/cards/cards.html"
            );
            assert!(v["similar"][0]["text"]
                .as_str()
                .unwrap()
                .contains("c-account_card"));
            assert_eq!(v["diagnostics"]["parse_damage"], 0.0);
        }
    }
}

#[test]
fn vf_index_query_half_pasted_and_original_rendering() {
    let repo = tempfile::tempdir().unwrap();
    let dir = repo.path().join("force-app/main/default/pages");
    fs::create_dir_all(&dir).unwrap();
    let source = "<apex:page controller=\"AccountController\">\n<apex:pageBlock title=\"Account\">\n<apex:pageBlockSection>\n<apex:repeat value=\"{!accounts}\" var=\"acc\">\n<apex:outputText value=\"{!acc.Name}\"/>\n</apex:repeat>\n<apex:outputText value=\"{!IF(show, 'yes', 'no')}\"/>\n</apex:pageBlockSection>\n</apex:pageBlock>\n</apex:page>\n";
    fs::write(dir.join("Account.page"), source).unwrap();
    let other = repo.path().join("src/pages");
    fs::create_dir_all(&other).unwrap();
    fs::write(
        other.join("Other.component"),
        "<apex:component><apex:outputText value=\"Other\"/></apex:component>",
    )
    .unwrap();
    let a = aroma()
        .args(["index", "--json", "--lang", "vf", "--jobs", "1"])
        .arg(repo.path())
        .assert()
        .code(0);
    assert_eq!(json(&a)["units"], 2, "{}", stdout(&a));
    let snippets = [source, "<apex:page controller=\"AccountController\">\n<apex:pageBlock title=\"Account\">\n<apex:pageBlockSection>\n<apex:repeat value=\"{!accounts}\" var=\"acc\">\n<apex:outputText value=\"{!acc.Name}\"/>"];
    for snippet in snippets {
        let a = aroma()
            .args(["query", "--stdin", "--json", "--lang", "vf", "--root"])
            .arg(repo.path())
            .write_stdin(snippet)
            .assert()
            .code(0);
        let v = json(&a);
        assert_eq!(
            v["similar"][0]["path"],
            "force-app/main/default/pages/Account.page"
        );
        assert!(v["similar"][0]["text"]
            .as_str()
            .unwrap()
            .contains("AccountController"));
        assert_eq!(v["diagnostics"]["parse_damage"], 0.0);
    }
    // `--kind page` filters to the page units (new VF kind).
    let a = aroma()
        .args([
            "query", "--stdin", "--json", "--lang", "vf", "--kind", "page", "--root",
        ])
        .arg(repo.path())
        .write_stdin(source)
        .assert()
        .code(0);
    let v = json(&a);
    assert_eq!(
        v["similar"][0]["path"],
        "force-app/main/default/pages/Account.page"
    );
}

#[test]
fn aura_index_query_half_pasted_and_original_rendering() {
    let repo = tempfile::tempdir().unwrap();
    let dir = repo.path().join("force-app/main/default/aura/Greeting");
    fs::create_dir_all(&dir).unwrap();
    let source = r#"<aura:component>
<aura:attribute name="name" type="String"/>
<aura:iteration var="acc" indexVar="i" items="{!v.accounts}">
  <aura:if isTrue="{!v.show}">
    <ui:outputText value="{!acc.Name}"/>
  </aura:if>
</aura:iteration>
<ui:button label="Save" press="{!c.save}"/>
</aura:component>
"#;
    fs::write(dir.join("Greeting.cmp"), source).unwrap();
    let other = repo.path().join("src/main/aura/Other");
    fs::create_dir_all(&other).unwrap();
    fs::write(
        other.join("Other.cmp"),
        "<aura:component><ui:outputText value=\"Other\"/></aura:component>",
    )
    .unwrap();
    let a = aroma()
        .args(["index", "--json", "--lang", "aura", "--jobs", "1"])
        .arg(repo.path())
        .assert()
        .code(0);
    assert_eq!(json(&a)["units"], 2, "{}", stdout(&a));
    let snippets = [source, "<aura:component>\n<aura:attribute name=\"name\" type=\"String\"/>\n<aura:iteration var=\"acc\" indexVar=\"i\" items=\"{!v.accounts}\">\n  <aura:if isTrue=\"{!v.show}\">\n    <ui:outputText value=\"{!acc.Name}\"/>\n  </aura:if>"];
    for snippet in snippets {
        let a = aroma()
            .args(["query", "--stdin", "--json", "--lang", "aura", "--root"])
            .arg(repo.path())
            .write_stdin(snippet)
            .assert()
            .code(0);
        let v = json(&a);
        assert_eq!(
            v["similar"][0]["path"],
            "force-app/main/default/aura/Greeting/Greeting.cmp"
        );
        assert!(v["similar"][0]["text"]
            .as_str()
            .unwrap()
            .contains("aura:component"));
        assert_eq!(v["diagnostics"]["parse_damage"], 0.0);
    }
    // `--kind component` filters to the root component units.
    let a = aroma()
        .args([
            "query",
            "--stdin",
            "--json",
            "--lang",
            "aura",
            "--kind",
            "component",
            "--root",
        ])
        .arg(repo.path())
        .write_stdin(source)
        .assert()
        .code(0);
    let v = json(&a);
    assert_eq!(
        v["similar"][0]["path"],
        "force-app/main/default/aura/Greeting/Greeting.cmp"
    );
}

#[test]
fn sfmeta_modes_index_query_json_and_manifest_mismatch() {
    let query = "<formula>Amount__c * 2</formula>";
    for mode in ["lexical", "structural", "default"] {
        let repo = tempfile::tempdir().unwrap();
        fs::write(repo.path().join("Double.field-meta.xml"), "<CustomField>\n<fullName>Double__c</fullName>\n<formula>Amount__c * 2</formula>\n<type>Number</type>\n</CustomField>\n").unwrap();
        fs::write(repo.path().join("Other.field-meta.xml"), "<CustomField>\n<fullName>Other__c</fullName>\n<formula>Count__c + 1</formula>\n<type>Number</type>\n</CustomField>\n").unwrap();
        let mut command = aroma();
        command
            .current_dir(repo.path())
            .args(["index", "--lang", "sfmeta", "--jobs", "1"]);
        if mode != "default" {
            command.args(["--sfmeta-mode", mode]);
        }
        command.assert().success();
        let mode = if mode == "default" {
            "structural"
        } else {
            mode
        };
        let result = aroma()
            .current_dir(repo.path())
            .args(["query", "--lang", "sfmeta", "--stdin", "--json"])
            .write_stdin(query)
            .assert()
            .success();
        let out = json(&result);
        assert_eq!(out["similar"][0]["path"], "Double.field-meta.xml");
        assert_eq!(
            out["diagnostics"]["experimental_lexical_only"],
            mode == "lexical"
        );
        if mode == "lexical" {
            assert_eq!(out["similar"][0]["score"], out["similar"][0]["bm25"]);
            assert!(out["similar"][0]["text"]
                .as_str()
                .unwrap()
                .contains("<type>Number</type>"));
        }
        let current = fs::read_to_string(repo.path().join(".aroma/CURRENT")).unwrap();
        let path = repo
            .path()
            .join(".aroma")
            .join(format!("gen-{}", current.trim()))
            .join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(manifest["sfmeta_mode"], mode);
        manifest["sfmeta_mode"] = "unknown".into();
        fs::write(path, serde_json::to_string(&manifest).unwrap()).unwrap();
        aroma()
            .current_dir(repo.path())
            .args(["query", "--lang", "sfmeta", "--stdin"])
            .write_stdin(query)
            .assert()
            .failure();
    }
}

#[test]
fn sfmeta_explicit_lexical_accepts_unparseable_query() {
    let repo = tempfile::tempdir().unwrap();
    fs::write(repo.path().join("Flag.field-meta.xml"), "<CustomField>\n<fullName>UrgentCustomer__c</fullName>\n<type>Checkbox</type>\n</CustomField>").unwrap();
    aroma()
        .current_dir(repo.path())
        .args([
            "index",
            "--lang",
            "sfmeta",
            "--jobs",
            "1",
            "--sfmeta-mode",
            "lexical",
        ])
        .assert()
        .success();
    let result = aroma()
        .current_dir(repo.path())
        .args(["query", "--lang", "sfmeta", "--stdin", "--json"])
        .write_stdin("</broken><>> UrgentCustomer__c & <")
        .assert()
        .success();
    let out = json(&result);
    assert_eq!(out["similar"][0]["path"], "Flag.field-meta.xml");
    assert_eq!(out["diagnostics"]["experimental_lexical_only"], true);
}

#[test]
fn aurajs_index_and_query_explicit_and_aura_alias() {
    let repo = tempfile::tempdir().unwrap();
    let dir = repo.path().join("force-app/main/default/aura/card");
    fs::create_dir_all(&dir).unwrap();
    let query = "save: function(cmp, event, helper) { const action = cmp.get('c.save'); action.setParams({ value: event.getParam('value') }); $A.enqueueAction(action); }";
    fs::write(dir.join("cardController.js"), format!("({{ {query} }})")).unwrap();
    fs::write(dir.join("ignored.js"), "function other() { return 1; }").unwrap();
    let indexed = aroma()
        .args(["index", "--lang", "aurajs", "--json"])
        .arg(repo.path())
        .assert()
        .code(0);
    assert_eq!(json(&indexed)["units"], 1);
    for lang in ["aurajs", "aura"] {
        let result = aroma()
            .args(["query", "--lang", lang, "--stdin", "--json", "--root"])
            .arg(repo.path())
            .write_stdin(query)
            .assert()
            .code(0);
        let v = json(&result);
        assert_eq!(
            v["similar"][0]["path"],
            "force-app/main/default/aura/card/cardController.js"
        );
        assert_eq!(v["similar"][0]["score"], 1.0);
        assert_eq!(v["similar"][0]["structural_rank"], 1);
    }
}
