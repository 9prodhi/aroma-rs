use aroma_core::features::{featurize, FeatureConfig};
use aroma_core::lang::Lang;
use aroma_core::spt::Spt;
use aroma_core::unit::{UnitKind, UnitStore};
use aroma_lang::build::{build_index, BuildOptions};
use aroma_lang::classify::{unit_spt, VarMode};
use aroma_lang::globals::Globals;
use aroma_lang::kraw::KRaw;
use aroma_lang::snippet::parse_snippet;
use aroma_lang::spec::{spec_for, LangSpec};
use aroma_lang::table::{declared_types, extract_units, parse, Limits};
use std::path::Path;

fn spec() -> &'static LangSpec {
    spec_for(Lang::LwcJs).expect("lwcjs adapter")
}
fn fixtures() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lwcjs");
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            (
                p.file_name().unwrap().to_str().unwrap().to_owned(),
                std::fs::read_to_string(p).unwrap(),
            )
        })
        .collect();
    files.sort();
    files
}
fn spts(src: &str) -> Vec<Spt> {
    let s = spec();
    let tree = parse(s, src, &Limits::DEFAULT).unwrap();
    assert!(
        !tree.root_node().has_error(),
        "{}",
        tree.root_node().to_sexp()
    );
    let g = Globals::new(s, declared_types(&tree, src, s));
    extract_units(&tree, src, s, &Limits::DEFAULT)
        .iter()
        .map(|u| unit_spt(u, s, &g, VarMode::ContextFree).unwrap())
        .collect()
}
fn describe(s: &Spt) -> String {
    fn node(s: &Spt, id: u32) -> String {
        let n = &s.nodes[id as usize];
        let mut parts = vec![serde_json::to_string(&n.label).unwrap()];
        for child in &n.children {
            parts.push(match child {
                aroma_core::spt::Elem::Keyword(k) => serde_json::to_string(k).unwrap(),
                aroma_core::spt::Elem::Leaf(id) => {
                    let l = &s.leaves[*id as usize];
                    format!(
                        "{{{}{:?}{}}}",
                        if l.is_var { "$" } else { "" },
                        l.text,
                        l.binding.map_or(String::new(), |b| format!("#{}", b.0))
                    )
                }
                aroma_core::spt::Elem::Tree(id) => node(s, *id),
            });
        }
        format!("[{}]", parts.join(", "))
    }
    node(s, s.root)
}
fn body(text: &str) -> Spt {
    spts(&format!("class Demo {{ run() {{ {text} }} }}")).remove(0)
}
fn span(k: &KRaw) -> std::ops::Range<usize> {
    let mut lo = u32::MAX;
    let mut hi = 0;
    let mut stack = vec![k];
    while let Some(k) = stack.pop() {
        match k {
            KRaw::Token { bytes, .. } => {
                lo = lo.min(bytes.start);
                hi = hi.max(bytes.end);
            }
            KRaw::Node { children, .. } => stack.extend(children),
        }
    }
    lo as usize..hi as usize
}

#[test]
fn routes_only_lwc_components_and_jest_files() {
    for p in [
        "lwc/card/card.js",
        "force-app/main/default/lwc/card/helper.js",
        "lwc/card/__tests__/card.test.js",
    ] {
        assert!((spec().matches)(p), "{p}");
    }
    for p in [
        "card.js",
        "src/card.js",
        "lwc/card/nested/helper.js",
        "lwc/card.js",
        "aura/card/cardController.js",
        "lwc/card/card.js-meta.xml",
        "lwc/card/card.html",
    ] {
        assert!(!(spec().matches)(p), "{p}");
    }
    assert!((spec().is_test_path)(Path::new(
        "lwc/card/__tests__/card.test.js"
    )));
    assert!(!(spec().is_test_path)(Path::new("lwc/contest/contest.js")));
}

#[test]
fn wrappers_parse_valid_javascript_without_errors() {
    for (w, text) in spec().wrappers.iter().zip([
        "this.value = event.detail.value;",
        "@api reset() { this.value = null; }",
        "{ value: this.value }",
    ]) {
        let tree = parse(
            spec(),
            &format!("{}{text}{}", w.prefix, w.suffix),
            &Limits::DEFAULT,
        )
        .unwrap();
        assert!(
            !tree.root_node().has_error(),
            "{}: {}",
            w.name,
            tree.root_node().to_sexp()
        );
    }
}

