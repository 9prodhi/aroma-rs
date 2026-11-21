//! T15: index build (full, incremental, globals change, lock, GC) and the `Index` facade
//! (open, compatibility, query rendering, stale sources) on temporary Java + Apex repos.

use aroma_core::lang::Lang;
use aroma_core::lexical::{term_frequency, term_id, LexicalSource};
use aroma_core::profile::ProfileName;
use aroma_core::store::generation::IndexDir;
use aroma_core::store::lexical::MmapLexical;
use aroma_core::store::lock::WriterLock;
use aroma_core::store::manifest::Manifest;
use aroma_core::store::spt_store::SptStore;
use aroma_core::store::unit_table::UnitTable;
use aroma_core::store::StoreError;
use aroma_core::unit::UnitKind;
use aroma_lang::build::{body_hash, build_index, BuildError, BuildOptions, BuildReport};
use std::fs;
use std::path::{Path, PathBuf};
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

const FILE_UTIL_TEST: &str = r#"package demo;

import java.util.List;

public class FileUtilTest {
    public void readsLines() throws Exception {
        List<String> lines = FileUtil.readLines("build.gradle");
        if (lines.isEmpty()) {
            throw new AssertionError("no lines");
        }
    }
}
"#;

const GENERATED: &str = r#"public class Gen {
    public int one() {
        return 1;
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

const ACCOUNT_SERVICE_TEST: &str = r#"@IsTest
private class AccountServiceTest {
    @IsTest
    static void closesAccounts() {
        Account a = new Account(Name = 'Acme');
        insert a;
        AccountService.closeAccounts(new List<Account>{ a });
        System.assertEquals('Closed', [SELECT Description FROM Account WHERE Id = :a.Id].Description);
    }
}
"#;

const ACCOUNT_TRIGGER: &str = r#"trigger AccountTrigger on Account (before update) {
    for (Account acc : Trigger.new) {
        acc.Description = 'Touched';
    }
}
"#;

const CONTACT_TRIGGER: &str = r#"trigger ContactTrigger on Contact (before insert) {
    for (Contact c : Trigger.new) {
        c.Description = 'New';
    }
}
"#;

/// Supported, not ignored: 4 Java files (7 units) + 4 Apex files (4 units).
/// `generated/` is ignored by `.gitignore`; `README.md` is unsupported.
const FIXTURE: &[(&str, &str)] = &[
    ("src/main/java/demo/FileUtil.java", FILE_UTIL),
    ("src/main/java/demo/ConfigLoader.java", CONFIG_LOADER),
    ("src/main/java/demo/Maths.java", MATHS),
    ("src/test/java/demo/FileUtilTest.java", FILE_UTIL_TEST),
    ("generated/Gen.java", GENERATED),
    (
        "force-app/main/default/classes/AccountService.cls",
        ACCOUNT_SERVICE,
    ),
    (
        "force-app/main/default/classes/ContactService.cls",
        CONTACT_SERVICE,
    ),
    (
        "force-app/main/default/classes/AccountServiceTest.cls",
        ACCOUNT_SERVICE_TEST,
    ),
    (
        "force-app/main/default/triggers/AccountTrigger.trigger",
        ACCOUNT_TRIGGER,
    ),
    (".gitignore", "generated/\n"),
    ("README.md", "# demo\n"),
];

const INDEXED_PATHS: [&str; 8] = [
    "force-app/main/default/classes/AccountService.cls",
    "force-app/main/default/classes/AccountServiceTest.cls",
    "force-app/main/default/classes/ContactService.cls",
    "force-app/main/default/triggers/AccountTrigger.trigger",
    "src/main/java/demo/ConfigLoader.java",
    "src/main/java/demo/FileUtil.java",
    "src/main/java/demo/Maths.java",
    "src/test/java/demo/FileUtilTest.java",
];

