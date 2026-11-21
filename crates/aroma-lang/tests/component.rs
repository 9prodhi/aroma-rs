//! Whole-component search joins normal per-language results by parent directory.
use aroma_core::lang::Lang;
use aroma_core::profile::ProfileName;
use aroma_lang::build::{build_index, BuildOptions};
use aroma_lang::index::{ComponentPart, ComponentQuery, Index, QueryOptions};
use std::fs;
use std::time::Duration;

const JAVA: &str = "class X { int sum(int[] values) {\n int total = 0;\n for (int value : values) { total += value; }\n return total;\n } }";
const APEX: &str = "public class Y { public void save(List<Account> accounts) {\n List<Account> changed = new List<Account>();\n for (Account acc : accounts) { acc.Description = 'Closed'; changed.add(acc); }\n update changed;\n } }";

fn repo() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().unwrap();
    for (rel, src) in [
        ("a/X.java", JAVA),
        ("a/Y.cls", APEX),
        ("b/X.java", JAVA),
        ("c/Y.cls", APEX),
        ("d/Z.java", "class Z { void unrelated() { } }"),
    ] {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let src = match rel {
            "b/X.java" => src.replace("total += value", "total += value * 2"),
            "c/Y.cls" => src.replace("'Closed'", "'Archived'"),
            _ => src.to_string(),
        };
        fs::write(path, src).unwrap();
    }
    build_index(&BuildOptions {
        root: dir.path().to_path_buf(),
        langs: vec![Lang::Java, Lang::Apex],
        profile: ProfileName::Reference,
        compat: false,
        lock_timeout: Duration::from_secs(10),
        jobs: Some(1),
    })
    .unwrap();
    let index = Index::open(dir.path()).unwrap();
    (dir, index)
}

fn query() -> ComponentQuery {
    ComponentQuery {
        parts: vec![
            ComponentPart {
                lang: Lang::Java,
                source: JAVA.into(),
                path: Some("a/X.java".into()),
            },
            ComponentPart {
                lang: Lang::Apex,
                source: APEX.into(),
                path: Some("a/Y.cls".into()),
            },
        ],
        top: 10,
        options: QueryOptions::new(Lang::Java),
    }
}

#[test]
fn component_join_ranks_complete_folder_and_preserves_missing_parts() {
    let (_dir, index) = repo();
    let out = index.query_component(&query()).unwrap();
    let dirs: Vec<_> = out.components.iter().map(|c| c.dir.as_str()).collect();
    assert_eq!(dirs[..3], ["a", "b", "c"]);
    assert!(out.components[0].parts.iter().all(Option::is_some));
    assert!(out.components[1].parts[0].is_some());
    assert!(out.components[1].parts[1].is_none());
    assert!(out.components[2].parts[0].is_none());
    assert!(out.components[2].parts[1].is_some());
    assert_eq!(out.components[1].score, out.components[2].score);
    assert!((out.components[0].score - 2.0 / 61.0).abs() < 1e-12);
    assert!((out.components[1].score - 1.0 / 62.0).abs() < 1e-12);
    assert_eq!(out.parts.len(), 2);
    assert_eq!(out.parts[1].lang, Lang::Apex);
    assert_eq!(out.parts[0].path.as_deref(), Some("a/X.java"));
    let v = serde_json::to_value(out).unwrap();
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["generation"], index.manifest().generation);
    assert!(v["components"][1]["parts"][1].is_null());
    assert!(v["parts"][0]["diagnostics"]["parse_damage"].is_number());
}

#[test]
fn component_query_reuses_options_and_limits_components_after_join() {
    let (_dir, index) = repo();
    let mut q = query();
    q.top = 3;
    q.options.top_similar = 0; // Component searches must still retrieve 100 per part.
    q.options.explain = true;
    q.options.lexical = false;
    let out = index.query_component(&q).unwrap();
    assert_eq!(out.components.len(), 3);
    for hit in out.components.iter().flat_map(|c| c.parts.iter().flatten()) {
        assert!(hit.explain.is_some());
        assert!(hit.bm25.is_none());
    }
    q.top = 1;
    assert_eq!(index.query_component(&q).unwrap().components.len(), 1);
    q.top = 0;
    assert!(index.query_component(&q).unwrap().components.is_empty());
    q.top = 10;
    q.options.kind = Some(aroma_core::unit::UnitKind::Constructor);
    assert!(index.query_component(&q).unwrap().components.is_empty());
}

#[test]
fn component_query_rejects_zero_parts() {
    let (_dir, index) = repo();
    let mut q = query();
    q.parts.clear();
    assert!(index
        .query_component(&q)
        .unwrap_err()
        .contains("at least one part"));
}