#[test]
fn unit_kinds_and_decorator_evidence() {
    let src = "class Card { @api reset() { this.value = null; } get title() { return this.value; } set title(v) { this.value = v; } @wire(getRecord, { fields: [NAME_FIELD] }) record; plain = 1; arrow = () => { this.reset(); }; concise = () => 1; } function helper() { return 1; } const top = () => { return 2; }; function nested() { const local = () => { return 3; }; return local(); }";
    let tree = parse(spec(), src, &Limits::DEFAULT).unwrap();
    assert!(!tree.root_node().has_error());
    let units = extract_units(&tree, src, spec(), &Limits::DEFAULT);
    assert_eq!(
        units.iter().map(|u| u.kind).collect::<Vec<_>>(),
        vec![
            UnitKind::Method,
            UnitKind::Accessor,
            UnitKind::Accessor,
            UnitKind::WireMember,
            UnitKind::Method,
            UnitKind::Method,
            UnitKind::Method,
            UnitKind::Method
        ]
    );
    assert!(
        matches!(&units[0].body, KRaw::Node { kind: "unit", children, .. } if children[0].kind() == "decorator")
    );
    assert_eq!(units[3].body.kind(), "call_expression");
    assert!(src[units[3].bytes.start as usize..units[3].bytes.end as usize].starts_with("@wire"));
}

#[test]
fn expression_identifiers_follow_context_free_var_rule() {
    let s = body("const local = input; call(local, NAME_FIELD, Name_Field, window, Window); this.value = local; object.value = local; const config = { key: local, local }; outer: while (local) { break outer; }");
    let marks: Vec<_> = s
        .leaves
        .iter()
        .map(|l| (l.text.as_ref(), l.is_var))
        .collect();
    assert_eq!(
        marks,
        vec![
            ("local", true),
            ("input", true),
            ("call", false),
            ("local", true),
            ("NAME_FIELD", false),
            ("Name_Field", true),
            ("window", false),
            ("Window", true),
            ("value", false),
            ("local", true),
            ("object", true),
            ("value", false),
            ("local", true),
            ("config", true),
            ("key", false),
            ("local", true),
            ("local", true),
            ("outer", false),
            ("local", true),
            ("outer", false)
        ]
    );
}

#[test]
fn decorators_this_optional_chain_and_templates_have_correct_labels() {
    let s = spts("class Card { @api @track @wire(getRecord) run() { return `Hi ${this.record?.Name} ${value}`; } }").remove(0);
    let labels: Vec<_> = s.nodes.iter().map(|n| n.label.as_ref()).collect();
    for keyword in ["@api", "@track", "@wire", "this", "?."] {
        assert!(
            labels.iter().any(|l: &&str| l.contains(keyword)),
            "{keyword}: {labels:?}"
        );
    }
    assert!(s
        .leaves
        .iter()
        .any(|l| l.text.as_ref() == "Hi " && !l.is_var));
    assert!(s
        .leaves
        .iter()
        .any(|l| l.text.as_ref() == "value" && l.is_var));
}

#[test]
fn thirty_six_construct_fixtures_match_snapshots() {
    let all = fixtures();
    assert_eq!(all.len(), 36);
    for (name, src) in all {
        let units = spts(&src);
        assert!(!units.is_empty(), "{name}");
        let mut settings = insta::Settings::clone_current();
        settings.set_snapshot_path("snapshots/lwcjs");
        settings.set_prepend_module_to_snapshot(false);
        settings.set_omit_expression(true);
        settings.bind(|| {
            insta::assert_snapshot!(
                name.trim_end_matches(".js"),
                units.iter().map(describe).collect::<Vec<_>>().join("\n")
            )
        });
    }
}

