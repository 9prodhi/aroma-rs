use aroma_core::features::{featurize, FeatureConfig};
use aroma_core::lang::Lang;
use aroma_core::spt::Spt;
use aroma_core::unit::{UnitKind, UnitStore};
use aroma_lang::build::{build_index, BuildOptions};
use aroma_lang::classify::{unit_spt, VarMode};
use aroma_lang::globals::Globals;
use aroma_lang::snippet::{parse_snippet, SnippetParse};
use aroma_lang::spec::{lang_for_path, resolve_query_lang, spec_for, LangSpec};
use aroma_lang::table::{extract_units, parse, Limits};

fn spec() -> &'static LangSpec {
    spec_for(Lang::AuraJs).expect("aurajs adapter")
}
fn spts(src: &str) -> Vec<Spt> {
    let tree = parse(spec(), src, &Limits::DEFAULT).unwrap();
    assert!(
        !tree.root_node().has_error(),
        "{}",
        tree.root_node().to_sexp()
    );
    extract_units(&tree, src, spec(), &Limits::DEFAULT)
        .iter()
        .map(|unit| {
            assert_eq!(unit.kind, UnitKind::Method);
            assert_eq!(unit.damage, 0.0);
            unit_spt(
                unit,
                spec(),
                &Globals::new(spec(), Vec::new()),
                VarMode::ContextFree,
            )
            .unwrap()
        })
        .collect()
}
fn body(text: &str) -> Spt {
    spts(&format!(
        "({{ run: function(cmp, event, helper) {{ {text} }} }})"
    ))
    .remove(0)
}
fn snippet(text: &str) -> SnippetParse {
    parse_snippet(
        spec(),
        text,
        &Globals::new(spec(), Vec::new()),
        VarMode::ContextFree,
        &Limits::DEFAULT,
    )
}
macro_rules! route_tests {
    ($($name:ident: $path:expr => $expected:expr,)*) => {$(
        #[test]
        fn $name() {
            let expected = $expected;
            assert_eq!((spec().matches)($path), expected);
            assert_eq!(lang_for_path($path, &[Lang::AuraJs]), expected.then_some(Lang::AuraJs));
            assert_eq!(lang_for_path($path, &[]), None);
        }
    )*};
}
route_tests! {
    route_controller: "aura/card/cardController.js" => true,
    route_helper: "aura/card/cardHelper.js" => true,
    route_renderer: "aura/card/cardRenderer.js" => true,
    route_mdapi: "src/aura/card/cardController.js" => true,
    route_sfdx: "force-app/main/default/aura/card/cardHelper.js" => true,
    reject_plain_js: "aura/card/card.js" => false,
    reject_arbitrary_js: "aura/card/util.js" => false,
    reject_design: "aura/card/card.design" => false,
    reject_markup: "aura/card/card.cmp" => false,
    reject_lwc: "lwc/card/cardController.js" => false,
    reject_no_bundle: "aura/cardController.js" => false,
    reject_empty_bundle: "aura//cardController.js" => false,
    reject_nested: "aura/card/nested/cardController.js" => false,
    reject_tests: "aura/card/__tests__/cardController.js" => false,
    reject_external_controller: "src/cardController.js" => false,
    reject_metadata: "aura/card/cardController.js-meta.xml" => false,
    reject_suffix_case: "aura/card/cardcontroller.js" => false,
    reject_extension_case: "aura/card/cardController.JS" => false,
}

#[test]
fn units_are_top_level_function_pairs_only() {
    let src = "({ run: function(cmp) { return cmp.get('v.x'); }, value: 1, nested: { inner: function() { return 2; } }, arrow: () => 3, shorthand() { return 4; }, save: function(event) { return event.getParam('x'); } })";
    let tree = parse(spec(), src, &Limits::DEFAULT).unwrap();
    let units = extract_units(&tree, src, spec(), &Limits::DEFAULT);
    assert_eq!(units.len(), 2);
    assert!(src[units[0].bytes.start as usize..units[0].bytes.end as usize].starts_with("run:"));
    assert!(src[units[1].bytes.start as usize..units[1].bytes.end as usize].starts_with("save:"));
    assert_eq!(spts(src).len(), 2);
}

#[test]
fn nested_callbacks_and_object_functions_are_not_units() {
    let src = "({ run: function(cmp, event, helper) { $A.enqueueAction(function(result) { const options = { callback: function(value) { return value; } }; helper.handle(cmp, result, options); }); $A.getCallback(function() { cmp.set('v.ready', true); })(); } })";
    let units = spts(src);
    assert_eq!(units.len(), 1);
    for name in ["result", "options", "value"] {
        assert!(units[0]
            .leaves
            .iter()
            .any(|l| l.text.as_ref() == name && l.is_var && l.binding.is_some()));
    }
}

#[test]
fn standalone_functions_and_class_members_are_not_units() {
    assert!(spts("function run() { return 1; } class Card { run() { return 2; } }").is_empty());
}

#[test]
fn arbitrary_variable_objects_are_not_controller_units() {
    assert!(spts("const options = { callback: function() { return 1; } };").is_empty());
}

