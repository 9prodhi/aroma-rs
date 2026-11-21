//! T23 end to end: the BM25 identifier channel through `build_index` + `Index::query`
//! (a snippet whose structure matches other methods but whose rare identifiers point
//! at the right one), `QueryOptions::lexical`, the `--compat` index (always off) and the
//! additive JSON fields.

use aroma_core::lang::Lang;
use aroma_core::profile::ProfileName;
use aroma_lang::build::{build_index, BuildOptions};
use aroma_lang::index::{Index, QueryOptions, QueryOutput};
use std::fs;
use std::time::Duration;

const TARGET: &str = "src/main/java/shop/Ledger.java";

/// Four methods with the snippet's shape (load a list, loop, call a sink) but other
/// names, and the target: a different shape that uses the snippet's rare identifiers.
/// Four methods with the snippet's shape (load a list, count while looping over it,
/// log the count) but other names, and the target: a different shape that uses the
/// snippet's rare identifiers.
const FILES: [(&str, &str); 5] = [
    (
        "src/main/java/shop/Mailer.java",
        "package shop;\n\nclass Mailer {\n    void sendAll(Outbox outbox, Smtp smtp) {\n        List<Mail> mails = outbox.drainQueued();\n        int sent = 0;\n        for (Mail m : mails) {\n            sent++;\n            smtp.deliver(m);\n        }\n        log.info(sent);\n    }\n}\n",
    ),
    (
        "src/main/java/shop/Indexer.java",
        "package shop;\n\nclass Indexer {\n    void indexAll(Catalog catalog, Search search) {\n        List<Product> products = catalog.listActive();\n        int indexed = 0;\n        for (Product p : products) {\n            indexed++;\n            search.upsert(p);\n        }\n        log.info(indexed);\n    }\n}\n",
    ),
    (
        "src/main/java/shop/Pruner.java",
        "package shop;\n\nclass Pruner {\n    void pruneAll(Cache cache, Clock clock) {\n        List<Entry> entries = cache.expiredEntries();\n        int evicted = 0;\n        for (Entry e : entries) {\n            evicted++;\n            clock.evict(e);\n        }\n        log.info(evicted);\n    }\n}\n",
    ),
    (
        "src/main/java/shop/Notifier.java",
        "package shop;\n\nclass Notifier {\n    void notifyAll(Users users, Push push) {\n        List<User> active = users.loadSubscribed();\n        int pinged = 0;\n        for (User u : active) {\n            pinged++;\n            push.ping(u);\n        }\n        log.info(pinged);\n    }\n}\n",
    ),
    (
        TARGET,
        "package shop;\n\nclass Ledger {\n    int settle(Repository repository, Audit auditTrail) {\n        if (repository.findPending() == null) {\n            throw new IllegalStateException(\"nothing settled\");\n        }\n        return auditTrail.recordCheckpoint(null, true);\n    }\n}\n",
    ),
];

/// Starts like the four look-alikes, but names the target's rare API.
const QUERY: &str = "List<Order> pending = repository.findPending();\nint count = 0;\nfor (Order o : pending) {\n    count++;\n    auditTrail.recordCheckpoint(o);\n}\nlog.info(count);\n";

fn repo(compat: bool) -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().unwrap();
    for (rel, text) in FILES {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    build_index(&BuildOptions {
        root: dir.path().to_path_buf(),
        langs: vec![Lang::Java],
        profile: ProfileName::Reference,
        compat,
        lock_timeout: Duration::from_secs(10),
        jobs: Some(1),
    })
    .unwrap();
    let index = Index::open(dir.path()).unwrap();
    (dir, index)
}

fn paths(out: &QueryOutput) -> Vec<&str> {
    out.similar.iter().map(|s| s.path.as_str()).collect()
}

fn opts(lexical: bool) -> QueryOptions {
    let mut o = QueryOptions::new(Lang::Java);
    o.lexical = lexical;
    o
}