#[test]
fn d18_invariant_lwcjs_indexed_fixture_repo_200_units() {
    let repo = tempfile::tempdir().unwrap();
    let s = spec();
    let all = fixtures();
    let mut selected = Vec::new();
    for i in 0..6 {
        for (name, src) in &all {
            let p = repo.path().join(format!("lwc/card{i}/{}", name));
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, src).unwrap();
            let tree = parse(s, src, &Limits::DEFAULT).unwrap();
            for u in extract_units(&tree, src, s, &Limits::DEFAULT) {
                selected.push((
                    p.strip_prefix(repo.path()).unwrap().to_owned(),
                    u.bytes.clone(),
                    src[span(&u.body)].to_owned(),
                ));
            }
        }
    }
    let opts = BuildOptions {
        root: repo.path().to_owned(),
        langs: vec![Lang::LwcJs],
        profile: aroma_core::profile::ProfileName::Reference,
        compat: false,
        lock_timeout: std::time::Duration::from_secs(10),
        jobs: Some(2),
    };
    build_index(&opts).unwrap();
    let index = aroma_lang::index::Index::open(repo.path()).unwrap();
    let g = Globals::new(s, ["Sample".to_string()]);
    assert!(index.n_units() >= 200);
    for id in 0..200 {
        let rec = index.record(id);
        let indexed = index.spt(id).unwrap();
        let (_, _, text) = selected
            .iter()
            .find(|(p, bytes, _)| p == index.file_path(rec.file) && bytes == &rec.bytes)
            .unwrap();
        let parsed = parse_snippet(s, text, &g, VarMode::ContextFree, &Limits::DEFAULT);
        assert_eq!(parsed.damage, 0.0, "{text}");
        let queried = parsed.spt.expect(text);
        assert_eq!(queried.tokens(None), indexed.tokens(None), "{text}");
        assert_eq!(
            featurize(&queried, &FeatureConfig::DEFAULT, None),
            featurize(&indexed, &FeatureConfig::DEFAULT, None),
            "{text}"
        );
    }
}

#[test]
fn repository_class_globals_and_jest_flags_survive_index_storage() {
    let repo = tempfile::tempdir().unwrap();
    for (path, source) in [
        ("lwc/card/service.js", "export class RepoService { run() { return this.value; } }"),
        ("lwc/card/card.js", "import ImportedService from 'c/service'; class Card { run() { consume(RepoService.value, ImportedService.value, NAME_FIELD, window, Window); } }"),
        ("lwc/card/__tests__/card.test.js", "function check() { expect(card.value).toBe(1); }"),
    ] {
        let p = repo.path().join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, source).unwrap();
    }
    let opts = BuildOptions {
        root: repo.path().to_owned(),
        langs: vec![Lang::LwcJs],
        profile: aroma_core::profile::ProfileName::Reference,
        compat: false,
        lock_timeout: std::time::Duration::from_secs(10),
        jobs: Some(2),
    };
    let report = build_index(&opts).unwrap();
    assert_eq!(report.units, 3);
    let index = aroma_lang::index::Index::open(repo.path()).unwrap();
    for id in 0..index.n_units() {
        let rec = index.record(id);
        let path = index.file_path(rec.file).to_str().unwrap();
        assert_eq!(rec.is_test, path.contains("__tests__"));
        if path.ends_with("/card.js") {
            let spt = index.spt(id).unwrap();
            let marks: Vec<_> = spt
                .leaves
                .iter()
                .map(|l| (l.text.as_ref(), l.is_var))
                .collect();
            assert_eq!(
                marks,
                vec![
                    ("consume", false),
                    ("RepoService", false),
                    ("value", false),
                    ("ImportedService", true),
                    ("value", false),
                    ("NAME_FIELD", false),
                    ("window", false),
                    ("Window", true)
                ]
            );
        }
    }
}

#[test]
fn pasted_bodies_with_template_and_regex_brackets_keep_index_features() {
    for text in [
        "{ return `literal { text ${value}`; }",
        "{ return /[{]/.test(value); }",
        "{ return `literal } text ${value}`; }",
    ] {
        let indexed = spts(&format!("class Card {{ run() {text} }}")).remove(0);
        let q = parse_snippet(
            spec(),
            text,
            &Globals::new(spec(), Vec::new()),
            VarMode::ContextFree,
            &Limits::DEFAULT,
        );
        assert_eq!(q.damage, 0.0, "{text}");
        assert_eq!(describe(&q.spt.unwrap()), describe(&indexed), "{text}");
    }
}

#[test]
fn javascript_binding_scopes_preserve_function_var_default_and_switch_uses() {
    let cases: &[(&str, &[Option<u32>])] = &[
        ("for (var Map of source) { let local = Map; } consume(Map);", &[Some(0), Some(0), Some(0)]),
        ("function Map() { return input; } consume(Map);", &[Some(0), Some(0)]),
        ("if (ready) { var Map = source; } consume(Map);", &[Some(0), Some(0)]),
        ("const { Map = fallback } = input; consume(Map);", &[Some(0), Some(0)]),
        ("try { run(); } catch ({ Map = fallback }) { consume(Map); }", &[Some(0), Some(0)]),
        ("let Map = outer; switch (value) { case 0: let Map = inner; consume(Map); } consume(Map);", &[Some(0), Some(1), Some(1), Some(0)]),
        ("var Map = outer; const nested = () => { var Map = inner; consume(Map); }; consume(Map);", &[Some(0), Some(2), Some(2), Some(0)]),
    ];
    for (source, expected) in cases {
        let spt = body(source);
        let marked: Vec<_> = spt
            .leaves
            .iter()
            .filter(|l| l.text.as_ref() == "Map")
            .collect();
        assert!(marked.iter().all(|l| l.is_var), "{source}: {marked:?}");
        assert_eq!(
            marked
                .iter()
                .map(|l| l.binding.map(|b| b.0))
                .collect::<Vec<_>>(),
            *expected,
            "{source}"
        );
    }
}