#[test]
fn nested_controller_body_objects_are_not_units() {
    assert_eq!(
        spts("({ run: function() { return { nested: function() { return 1; } }; } })").len(),
        1
    );
}

#[test]
fn quoted_computed_and_async_function_pairs_are_units() {
    assert_eq!(spts("({ 'run': function() { return 1; }, [name]: function() { return 2; }, load: async function() { return await fetch('/data'); } })").len(), 3);
}

#[test]
fn wrappers_are_valid_javascript() {
    for (w, text) in spec().wrappers.iter().zip([
        "cmp.set('v.x', event);",
        "run: function(cmp) { return cmp; }",
        "cmp.get('v.x')",
    ]) {
        let src = format!("{}{text}{}", w.prefix, w.suffix);
        let tree = parse(spec(), &src, &Limits::DEFAULT).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "{}: {}",
            w.name,
            tree.root_node().to_sexp()
        );
    }
}

#[test]
fn aura_and_javascript_globals_are_not_variables() {
    let spt = body("consume($A, window, document, console, JSON, Math, Promise, Object, Array, Date, Number, String, Boolean, Error, Map, Set, sforce);");
    assert!(spt.leaves.iter().all(|l| !l.is_var));
}

#[test]
fn parameters_are_context_free_locals() {
    let spt = body("helper.save(cmp, component, event); consume(cmp, component, event, helper);");
    for name in ["cmp", "component", "event", "helper"] {
        let leaves: Vec<_> = spt
            .leaves
            .iter()
            .filter(|l| l.text.as_ref() == name)
            .collect();
        assert!(!leaves.is_empty());
        assert!(leaves.iter().all(|l| l.is_var));
    }
}

#[test]
fn aura_global_shadowed_in_unit_is_local() {
    let spt = body("const $A = source; consume($A);");
    let leaves: Vec<_> = spt
        .leaves
        .iter()
        .filter(|l| l.text.as_ref() == "$A")
        .collect();
    assert_eq!(leaves.len(), 2);
    assert!(leaves
        .iter()
        .all(|l| l.is_var && l.binding == Some(aroma_core::spt::SymbolId(0))));
}

macro_rules! binding_tests {
    ($($name:ident: $text:expr => $expected:expr,)*) => {$(
        #[test]
        fn $name() {
            let spt = body($text);
            let leaves: Vec<_> = spt.leaves.iter().filter(|l| l.text.as_ref() == "Map").collect();
            assert_eq!(leaves.iter().map(|l| l.binding.map(|b| b.0)).collect::<Vec<_>>(), $expected);
        }
    )*};
}
binding_tests! {
    var_hoists_across_block: "if (ready) { var Map = source; } consume(Map);" => vec![Some(0), Some(0)],
    let_stays_in_block: "if (ready) { let Map = source; } consume(Map);" => vec![Some(0), None],
    var_in_for_in_hoists: "for (var Map in source) { consume(Map); } consume(Map);" => vec![Some(0), Some(0), Some(0)],
    callback_var_does_not_escape: "$A.getCallback(function() { var Map = source; consume(Map); }); consume(Map);" => vec![Some(0), Some(0), None],
    nested_function_shadowing: "var Map = outer; const nested = () => { var Map = inner; consume(Map); }; consume(Map);" => vec![Some(0), Some(2), Some(2), Some(0)],
    function_name_binds_enclosing_scope: "function Map() { return input; } consume(Map);" => vec![Some(0), Some(0)],
    callback_parameters_bind_locally: "$A.enqueueAction(function(Map) { consume(Map); }); consume(Map);" => vec![Some(0), Some(0), None],
    catch_parameter_binds_locally: "try { run(); } catch (Map) { consume(Map); } consume(Map);" => vec![Some(0), Some(0), None],
}