#[test]
fn rare_identifiers_find_the_target_only_with_lexical_on() {
    let (_dir, index) = repo(false);
    assert!(QueryOptions::new(Lang::Java).lexical, "on by default");

    let off = index.query(QUERY, &opts(false));
    assert!(
        !off.similar.is_empty(),
        "the look-alikes match structurally"
    );
    assert!(!paths(&off).contains(&TARGET), "{:?}", paths(&off));
    assert!(!off.diagnostics.engine.lexical);
    for s in &off.similar {
        assert_eq!((s.lexical_rank, s.bm25), (None, None));
        assert!(s.matched_terms.is_empty());
    }

    let on = index.query(QUERY, &opts(true));
    let rank = paths(&on).iter().position(|p| *p == TARGET);
    assert!(
        rank.is_some_and(|r| r < 5),
        "target not in the top 5: {:?}",
        paths(&on)
    );
    let hit = &on.similar[rank.unwrap()];
    assert_eq!(hit.lexical_rank, Some(1));
    assert!(hit.bm25.unwrap() > 0.0);
    for term in [
        "recordcheckpoint",
        "findpending",
        "audittrail",
        "repository",
    ] {
        assert!(
            hit.matched_terms.iter().any(|t| t == term),
            "{term} not in {:?}",
            hit.matched_terms
        );
    }
    assert!(hit.text.contains("auditTrail.recordCheckpoint(null, true)"));
    let d = &on.diagnostics.engine;
    assert!(d.lexical && d.query_terms > 0 && d.known_query_terms > 0);
    assert!(d.known_query_terms <= d.query_terms);
}

#[test]
fn compat_index_never_uses_the_lexical_channel() {
    let (_dir, index) = repo(true);
    let out = index.query(QUERY, &QueryOptions::new(Lang::Java));
    assert!(!paths(&out).contains(&TARGET), "{:?}", paths(&out));
    assert!(!out.diagnostics.engine.lexical);
    assert!(out.similar.iter().all(|s| s.bm25.is_none()));
    let off = index.query(QUERY, &opts(false));
    let key = |o: &QueryOutput| {
        o.similar
            .iter()
            .map(|s| (s.path.clone(), s.score.to_bits(), s.kept_spans.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(key(&out), key(&off));
}

#[test]
fn json_gains_additive_lexical_fields() {
    let (_dir, index) = repo(false);
    let mut o = opts(true);
    o.explain = true;
    let v = serde_json::to_value(index.query(QUERY, &o)).unwrap();
    assert_eq!(v["schema_version"], 1);
    let first = &v["similar"][0];
    assert_eq!(
        first["structural_rank"].as_u64().map(|r| r >= 1),
        Some(true)
    );
    assert!(first.get("lexical_rank").is_some());
    assert!(first["bm25"].is_number());
    assert!(first["matched_terms"].is_array());
    assert!(!first["explain"].is_null());
    let d = &v["diagnostics"];
    assert_eq!(d["lexical"], true);
    assert!(d["query_terms"].as_u64().unwrap() > 0);
    assert!(d["known_query_terms"].is_u64());

    let v = serde_json::to_value(index.query(QUERY, &opts(false))).unwrap();
    assert!(v["similar"][0]["bm25"].is_null());
    assert!(v["similar"][0]["lexical_rank"].is_null());
    assert_eq!(v["similar"][0]["matched_terms"], serde_json::json!([]));
    assert_eq!(v["diagnostics"]["lexical"], false);
}

#[test]
fn string_literals_never_become_terms() {
    let (dir, _index) = repo(false);
    let lexical = aroma_core::store::lexical::MmapLexical::open(
        &aroma_core::store::generation::IndexDir::new(dir.path())
            .current()
            .unwrap()
            .unwrap()
            .1
            .join("lexical.bin"),
    )
    .unwrap();
    use aroma_core::lexical::{term_id, LexicalSource};
    // `"nothing settled"` is a string literal in the target.
    for t in [
        "nothing",
        "settled",
        "nothing settled",
        "\"nothing settled\"",
    ] {
        assert!(lexical.postings(term_id(t)).is_empty(), "{t}");
    }
    assert!(!lexical.postings(term_id("recordcheckpoint")).is_empty());
}