/// A pasted member must reach the same representation as its independently indexed
/// source, including leading decorators. Each fixture is a handwritten one-unit repo.
fn assert_member_query(member: &str, query: &str, partial: bool) {
    let repo = tempfile::tempdir().unwrap();
    let path = repo.path().join("lwc/card/card.js");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, format!("export default class Card {{\n{member}\n}}")).unwrap();
    let report = build_index(&BuildOptions {
        root: repo.path().to_owned(),
        langs: vec![Lang::LwcJs],
        profile: aroma_core::profile::ProfileName::Reference,
        compat: false,
        lock_timeout: std::time::Duration::from_secs(10),
        jobs: Some(1),
    })
    .unwrap();
    assert_eq!(report.units, 1);
    let index = aroma_lang::index::Index::open(repo.path()).unwrap();
    let indexed = index.spt(0).unwrap();
    let parsed = parse_snippet(
        spec(),
        query,
        &Globals::new(spec(), ["Card".to_string()]),
        VarMode::ContextFree,
        &Limits::DEFAULT,
    );
    assert_eq!(parsed.wrapper, "member", "{query}");
    assert_eq!(parsed.damage, 0.0, "{query}");
    let queried = parsed.spt.unwrap();
    if partial {
        assert!(queried.leaves.len() < indexed.leaves.len());
        assert!(queried
            .leaves
            .iter()
            .zip(&indexed.leaves)
            .all(|(a, b)| a.text == b.text));
        let prefix: Vec<_> = (0..indexed.leaves.len())
            .map(|i| i < queried.leaves.len())
            .collect();
        // Pair features are attributed to both endpoints; masking the suffix
        // compares the actual feature prefix without inventing a following leaf.
        assert_eq!(
            featurize(&queried, &FeatureConfig::DEFAULT, None),
            featurize(&indexed, &FeatureConfig::DEFAULT, Some(&prefix))[..queried.leaves.len()]
        );
    } else {
        assert_eq!(queried.tokens(None), indexed.tokens(None));
        assert_eq!(
            featurize(&queried, &FeatureConfig::DEFAULT, None),
            featurize(&indexed, &FeatureConfig::DEFAULT, None)
        );
    }
    for leaf in &queried.leaves {
        assert_eq!(
            &query[leaf.bytes.start as usize..leaf.bytes.end as usize],
            leaf.text.as_ref()
        );
    }
    for lexical in [false, true] {
        let mut options = aroma_lang::index::QueryOptions::new(Lang::LwcJs);
        options.lexical = lexical;
        let result = index.query(query, &options);
        assert_eq!(result.similar.len(), 1);
        assert_eq!(result.similar[0].path, "lwc/card/card.js");
        assert_eq!(result.similar[0].score, 1.0, "{query}");
        assert_eq!(result.similar[0].structural_rank, 1);
    }
}

macro_rules! member_query_tests {
    ($($name:ident: $text:expr,)*) => {$(
        #[test]
        fn $name() {
            assert_member_query($text, $text, false);
        }
    )*};
}

member_query_tests! {
    pasted_plain_member_matches_index: "save(event) { this.value = event.detail.value; }",
    pasted_api_member_matches_index: "@api save(event) { this.value = event.detail.value; }",
    pasted_multiline_decorators_match_index: "// member docs\n@api\n@wire(\n    getRecord,\n    { recordId: '$recordId', fields: [NAME_FIELD] }\n)\nsave(event) { this.value = event.detail.value; }",
    pasted_track_arrow_matches_index: "@track save = (event) => { this.value = event.detail.value; };",
    pasted_wire_field_matches_index: "@wire(getRecord, { recordId: '$recordId', fields: [NAME_FIELD] }) record;",
    pasted_getter_matches_index: "get value() { return this.record.value; }",
    pasted_setter_matches_index: "set value(event) { this.record.value = event.detail.value; }",
    pasted_async_member_matches_index: "async save(event) { this.value = await load(event.detail.value); }",
    pasted_static_member_matches_index: "static save(event) { this.value = event.detail.value; }",
    pasted_generator_member_matches_index: "*values(event) { yield event.detail.value; }",
    pasted_arrow_member_matches_index: "save = (event) => { this.value = event.detail.value; };",
}