fn assert_pair_query(pair: &str, query: &str, partial: bool) {
    let repo = tempfile::tempdir().unwrap();
    let path = repo.path().join("aura/card/cardController.js");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, format!("({{ {pair} }})")).unwrap();
    let report = build_index(&BuildOptions {
        root: repo.path().to_owned(),
        langs: vec![Lang::AuraJs],
        profile: aroma_core::profile::ProfileName::Reference,
        compat: false,
        lock_timeout: std::time::Duration::from_secs(10),
        jobs: Some(1),
    })
    .unwrap();
    assert_eq!(report.units, 1);
    assert_eq!(report.files_failed, 0);
    assert_eq!(report.units_skipped_damaged, 0);
    let index = aroma_lang::index::Index::open(repo.path()).unwrap();
    let indexed = index.spt(0).unwrap();
    let parsed = snippet(query);
    assert_eq!(parsed.wrapper, "member", "{query}");
    assert_eq!(parsed.damage, 0.0);
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
    for l in &queried.leaves {
        assert_eq!(
            &query[l.bytes.start as usize..l.bytes.end as usize],
            l.text.as_ref()
        );
    }
    for lexical in [false, true] {
        let mut options = aroma_lang::index::QueryOptions::new(Lang::AuraJs);
        options.lexical = lexical;
        let result = index.query(query, &options);
        assert_eq!(result.similar.len(), 1);
        assert_eq!(result.similar[0].score, 1.0);
        assert_eq!(result.similar[0].structural_rank, 1);
    }
}
macro_rules! pair_tests {
    ($($name:ident: $text:expr,)*) => {$(
        #[test]
        fn $name() { assert_pair_query($text, $text, false); }
    )*};
}
pair_tests! {
    pasted_pair_has_perfect_score: "save: function(cmp, event, helper) { helper.save(cmp, event.getParam('value')); }",
    pasted_pair_with_callback_has_perfect_score: "run: function(cmp) { $A.enqueueAction($A.getCallback(function(result) { cmp.set('v.result', result); })); }",
    pasted_async_pair_has_perfect_score: "load: async function(cmp) { const value = await fetch('/data'); cmp.set('v.value', value); }",
    pasted_commented_pair_has_perfect_score: "/* docs */\nload: function(cmp) { cmp.set('v.x', true); }",
    pasted_quoted_pair_has_perfect_score: "'save': function(cmp) { cmp.set('v.x', true); }",
    pasted_pair_with_trailing_comma_has_perfect_score: "save: function(cmp) { cmp.set('v.x', true); },",
    pasted_allman_pair_has_perfect_score: "save: function(cmp)\n{ cmp.set('v.x', true); }",
}

#[test]
fn half_pasted_pair_repairs_and_matches_feature_prefix() {
    assert_pair_query("save: function(cmp) {\n const action = cmp.get('c.save');\n if (action) {\n $A.enqueueAction(action);\n cmp.set('v.ready', true);\n }\n}", "save: function(cmp) {\n const action = cmp.get('c.save');\n if (action) {\n $A.enqueueAction(action);", true);
}

#[test]
fn pasted_body_matches_index_features() {
    let text = "{ const action = cmp.get('c.save'); $A.enqueueAction(action); }";
    let indexed = spts(&format!("({{ save: function(cmp) {text} }})")).remove(0);
    let parsed = snippet(text);
    assert_eq!(parsed.damage, 0.0);
    assert_eq!(
        featurize(&parsed.spt.unwrap(), &FeatureConfig::DEFAULT, None),
        featurize(&indexed, &FeatureConfig::DEFAULT, None)
    );
}

#[test]
fn half_pasted_statement_body_repairs() {
    let parsed = snippet("if (cmp.isValid()) {\n $A.enqueueAction(cmp.get('c.save')); ");
    assert_eq!(parsed.wrapper, "statements");
    assert_eq!(parsed.damage, 0.0);
    assert!(parsed.spt.is_some());
}

#[test]
fn statement_keywords_cannot_win_as_object_members() {
    for keyword in [
        "if", "for", "while", "do", "switch", "try", "catch", "with", "function", "return", "var",
        "let", "const", "throw", "break", "continue", "class", "import", "export",
    ] {
        for leading in ["", "/* docs */\n"] {
            let text = format!("{leading}{keyword}: function(cmp) {{ consume(cmp); }}");
            assert_ne!(snippet(&text).wrapper, "member", "{text}");
        }
    }
}

#[test]
fn control_flow_keeps_statements_precedence() {
    for text in [
        "if (ready) { helper.save(cmp); }",
        "for (let i = 0; i < 3; i++) { helper.save(i); }",
        "while (ready) { helper.save(cmp); }",
        "function run() { helper.save(cmp); }",
        "foo()\n{ helper.save(cmp); }",
        "try { run(); } catch (error) { console.log(error); }",
    ] {
        assert_eq!(snippet(text).wrapper, "statements", "{text}");
    }
}

#[test]
fn member_discarding_tail_cannot_win() {
    let parsed = snippet("save: function(cmp) { consume(cmp); }, tail: 42");
    assert_ne!(parsed.wrapper, "member");
}

#[test]
fn single_expression_unwraps_like_javascript() {
    let spt = snippet("cmp.get('v.value')").spt.unwrap();
    assert!(spt.leaves[0].is_var);
    assert_eq!(
        spt.tokens(None),
        vec!["cmp", ".", "get", "(", "'v.value'", ")"]
    );
}

#[test]
fn empty_and_comment_queries_are_empty() {
    for text in ["", " \n", "// comment", "/* comment */"] {
        let parsed = snippet(text);
        assert!(parsed.spt.is_none());
        assert_eq!(parsed.damage, 0.0);
    }
}

#[test]
fn alias_resolves_javascript() {
    for text in [
        "",
        "cmp.get('v.value')",
        "save: function(cmp) { return cmp; }",
    ] {
        assert_eq!(resolve_query_lang("aura", text), Ok(Lang::AuraJs));
        assert_eq!(resolve_query_lang("aurajs", text), Ok(Lang::AuraJs));
    }
    assert_eq!(spec().grammar_version, "tree-sitter-javascript 0.25.0");
    assert_eq!(spec().term_rule, aroma_core::lexical::TermRule::Identifier);
    assert!(!(spec().is_test_path)(std::path::Path::new(
        "aura/card/cardController.js"
    )));
}