#[test]
fn component_keeps_only_best_ranked_region_per_part_and_folder() {
    let (dir, index) = repo();
    drop(index);
    fs::write(
        dir.path().join("a/Other.java"),
        JAVA.replace("class X", "class Other")
            .replace("total += value", "total += value * 3"),
    )
    .unwrap();
    build_index(&BuildOptions {
        root: dir.path().to_path_buf(),
        langs: vec![Lang::Java, Lang::Apex],
        profile: ProfileName::Reference,
        compat: false,
        lock_timeout: Duration::from_secs(10),
        jobs: Some(1),
    })
    .unwrap();
    let index = Index::open(dir.path()).unwrap();
    let mut q = query();
    q.parts.truncate(1);
    let normal = index.query(JAVA, &q.options);
    assert!(
        normal
            .similar
            .iter()
            .filter(|s| s.path.starts_with("a/"))
            .count()
            >= 2
    );
    let out = index.query_component(&q).unwrap();
    let a = out.components.iter().find(|c| c.dir == "a").unwrap();
    assert_eq!(a.parts[0].as_ref().unwrap().path, normal.similar[0].path);
    assert!((a.score - 1.0 / 61.0).abs() < 1e-12);
}

// Losing clone expansion must lose the second folder's JS contribution, while
// normal queries must continue to display only one representative of the body.
fn assert_js_clones_join(
    markup_lang: Lang,
    js_lang: Lang,
    files: &[(&str, &str)],
    markup: &str,
    js: &str,
    expected: &[(&str, &str, [u32; 2], &str)],
) {
    let dir = tempfile::tempdir().unwrap();
    for (rel, source) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    build_index(&BuildOptions {
        root: dir.path().to_path_buf(),
        langs: vec![markup_lang, js_lang],
        profile: ProfileName::Reference,
        compat: false,
        lock_timeout: Duration::from_secs(10),
        jobs: Some(1),
    })
    .unwrap();
    let index = Index::open(dir.path()).unwrap();
    let mut opts = QueryOptions::new(js_lang);
    opts.top_similar = 100;
    opts.explain = true;
    let normal = index.query(js, &opts);
    assert_eq!(normal.similar.len(), 1, "normal query still deduplicates");
    let normal_json = serde_json::to_value(&normal.similar).unwrap();
    let mut markup_opts = opts.clone();
    markup_opts.lang = markup_lang;
    let markup_hits = index.query(markup, &markup_opts).similar;
    let query = ComponentQuery {
        parts: vec![
            ComponentPart {
                lang: markup_lang,
                source: markup.into(),
                path: None,
            },
            ComponentPart {
                lang: js_lang,
                source: js.into(),
                path: None,
            },
        ],
        top: 10,
        options: opts.clone(),
    };
    let out = index.query_component(&query).unwrap();
    assert_eq!(out.components.len(), expected.len());
    for &(folder, path, lines, method) in expected {
        let component = out.components.iter().find(|c| c.dir == folder).unwrap();
        let hit = component.parts[1]
            .as_ref()
            .expect("each clone folder must receive the JS part");
        assert_eq!(hit.path, path);
        assert_eq!(hit.lines, lines);
        assert!(hit.text.contains(method), "{}", hit.text);
        assert!(!hit.stale_source);
        assert!(hit.explain.is_some());
        let source = fs::read_to_string(dir.path().join(path)).unwrap();
        assert!(!hit.kept_spans.is_empty());
        for &[start, end] in &hit.kept_spans {
            let start = start as usize;
            let end = end as usize;
            assert!(end <= source.len());
            let line = source[..start].bytes().filter(|&b| b == b'\n').count() as u32 + 1;
            assert!((lines[0]..=lines[1]).contains(&line));
        }
        let markup_rank = markup_hits
            .iter()
            .position(|h| h.path.starts_with(&format!("{folder}/")))
            .unwrap();
        // Even two clone methods in one folder contribute only once. The JS
        // contribution uses the displayed rank (1), including for the hidden clone.
        assert!((component.score - (1.0 / (61.0 + markup_rank as f64) + 1.0 / 61.0)).abs() < 1e-12);
    }
    assert_eq!(
        serde_json::to_value(index.query(js, &opts).similar).unwrap(),
        normal_json,
    );
    assert_eq!(
        serde_json::to_value(index.query_component(&query).unwrap()).unwrap(),
        serde_json::to_value(out).unwrap(),
        "component ordering and selected clone regions are deterministic",
    );
}

#[test]
fn lwc_component_join_expands_exact_body_clones() {
    let markup = "<template><lightning-button label=\"Alpha\" onclick={destroyAlpha}></lightning-button></template>";
    let js = "export default class Alpha {\n destroyAlpha() {\n  this.dispatchEvent(new CustomEvent('remove'));\n }\n copyAlpha() {\n  this.dispatchEvent(new CustomEvent('remove'));\n }\n}";
    assert_js_clones_join(
        Lang::LwcHtml,
        Lang::LwcJs,
        &[
            ("lwc/alpha/alpha.html", markup),
            ("lwc/alpha/alpha.js", js),
            ("lwc/beta/beta.html", "<template><lightning-button label=\"Beta\" onclick={destroyBeta}></lightning-button></template>"),
            ("lwc/beta/beta.js", "export default class Beta {\n\n\n destroyBeta() {\n  this.dispatchEvent(new CustomEvent('remove'));\n }\n}"),
        ],
        markup,
        js,
        &[
            ("lwc/alpha", "lwc/alpha/alpha.js", [2, 4], "destroyAlpha"),
            ("lwc/beta", "lwc/beta/beta.js", [4, 6], "destroyBeta"),
        ],
    );
}