#[test]
fn pasted_cut_member_keeps_index_feature_prefix_and_perfect_score() {
    assert_member_query(
        "save(event) {\n    this.value = event.detail.value;\n    if (this.ready) {\n        publish(this.value);\n        reset(this.value);\n    }\n}",
        "save(event) {\n    this.value = event.detail.value;\n    if (this.ready) {\n        publish(this.value);",
        true,
    );
}

#[test]
fn statement_queries_keep_their_statement_structure() {
    for text in [
        "if (x) { this.a = 1; }",
        "for (let x = 0; x < 3; x++) { foo(x); }",
        "while (x) { foo(x); }",
        "do { foo(x); } while (x);",
        "switch (x) { case 1: foo(); break; }",
        "try { foo(); } catch (x) { bar(x); }",
        "catch (x) { bar(x); }",
        "with (x) { foo(); }",
        "function f() { this.a = 1; }",
        "foo()\n{ this.a = 1; }",
        "foo();\n{ this.a = 1; }",
        "this.a = 1; foo();",
        // The class-body parse drops tail(); it must lose to the undamaged
        // statements parse even though its first token looks like a member.
        "save(event) { this.value = event.detail.value; } tail();",
    ] {
        let parsed = parse_snippet(
            spec(),
            text,
            &Globals::new(spec(), Vec::new()),
            VarMode::ContextFree,
            &Limits::DEFAULT,
        );
        assert_eq!(parsed.wrapper, "statements", "{text}");
        let queried = parsed.spt.unwrap();
        // Extract the wrapper body directly, without snippet wrapper selection.
        // Standalone catch is deliberately tolerated by tree-sitter recovery.
        let src = format!("class Card {{ run() {{\n{text}\n}} }}");
        let tree = parse(spec(), &src, &Limits::DEFAULT).unwrap();
        let unit = extract_units(&tree, &src, spec(), &Limits::DEFAULT).remove(0);
        let indexed = unit_spt(
            &unit,
            spec(),
            &Globals::new(spec(), Vec::new()),
            VarMode::ContextFree,
        )
        .unwrap();
        assert_eq!(queried.tokens(None), indexed.tokens(None), "{text}");
        assert_eq!(
            featurize(&queried, &FeatureConfig::DEFAULT, None),
            featurize(&indexed, &FeatureConfig::DEFAULT, None),
            "{text}"
        );
    }
}

#[test]
fn statement_keyword_member_names_are_never_selected_even_after_decorators() {
    for keyword in [
        "if", "for", "while", "do", "switch", "try", "catch", "with", "function", "return", "var",
        "let", "const", "throw", "break", "continue", "class", "import", "export",
    ] {
        for leading in ["", "/* docs */\n@api\n@wire(\n    getRecord\n)\n"] {
            let text = format!("{leading}{keyword}(x) {{ this.value = x; }}");
            let parsed = parse_snippet(
                spec(),
                &text,
                &Globals::new(spec(), Vec::new()),
                VarMode::ContextFree,
                &Limits::DEFAULT,
            );
            assert_ne!(parsed.wrapper, "member", "{text}");
        }
    }
}

/// Captured before the JS-family table refactor: labels, bindings and every
/// per-leaf feature count must stay byte-identical for both feature profiles.
#[test]
fn pre_refactor_feature_bags_stay_identical() {
    let sources = [
        "class Card { @api save(event) { if (event) { var Map = event.detail; } consume(Map); const f = x => { let Map = x; return Map; }; return f(Map); } }",
        "class Card { @wire(getRecord, { recordId: '$recordId' }) record; get title() { return this.record?.Name; } }",
        "function run(input) { try { const { value: local = fallback } = input; return `${local}`; } catch (error) { console.log(error); } }",
    ];
    let mut fingerprints = Vec::new();
    for src in sources {
        for spt in spts(src) {
            for config in [FeatureConfig::DEFAULT, FeatureConfig::REFERENCE] {
                let encoded = serde_json::to_vec(&featurize(&spt, &config, None)).unwrap();
                fingerprints.push(blake3::hash(&encoded).to_hex().to_string());
            }
        }
    }
    insta::assert_debug_snapshot!("pre_refactor_feature_bags", fingerprints);
}