/// Files compared byte for byte between generations built from the same inputs.
const DETERMINISTIC_FILES: [&str; 6] = [
    "postings.bin",
    "lexical.bin",
    "units.bin",
    "spt.bin",
    "files.json",
    "globals.json",
];

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (rel, text) in FIXTURE {
        write(dir.path(), rel, text);
    }
    dir
}

fn opts(root: &Path) -> BuildOptions {
    BuildOptions {
        root: root.to_path_buf(),
        langs: vec![Lang::Java, Lang::Apex],
        profile: ProfileName::Reference,
        compat: false,
        lock_timeout: Duration::from_secs(10),
        jobs: Some(2),
    }
}

fn gen_dir(root: &Path, n: u64) -> PathBuf {
    root.join(".aroma").join(format!("gen-{n}"))
}

fn current_gen(root: &Path) -> PathBuf {
    IndexDir::new(root).current().unwrap().unwrap().1
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() == ".aroma" {
            continue;
        }
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn assert_same_files(a: &Path, b: &Path) {
    for name in DETERMINISTIC_FILES {
        assert_eq!(
            fs::read(a.join(name)).unwrap(),
            fs::read(b.join(name)).unwrap(),
            "{name} differs between {} and {}",
            a.display(),
            b.display()
        );
    }
}

/// Full build of a copy of `root`'s sources in a fresh directory, compared with `root`'s
/// current generation.
fn assert_matches_full_build(root: &Path) {
    let fresh = tempfile::tempdir().unwrap();
    copy_tree(root, fresh.path());
    let report = build_index(&opts(fresh.path())).unwrap();
    assert_eq!(report.files_unchanged, 0);
    assert_same_files(&current_gen(root), &current_gen(fresh.path()));
}

fn report(generation: u64, seen: u32, reparsed: u32, unchanged: u32, units: u32) -> BuildReport {
    BuildReport {
        generation,
        files_seen: seen,
        files_reparsed: reparsed,
        files_unchanged: unchanged,
        units,
        ..BuildReport::default()
    }
}

#[test]
fn full_build_indexes_java_and_apex_units() {
    let repo = fixture();
    assert_eq!(
        build_index(&opts(repo.path())).unwrap(),
        report(1, 8, 8, 0, 11)
    );

    let dir = gen_dir(repo.path(), 1);
    for name in [
        "manifest.json",
        "postings.bin",
        "lexical.bin",
        "units.bin",
        "files.json",
        "spt.bin",
        "globals.json",
        "inventory.postcard",
        "build.json",
    ] {
        assert!(dir.join(name).is_file(), "missing {name}");
    }
    let m = Manifest::read(&dir).unwrap();
    assert_eq!(
        (m.generation, m.n_units, m.n_files, m.profile, m.compat),
        (1, 11, 8, ProfileName::Reference, false)
    );
    assert_eq!(m.schema_version, 3);
    let lexical = MmapLexical::open(&dir.join("lexical.bin")).unwrap();
    lexical.verify().unwrap();
    assert_eq!(lexical.n_units(), 11);
    // FileUtil.readLines (unit 4) holds `readLine` once and `reader` several times.
    assert_eq!(term_frequency(&lexical, term_id("readline"), 4), Some(1));
    assert!(term_frequency(&lexical, term_id("reader"), 4).unwrap() >= 2);
    assert_eq!(term_frequency(&lexical, term_id("readline"), 0), None);
    assert_eq!(
        m.adapters.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["apex", "java"]
    );
    let files: Vec<String> =
        serde_json::from_str(&fs::read_to_string(dir.join("files.json")).unwrap()).unwrap();
    assert_eq!(files, INDEXED_PATHS);

    let expected = [
        (INDEXED_PATHS[0], UnitKind::Method, 47..310, 2..10, false),
        (INDEXED_PATHS[1], UnitKind::Method, 47..326, 3..10, true),
        (INDEXED_PATHS[2], UnitKind::Method, 47..305, 2..10, false),
        (INDEXED_PATHS[3], UnitKind::Trigger, 0..134, 1..6, false),
        (INDEXED_PATHS[4], UnitKind::Method, 185..534, 10..20, false),
        (INDEXED_PATHS[4], UnitKind::Method, 540..641, 21..24, false),
        (INDEXED_PATHS[5], UnitKind::Method, 181..538, 10..20, false),
        (INDEXED_PATHS[5], UnitKind::Method, 544..651, 21..24, false),
        (INDEXED_PATHS[6], UnitKind::Method, 40..201, 4..12, false),
        (INDEXED_PATHS[6], UnitKind::Method, 207..352, 13..20, false),
        (INDEXED_PATHS[7], UnitKind::Method, 71..276, 6..12, true),
    ];
    let table = UnitTable::open(&dir).unwrap();
    let spts = SptStore::open(&dir.join("spt.bin")).unwrap();
    assert_eq!(table.n_units(), 11);
    assert_eq!(spts.len(), 11);
    for (i, (path, kind, bytes, lines, is_test)) in expected.iter().enumerate() {
        let id = i as u32;
        let r = table.record(id);
        assert_eq!(r.id, id);
        assert_eq!(table.file_path(r.file), Path::new(path), "unit {id}");
        assert_eq!(
            (r.kind, r.bytes.clone(), r.lines.clone(), r.is_test),
            (*kind, bytes.clone(), lines.clone(), *is_test),
            "unit {id}"
        );
        let spt = spts.get(id).unwrap();
        assert_eq!(r.body_hash, body_hash(&spt), "unit {id}");
        assert_eq!(r.n_leaves as usize, spt.leaves.len(), "unit {id}");
        assert_eq!(
            r.lang,
            if path.ends_with(".java") {
                Lang::Java
            } else {
                Lang::Apex
            }
        );
    }
    // FileUtil.readLines and ConfigLoader.loadEntries differ only in spelling.
    assert_ne!(table.record(4).body_hash, table.record(6).body_hash);
}

#[test]
fn unchanged_rebuild_reuses_every_file_and_is_byte_identical() {
    let repo = fixture();
    build_index(&opts(repo.path())).unwrap();
    assert_eq!(
        build_index(&opts(repo.path())).unwrap(),
        report(2, 8, 0, 8, 11)
    );
    assert_same_files(&gen_dir(repo.path(), 1), &gen_dir(repo.path(), 2));
}

#[test]
fn editing_a_body_reparses_only_that_file() {
    let repo = fixture();
    build_index(&opts(repo.path())).unwrap();
    write(
        repo.path(),
        "src/main/java/demo/Maths.java",
        &MATHS.replace("int t = a % b;", "int t = Math.floorMod(a, b);"),
    );
    assert_eq!(
        build_index(&opts(repo.path())).unwrap(),
        report(2, 8, 1, 7, 11)
    );
    assert_matches_full_build(repo.path());
}

#[test]
fn adding_a_trigger_keeps_globals_and_reparses_one_file() {
    let repo = fixture();
    build_index(&opts(repo.path())).unwrap();
    write(
        repo.path(),
        "force-app/main/default/triggers/ContactTrigger.trigger",
        CONTACT_TRIGGER,
    );
    assert_eq!(
        build_index(&opts(repo.path())).unwrap(),
        report(2, 9, 1, 8, 12)
    );
    assert_matches_full_build(repo.path());
}

#[test]
fn deleting_a_type_reclassifies_every_file_of_that_language() {
    let repo = fixture();
    build_index(&opts(repo.path())).unwrap();
    fs::remove_file(
        repo.path()
            .join("force-app/main/default/classes/ContactService.cls"),
    )
    .unwrap();
    // `contactservice` leaves the Apex globals: the 3 remaining Apex files are re-parsed,
    // the 4 Java files are reused.
    assert_eq!(
        build_index(&opts(repo.path())).unwrap(),
        report(2, 7, 3, 4, 10)
    );
    assert_matches_full_build(repo.path());
}

#[test]
fn renaming_a_file_reparses_only_the_new_path() {
    let repo = fixture();
    build_index(&opts(repo.path())).unwrap();
    fs::rename(
        repo.path().join("src/main/java/demo/Maths.java"),
        repo.path().join("src/main/java/demo/MathUtil.java"),
    )
    .unwrap();
    assert_eq!(
        build_index(&opts(repo.path())).unwrap(),
        report(2, 8, 1, 7, 11)
    );
    let files: Vec<String> = serde_json::from_str(
        &fs::read_to_string(current_gen(repo.path()).join("files.json")).unwrap(),
    )
    .unwrap();
    assert!(files.contains(&"src/main/java/demo/MathUtil.java".to_string()));
    assert!(!files.contains(&"src/main/java/demo/Maths.java".to_string()));
    assert_matches_full_build(repo.path());
}

#[test]
fn mixed_edits_match_a_full_build() {
    let repo = fixture();
    build_index(&opts(repo.path())).unwrap();
    write(
        repo.path(),
        "src/main/java/demo/Maths.java",
        &MATHS.replace("total += x;", "total = total + x;"),
    );
    write(
        repo.path(),
        "force-app/main/default/triggers/ContactTrigger.trigger",
        CONTACT_TRIGGER,
    );
    fs::remove_file(repo.path().join("src/test/java/demo/FileUtilTest.java")).unwrap();
    // FileUtilTest declared a Java type: every remaining Java file (3) is re-parsed, plus
    // the new trigger; the 4 old Apex files are reused.
    assert_eq!(
        build_index(&opts(repo.path())).unwrap(),
        report(2, 8, 4, 4, 11)
    );
    assert_matches_full_build(repo.path());
}

#[test]
fn job_count_does_not_change_the_index() {
    let one = fixture();
    let four = fixture();
    let mut o = opts(one.path());
    o.jobs = Some(1);
    build_index(&o).unwrap();
    let mut o = opts(four.path());
    o.jobs = Some(4);
    build_index(&o).unwrap();
    assert_same_files(&gen_dir(one.path(), 1), &gen_dir(four.path(), 1));
}

#[test]
fn compat_or_profile_change_forces_a_full_rebuild() {
    let repo = fixture();
    build_index(&opts(repo.path())).unwrap();
    let mut o = opts(repo.path());
    o.compat = true;
    assert_eq!(build_index(&o).unwrap(), report(2, 8, 8, 0, 11));
    assert!(Manifest::read(&gen_dir(repo.path(), 2)).unwrap().compat);
    let mut o = opts(repo.path());
    o.profile = ProfileName::Paper;
    // The third build garbage-collects generation 1 (GC keeps n and n-1).
    assert_eq!(
        build_index(&o).unwrap(),
        BuildReport {
            gc_removed: vec![1],
            ..report(3, 8, 8, 0, 11)
        }
    );
    let m = Manifest::read(&gen_dir(repo.path(), 3)).unwrap();
    assert_eq!((m.profile, m.compat), (ProfileName::Paper, false));
}

/// Rewrite the current generation's manifest as a schema-1 (pre-lexical.bin) index.
fn downgrade_to_schema_1(root: &Path) {
    let path = current_gen(root).join("manifest.json");
    let mut v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    v["schema_version"] = serde_json::Value::from(1);
    fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
}

#[test]
fn schema_1_index_forces_a_full_rebuild() {
    let repo = fixture();
    build_index(&opts(repo.path())).unwrap();
    downgrade_to_schema_1(repo.path());
    fs::remove_file(gen_dir(repo.path(), 1).join("lexical.bin")).unwrap();
    assert_eq!(
        build_index(&opts(repo.path())).unwrap(),
        report(2, 8, 8, 0, 11)
    );
    assert!(gen_dir(repo.path(), 2).join("lexical.bin").is_file());
    assert_eq!(
        Manifest::read(&gen_dir(repo.path(), 2))
            .unwrap()
            .schema_version,
        3
    );
}

#[test]
fn corrupt_inventory_forces_a_full_rebuild() {
    let repo = fixture();
    build_index(&opts(repo.path())).unwrap();
    fs::write(
        gen_dir(repo.path(), 1).join("inventory.postcard"),
        b"garbage",
    )
    .unwrap();
    assert_eq!(
        build_index(&opts(repo.path())).unwrap(),
        report(2, 8, 8, 0, 11)
    );
    assert_same_files(&gen_dir(repo.path(), 1), &gen_dir(repo.path(), 2));
}

#[test]
fn language_filter_indexes_only_requested_languages() {
    let repo = fixture();
    let mut o = opts(repo.path());
    o.langs = vec![Lang::Java];
    assert_eq!(build_index(&o).unwrap(), report(1, 4, 4, 0, 7));
    let m = Manifest::read(&gen_dir(repo.path(), 1)).unwrap();
    assert_eq!(
        m.adapters.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["java"]
    );
    // An empty list means every supported language.
    let mut o = opts(repo.path());
    o.langs = Vec::new();
    assert_eq!(build_index(&o).unwrap(), report(2, 8, 8, 0, 11));
}

#[test]
fn oversized_files_are_skipped_and_counted() {
    let repo = fixture();
    fs::write(
        repo.path().join("src/main/java/demo/Huge.java"),
        vec![b'/'; 2 * 1024 * 1024 + 1],
    )
    .unwrap();
    let got = build_index(&opts(repo.path())).unwrap();
    assert_eq!(
        got,
        BuildReport {
            files_skipped_large: 1,
            ..report(1, 9, 8, 0, 11)
        }
    );
}

#[test]
fn empty_repository_is_an_error_and_creates_nothing() {
    let repo = tempfile::tempdir().unwrap();
    write(repo.path(), "README.md", "# nothing to index\n");
    match build_index(&opts(repo.path())) {
        Err(BuildError::Empty(p)) => assert_eq!(p, repo.path()),
        other => panic!("expected Empty, got {other:?}"),
    }
    assert!(!repo.path().join(".aroma").exists());
}

#[test]
fn a_held_lock_times_out_with_the_holder_pid() {
    let repo = fixture();
    let held = WriterLock::acquire(&repo.path().join(".aroma"), Duration::ZERO).unwrap();
    let mut o = opts(repo.path());
    o.lock_timeout = Duration::from_millis(100);
    match build_index(&o) {
        Err(BuildError::Store(StoreError::Locked { pid })) => assert_eq!(pid, std::process::id()),
        other => panic!("expected Locked, got {other:?}"),
    }
    drop(held);
    assert_eq!(build_index(&o).unwrap().generation, 1);
}

#[test]
fn gc_keeps_the_current_and_previous_generation() {
    let repo = fixture();
    assert!(build_index(&opts(repo.path()))
        .unwrap()
        .gc_removed
        .is_empty());
    assert!(build_index(&opts(repo.path()))
        .unwrap()
        .gc_removed
        .is_empty());
    assert_eq!(build_index(&opts(repo.path())).unwrap().gc_removed, vec![1]);
    assert!(!gen_dir(repo.path(), 1).exists());
    assert!(gen_dir(repo.path(), 2).is_dir());
    assert!(gen_dir(repo.path(), 3).is_dir());
}

mod facade {
    use super::*;
    use aroma_core::unit::UnitStore;
    use aroma_lang::index::{Index, QueryOptions, QueryOutput, SimilarOut};

    const JAVA_QUERY: &str = "BufferedReader br = new BufferedReader(new FileReader(name));\nString l;\nwhile ((l = br.readLine()) != null) {\n    out.add(l);\n}\n";
    const APEX_QUERY: &str = "for (Contact x : items) {\n    x.Description = 'Done';\n    pending.add(x);\n}\nupdate pending;\n";
    const TEST_QUERY: &str = "List<String> lines = FileUtil.readLines(\"build.gradle\");\nif (lines.isEmpty()) {\n    throw new AssertionError(\"no lines\");\n}\n";

    fn indexed() -> (tempfile::TempDir, Index) {
        let repo = fixture();
        build_index(&opts(repo.path())).unwrap();
        let index = Index::open(repo.path()).unwrap();
        (repo, index)
    }

    fn hit<'a>(out: &'a QueryOutput, path: &str) -> &'a SimilarOut {
        out.similar
            .iter()
            .find(|s| s.path == path)
            .unwrap_or_else(|| panic!("{path} not among {:?}", out.similar))
    }

    #[test]
    fn open_without_an_index_is_not_found() {
        let repo = tempfile::tempdir().unwrap();
        match Index::open(repo.path()) {
            Err(StoreError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            Err(e) => panic!("expected NotFound, got {e}"),
            Ok(_) => panic!("expected NotFound, got an index"),
        }
    }

    #[test]
    fn open_exposes_the_manifest_and_unit_store() {
        let (_repo, index) = indexed();
        let m = index.manifest();
        assert_eq!((m.generation, m.n_units, m.n_files), (1, 11, 8));
        assert_eq!(index.n_units(), 11);
        let trigger = index.record(3);
        assert_eq!(trigger.kind, UnitKind::Trigger);
        assert_eq!(
            index.file_path(trigger.file),
            Path::new("force-app/main/default/triggers/AccountTrigger.trigger")
        );
        assert_eq!(
            index.spt(3).unwrap().leaves.len(),
            trigger.n_leaves as usize
        );
    }

    #[test]
    fn schema_1_index_is_incompatible_and_asks_for_a_rebuild() {
        let repo = fixture();
        build_index(&opts(repo.path())).unwrap();
        downgrade_to_schema_1(repo.path());
        match Index::open(repo.path()) {
            Err(e @ StoreError::Incompatible(_)) => {
                let msg = e.to_string();
                assert!(msg.contains("schema_version: index 1, expected 3"), "{msg}");
                assert!(msg.contains("re-index required"), "{msg}");
            }
            Err(e) => panic!("expected Incompatible, got {e}"),
            Ok(_) => panic!("expected Incompatible, got an index"),
        }
    }

    #[test]
    fn tampered_adapter_version_is_incompatible() {
        let repo = fixture();
        build_index(&opts(repo.path())).unwrap();
        let path = gen_dir(repo.path(), 1).join("manifest.json");
        let mut v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        v["adapters"]["java"] = serde_json::Value::from("java-old");
        fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        match Index::open(repo.path()) {
            Err(StoreError::Incompatible(msg)) => {
                assert!(msg.contains("adapters: index"), "{msg}")
            }
            Err(e) => panic!("expected Incompatible, got {e}"),
            Ok(_) => panic!("expected Incompatible, got an index"),
        }
    }

    #[test]
    fn java_query_finds_both_siblings_with_rendered_text() {
        let (_repo, index) = indexed();
        let out = index.query(JAVA_QUERY, &QueryOptions::new(Lang::Java));
        assert_eq!(
            (out.schema_version, out.generation, out.profile),
            (1, 1, ProfileName::Reference)
        );
        let mut top2: Vec<&str> = out
            .similar
            .iter()
            .take(2)
            .map(|s| s.path.as_str())
            .collect();
        top2.sort();
        assert_eq!(
            top2,
            [
                "src/main/java/demo/ConfigLoader.java",
                "src/main/java/demo/FileUtil.java"
            ]
        );
        let file_util = hit(&out, "src/main/java/demo/FileUtil.java");
        assert_eq!(file_util.lines, [10, 19]);
        assert_eq!(file_util.kind, UnitKind::Method);
        assert!(file_util.score > 0.65);
        assert!(!file_util.stale_source);
        assert!(file_util
            .text
            .contains("while ((line = reader.readLine()) != null) {"));
        assert!(file_util.text.contains("// your code ..."));
        assert!(!file_util.kept_spans.is_empty());
        for [start, end] in &file_util.kept_spans {
            assert!(start < end && *end as usize <= FILE_UTIL.len());
        }
        let loader = hit(&out, "src/main/java/demo/ConfigLoader.java");
        assert_eq!(loader.lines, [10, 19]);
        assert!(loader
            .text
            .contains("while ((entry = in.readLine()) != null) {"));
        assert!(out.similar.iter().all(|s| s.explain.is_none()));
        assert!(out.similar.iter().all(|s| !s.path.starts_with("src/test/")));
        assert!(out.recommendations.is_empty());
        let d = &out.diagnostics;
        assert_eq!(d.wrapper, "statements");
        assert_eq!(d.parse_damage, 0.0);
        assert!(!d.low_confidence && !d.no_evidence);
        assert!(d.engine.query_features > 0);
        assert!(d.engine.known_query_features > 0);
        assert_eq!(d.engine.stale_units, 0);
        assert_eq!(d.engine.budget_exhausted, None);
    }

    #[test]
    fn apex_query_ranks_the_same_sobject_sibling_first() {
        let (_repo, index) = indexed();
        let out = index.query(APEX_QUERY, &QueryOptions::new(Lang::Apex));
        assert_eq!(
            out.similar[0].path,
            "force-app/main/default/classes/ContactService.cls"
        );
        assert_eq!(out.similar[0].lines, [2, 9]);
        assert!(out.similar.iter().all(|s| !s.path.contains("Test")));
    }

    #[test]
    fn tests_are_excluded_unless_requested() {
        let (_repo, index) = indexed();
        let mut opts = QueryOptions::new(Lang::Java);
        let without = index.query(TEST_QUERY, &opts);
        assert!(without
            .similar
            .iter()
            .all(|s| !s.path.starts_with("src/test/")));
        opts.include_tests = true;
        let with = index.query(TEST_QUERY, &opts);
        assert_eq!(with.similar[0].path, "src/test/java/demo/FileUtilTest.java");
        assert_eq!(with.similar[0].lines, [6, 11]);
    }

    #[test]
    fn kind_filter_keeps_only_that_kind() {
        let (_repo, index) = indexed();
        let mut opts = QueryOptions::new(Lang::Apex);
        opts.kind = Some(UnitKind::Trigger);
        let out = index.query(APEX_QUERY, &opts);
        assert!(out.similar.iter().all(|s| s.kind == UnitKind::Trigger));
        opts.lang = Lang::Java;
        assert!(index.query(JAVA_QUERY, &opts).similar.is_empty());
    }

    #[test]
    fn edited_source_falls_back_to_stored_tokens() {
        let (repo, index) = indexed();
        write(
            repo.path(),
            "src/main/java/demo/FileUtil.java",
            &format!("// edited\n{FILE_UTIL}"),
        );
        let out = index.query(JAVA_QUERY, &QueryOptions::new(Lang::Java));
        let stale = hit(&out, "src/main/java/demo/FileUtil.java");
        assert!(stale.stale_source);
        assert!(!stale.text.contains('\n'), "{}", stale.text);
        assert!(stale.text.contains("readLine"));
        assert!(!hit(&out, "src/main/java/demo/ConfigLoader.java").stale_source);
        // Every listed unit of the edited file is stale (with the lexical channel,
        // FileUtil.countLines is listed too: it calls `readLines`).
        let edited: Vec<_> = out
            .similar
            .iter()
            .filter(|s| s.path == "src/main/java/demo/FileUtil.java")
            .collect();
        assert!(edited.iter().all(|s| s.stale_source));
        assert_eq!(out.diagnostics.engine.stale_units, edited.len() as u32);
        let mut opts = QueryOptions::new(Lang::Java);
        opts.lexical = false;
        let structural = index.query(JAVA_QUERY, &opts);
        assert_eq!(structural.diagnostics.engine.stale_units, 1);
    }

    #[test]
    fn explain_is_attached_only_on_request() {
        let (_repo, index) = indexed();
        let mut opts = QueryOptions::new(Lang::Java);
        opts.explain = true;
        let out = index.query(JAVA_QUERY, &opts);
        assert!(!out.similar.is_empty());
        for s in &out.similar {
            let e = s.explain.as_ref().expect("explain requested");
            assert!(!e.matched.is_empty());
            assert!(e.jaccard > 0.0);
        }
    }

    #[test]
    fn recommendations_need_the_flag_and_the_gate() {
        let (_repo, index) = indexed();
        let mut opts = QueryOptions::new(Lang::Java);
        assert!(index.query(JAVA_QUERY, &opts).recommendations.is_empty());
        opts.experimental_recommend = true;
        let out = index.query(JAVA_QUERY, &opts);
        // Only two independent siblings exist; the D27 gate needs three.
        assert!(out
            .recommendations
            .iter()
            .all(|r| r.independent_support >= 3));
    }

    #[test]
    fn empty_snippet_has_no_evidence() {
        let (_repo, index) = indexed();
        let out = index.query("", &QueryOptions::new(Lang::Java));
        assert!(out.similar.is_empty() && out.recommendations.is_empty());
        let d = &out.diagnostics;
        assert!(d.no_evidence);
        assert_eq!(d.wrapper, "none");
        assert_eq!(d.parse_damage, 0.0);
        assert_eq!(d.engine.query_features, 0);
    }

    #[test]
    fn zero_budget_stops_after_search() {
        let (_repo, index) = indexed();
        let mut opts = QueryOptions::new(Lang::Java);
        opts.budget = Some(Duration::ZERO);
        let out = index.query(JAVA_QUERY, &opts);
        assert!(out.similar.is_empty());
        assert_eq!(out.diagnostics.engine.budget_exhausted, Some("search"));
    }

    #[test]
    fn output_serializes_with_flattened_engine_diagnostics() {
        let (_repo, index) = indexed();
        let out = index.query(JAVA_QUERY, &QueryOptions::new(Lang::Java));
        let v = serde_json::to_value(&out).unwrap();
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["generation"], 1);
        assert_eq!(v["profile"], "reference");
        assert_eq!(v["similar"][0]["kind"], "method");
        assert!(v["similar"][0]["explain"].is_null());
        assert_eq!(v["diagnostics"]["wrapper"], "statements");
        assert_eq!(v["diagnostics"]["stale_units"], 0);
        assert!(v["diagnostics"]["budget_exhausted"].is_null());
        assert!(v["diagnostics"]["query_features"].as_u64().unwrap() > 0);
    }

    #[test]
    fn stats_count_units_by_language_and_kind() {
        let (_repo, index) = indexed();
        let s = index.stats();
        assert_eq!(
            (s.generation, s.n_files, s.n_units, s.test_units),
            (1, 8, 11, 2)
        );
        assert_eq!(
            s.units_by_lang.into_iter().collect::<Vec<_>>(),
            vec![("apex".to_string(), 4), ("java".to_string(), 7)]
        );
        assert_eq!(
            s.units_by_kind.into_iter().collect::<Vec<_>>(),
            vec![("method".to_string(), 10), ("trigger".to_string(), 1)]
        );
        assert!(s.n_features > 0 && s.n_postings >= s.n_features);
        assert_eq!(s.build.unwrap()["units"], 11);
    }
}