#[test]
fn aura_component_join_expands_exact_body_clones() {
    let markup = "<aura:component><lightning:button label=\"Alpha\" onclick=\"{!c.destroyAlpha}\"/></aura:component>";
    let js = "({\n destroyAlpha: function(component, event, helper) {\n  component.destroy();\n },\n copyAlpha: function(component, event, helper) {\n  component.destroy();\n }\n})";
    assert_js_clones_join(
        Lang::Aura,
        Lang::AuraJs,
        &[
            ("aura/alpha/alpha.cmp", markup),
            ("aura/alpha/alphaController.js", js),
            ("aura/beta/beta.cmp", "<aura:component><lightning:button label=\"Beta\" onclick=\"{!c.destroyBeta}\"/></aura:component>"),
            ("aura/beta/betaController.js", "({\n\n\n destroyBeta: function(component, event, helper) {\n  component.destroy();\n }\n})"),
        ],
        markup,
        js,
        &[
            ("aura/alpha", "aura/alpha/alphaController.js", [2, 4], "destroyAlpha"),
            ("aura/beta", "aura/beta/betaController.js", [4, 6], "destroyBeta"),
        ],
    );
}

#[test]
fn component_clone_expansion_respects_language_kind_and_test_filters() {
    use aroma_core::unit::{UnitKind, UnitStore};

    let dir = tempfile::tempdir().unwrap();
    let method = "export default class Card { run() { return this.value; } }";
    for (rel, source) in [
        ("lwc/alpha/alpha.js", method),
        ("lwc/beta/beta.js", method),
        ("lwc/alpha/__tests__/alpha.test.js", method),
        (
            "lwc/accessor/accessor.js",
            "export default class Card { get run() { return this.value; } }",
        ),
        (
            "aura/other/otherController.js",
            "({ run: function() { return this.value; } })",
        ),
    ] {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    build_index(&BuildOptions {
        root: dir.path().to_path_buf(),
        langs: vec![Lang::LwcJs, Lang::AuraJs],
        profile: ProfileName::Reference,
        compat: false,
        lock_timeout: Duration::from_secs(10),
        jobs: Some(1),
    })
    .unwrap();
    let index = Index::open(dir.path()).unwrap();
    // All five bodies really are clones: otherwise these exclusions would not
    // exercise expansion's filters at all.
    assert_eq!(index.n_units(), 5);
    let hash = index.record(0).body_hash;
    assert!((0..index.n_units()).all(|id| index.record(id).body_hash == hash));
    let mut q = ComponentQuery {
        parts: vec![ComponentPart {
            lang: Lang::LwcJs,
            source: method.into(),
            path: None,
        }],
        top: 10,
        // A part's language overrides the shared options' language.
        options: QueryOptions::new(Lang::AuraJs),
    };
    q.options.kind = Some(UnitKind::Method);
    let dirs = |q: &ComponentQuery| {
        index
            .query_component(q)
            .unwrap()
            .components
            .into_iter()
            .map(|c| {
                assert!((c.score - 1.0 / 61.0).abs() < 1e-12);
                c.dir
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(dirs(&q), ["lwc/alpha", "lwc/beta"]);
    q.options.include_tests = true;
    assert_eq!(dirs(&q), ["lwc/alpha", "lwc/alpha/__tests__", "lwc/beta"]);
    q.options.include_tests = false;
    q.options.kind = None;
    assert_eq!(dirs(&q), ["lwc/accessor", "lwc/alpha", "lwc/beta"]);
}

#[test]
fn component_hidden_clone_uses_admitted_clones_displayed_rank() {
    let (dir, index) = repo();
    drop(index);
    fs::write(
        dir.path().join("c/X.java"),
        JAVA.replace("total += value", "total += value * 2"),
    )
    .unwrap();
    build_index(&BuildOptions {
        root: dir.path().to_path_buf(),
        langs: vec![Lang::Java, Lang::Apex],
        profile: ProfileName::Reference,
        compat: false,
        lock_timeout: Duration::from_secs(10),
        jobs: Some(1),
    })
    .unwrap();
    let index = Index::open(dir.path()).unwrap();
    let mut q = query();
    q.parts.truncate(1);
    let normal = index.query(JAVA, &q.options);
    assert_eq!(normal.similar[0].path, "a/X.java");
    assert!(["b/X.java", "c/X.java"].contains(&normal.similar[1].path.as_str()));
    assert_eq!(
        normal
            .similar
            .iter()
            .filter(|h| ["b/X.java", "c/X.java"].contains(&h.path.as_str()))
            .count(),
        1,
    );
    let out = index.query_component(&q).unwrap();
    for folder in ["b", "c"] {
        let component = out.components.iter().find(|c| c.dir == folder).unwrap();
        assert!((component.score - 1.0 / 62.0).abs() < 1e-12);
        assert_eq!(
            component.parts[0].as_ref().unwrap().path,
            format!("{folder}/X.java"),
        );
    }
}
