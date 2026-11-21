//! T12: Java virtual-ANTLR normalizer and refined `LangSpec` tables.
//!
//! SPTs are compared through `sexpr`, a canonical one-line rendering of the reference's
//! JSON shape: a node is `["label", child, …]`, a keyword is its JSON string, a leaf is
//! `{"text"}`, a `#VAR` leaf `{$"text"}` and, with bindings, `{$"text"#id}`.

use aroma_core::features::{featurize, FeatureConfig};
use aroma_core::lang::Lang;
use aroma_core::spt::{Elem, NodeId, Spt};
use aroma_lang::classify::{to_raw, unit_spt, VarMode};
use aroma_lang::globals::Globals;
use aroma_lang::normalize::{Identity, Normalizer};
use aroma_lang::spec::{spec_for, LangSpec};
use aroma_lang::table::{declared_types, extract_units, parse, to_kraw, AdapterDiag, Limits};
use std::path::PathBuf;

fn java() -> &'static LangSpec {
    spec_for(Lang::Java).unwrap()
}

fn q(s: &str) -> String {
    serde_json::to_string(s).expect("a str always serializes")
}

fn sexpr_node(spt: &Spt, n: NodeId, bindings: bool, out: &mut String) {
    let node = &spt.nodes[n as usize];
    out.push('[');
    out.push_str(&q(&node.label));
    for c in &node.children {
        out.push_str(", ");
        match c {
            Elem::Keyword(k) => out.push_str(&q(k)),
            Elem::Leaf(l) => {
                let leaf = &spt.leaves[*l as usize];
                out.push('{');
                if leaf.is_var {
                    out.push('$');
                }
                out.push_str(&q(&leaf.text));
                if bindings {
                    if let Some(b) = leaf.binding {
                        out.push_str(&format!("#{}", b.0));
                    }
                }
                out.push('}');
            }
            Elem::Tree(t) => sexpr_node(spt, *t, bindings, out),
        }
    }
    out.push(']');
}

fn sexpr(spt: &Spt, bindings: bool) -> String {
    let mut out = String::new();
    sexpr_node(spt, spt.root, bindings, &mut out);
    out
}

/// SPT of every unit in `src` (globals = builtins + the file's declared types).
fn unit_spts(src: &str, mode: VarMode) -> Vec<Spt> {
    let spec = java();
    let tree = parse(spec, src, &Limits::DEFAULT).expect("parses");
    let globals = Globals::new(spec, declared_types(&tree, src, spec));
    extract_units(&tree, src, spec, &Limits::DEFAULT)
        .iter()
        .map(|u| unit_spt(u, spec, &globals, mode).expect("non-empty unit"))
        .collect()
}

/// `sexpr` of method `void m() { <body> }` of class `T` (the first unit; methods of
/// anonymous classes inside it are further units).
fn body(body: &str, mode: VarMode, bindings: bool) -> String {
    let src = format!("class T {{ void m() {{ {body} }} }}");
    let spts = unit_spts(&src, mode);
    sexpr(spts.first().expect("one unit"), bindings)
}

fn compat(src: &str) -> String {
    body(src, VarMode::Compat, false)
}

// ---------------------------------------------------------------- Step group 1

#[test]
fn local_declaration_splits_off_its_semicolon() {
    assert_eq!(
        compat("int x = 1;"),
        r####"["{#}", "{", ["#;", ["int#", "int", ["#=#", {$"x"}, "=", {"1"}]], ";"], "}"]"####
    );
}

#[test]
fn receiver_call_is_object_dot_method_call() {
    assert_eq!(
        compat("a.b(c, d);"),
        r####"["{#}", "{", ["#;", ["#.#", {$"a"}, ".", ["#(#)", {"b"}, "(", ["#,#", {$"c"}, ",", {$"d"}], ")"]], ";"], "}"]"####
    );
}

#[test]
fn bare_call_keeps_its_parentheses() {
    assert_eq!(
        compat("run(x);"),
        r####"["{#}", "{", ["#;", ["#(#)", {"run"}, "(", {$"x"}, ")"], ";"], "}"]"####
    );
}

#[test]
fn creator_keeps_an_arguments_node() {
    assert_eq!(
        compat("Foo f = new Foo(x);"),
        r####"["{#}", "{", ["#;", ["##", {"Foo"}, ["#=#", {$"f"}, "=", ["new#", "new", ["##", {"Foo"}, ["(#)", "(", {$"x"}, ")"]]]]], ";"], "}"]"####
    );
}

#[test]
fn for_header_becomes_for_control() {
    assert_eq!(
        compat("for (int i = 0; i < n; i++) go(i);"),
        r####"["{#}", "{", ["for(#)#", "for", "(", ["#;#;#", ["int#", "int", ["#=#", {$"i"}, "=", {"0"}]], ";", ["#<#", {$"i"}, "<", {$"n"}], ";", ["#++", {$"i"}, "++"]], ")", ["#;", ["#(#)", {"go"}, "(", {$"i"}, ")"], ";"]], "}"]"####
    );
}

#[test]
fn enhanced_for_gets_a_control_list() {
    assert_eq!(
        compat("for (String s : xs) use(s);"),
        r####"["{#}", "{", ["for(#)#", "for", "(", ["##:#", {"String"}, {$"s"}, ":", {$"xs"}], ")", ["#;", ["#(#)", {"use"}, "(", {$"s"}, ")"], ";"]], "}"]"####
    );
}

#[test]
fn catch_parameter_is_inlined() {
    assert_eq!(
        compat("try { a(); } catch (E e) { b(e); }"),
        r####"["{#}", "{", ["try##", "try", ["{#}", "{", ["#;", ["#()", {"a"}, "(", ")"], ";"], "}"], ["catch(##)#", "catch", "(", {"E"}, {$"e"}, ")", ["{#}", "{", ["#;", ["#(#)", {"b"}, "(", {$"e"}, ")"], ";"], "}"]]], "}"]"####
    );
}

#[test]
fn shift_operators_split_into_single_characters() {
    assert_eq!(
        compat("y = x >> 2 >>> 1 << 3;"),
        r####"["{#}", "{", ["#;", ["#=#", {$"y"}, "=", ["#<<#", ["#>>>#", ["#>>#", {$"x"}, ">", ">", {"2"}], ">", ">", ">", {"1"}], "<", "<", {"3"}]], ";"], "}"]"####
    );
}

#[test]
fn scoped_and_generic_types_flatten() {
    assert_eq!(
        compat("java.util.Map.Entry<K, V> e = null;"),
        r####"["{#}", "{", ["#;", ["##", ["#.#.#.##", {"java"}, ".", {"util"}, ".", {"Map"}, ".", {"Entry"}, ["<#,#>", "<", {"K"}, ",", {"V"}, ">"]], ["#=null", {$"e"}, "=", "null"]], ";"], "}"]"####
    );
}

#[test]
fn switch_labels_merge_into_the_next_group() {
    assert_eq!(
        compat("switch (k) { case 1: case 2: a(); default: b(); }"),
        r####"["{#}", "{", ["switch#{##}", "switch", ["(#)", "(", {$"k"}, ")"], "{", ["###", ["case#:", "case", {"1"}, ":"], ["case#:", "case", {"2"}, ":"], ["#;", ["#()", {"a"}, "(", ")"], ";"]], ["default:#", "default", ":", ["#;", ["#()", {"b"}, "(", ")"], ";"]], "}"], "}"]"####
    );
}

#[test]
fn array_creation_and_declarator_dimensions() {
    assert_eq!(
        compat("int a[] = new int[n];"),
        r####"["{#}", "{", ["#;", ["int#", "int", ["#=#", ["#[]", {$"a"}, "[", "]"], "=", ["new#", "new", ["int#", "int", ["[#]", "[", {$"n"}, "]"]]]]], ";"], "}"]"####
    );
}

#[test]
fn member_modifiers_are_hoisted_into_class_body_declaration() {
    assert_eq!(
        compat("Object o = new Object() { public int hashCode() { return 1; } };"),
        r####"["{#}", "{", ["#;", ["##", {"Object"}, ["#=#", {$"o"}, "=", ["new#", "new", ["##", {"Object"}, ["()#", "(", ")", ["{#}", "{", ["public#", "public", ["int#()#", "int", {"hashCode"}, "(", ")", ["{#}", "{", ["return#;", "return", {"1"}, ";"], "}"]]], "}"]]]]]], ";"], "}"]"####
    );
}

#[test]
fn resources_are_grouped() {
    assert_eq!(
        compat("try (A a = open(); B b = make(a)) { use(a, b); }"),
        r####"["{#}", "{", ["try##", "try", ["(#)", "(", ["#;#", ["##=#", {"A"}, {$"a"}, "=", ["#()", {"open"}, "(", ")"]], ";", ["##=#", {"B"}, {$"b"}, "=", ["#(#)", {"make"}, "(", {$"a"}, ")"]]], ")"], ["{#}", "{", ["#;", ["#(#)", {"use"}, "(", ["#,#", {$"a"}, ",", {$"b"}], ")"], ";"], "}"]], "}"]"####
    );
}

#[test]
fn method_reference_receiver_is_an_expression_operand() {
    // default (D18) mode: `seen` is a receiver variable, `add` the referenced method
    assert_eq!(
        body("xs.forEach(seen::add);", VarMode::ContextFree, false),
        r####"["{#}", "{", ["#;", ["#.#", {$"xs"}, ".", ["#(#)", {"forEach"}, "(", ["#::#", {$"seen"}, "::", {"add"}], ")"]], ";"], "}"]"####
    );
}

#[test]
fn explicit_constructor_invocation_is_a_method_call_statement() {
    let src = "class T { T(int a) { this(a, 0); } T(int a, int b) { } }";
    let spec = java();
    let tree = parse(spec, src, &Limits::DEFAULT).expect("parses");
    let mut cursor = tree.walk();
    let mut ctor_body = None;
    'search: loop {
        let n = cursor.node();
        if n.kind() == "constructor_body" {
            ctor_body = Some(n);
            break 'search;
        }
        if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                break 'search;
            }
        }
    }
    let node = ctor_body.expect("a constructor body");
    let mut diag = AdapterDiag::default();
    let kraw = to_kraw(node, src.as_bytes(), spec, &Limits::DEFAULT, &mut diag).expect("tree");
    let globals = Globals::new(spec, Vec::<String>::new());
    let raw = to_raw(
        spec.normalizer.normalize(kraw),
        spec,
        &globals,
        VarMode::Compat,
    );
    let spt = Spt::build(raw).expect("non-empty");
    assert_eq!(
        sexpr(&spt, false),
        r####"["{#}", "{", ["#;", ["this(#)", "this", "(", ["#,#", {$"a"}, ",", {"0"}], ")"], ";"], "}"]"####
    );
}

// ---------------------------------------------------------------- Step group 2

#[test]
fn lambda_parameters_are_declarations() {
    assert_eq!(
        body("f(x -> x, (p, q) -> p);", VarMode::ContextFree, true),
        r####"["{#}", "{", ["#;", ["#(#)", {"f"}, "(", ["#,#", ["#->#", {$"x"#0}, "->", {$"x"#0}], ",", ["#->#", ["(#,#)", "(", {$"p"#1}, ",", {$"q"#2}, ")"], "->", {$"p"#1}]], ")"], ";"], "}"]"####
    );
}

#[test]
fn pattern_variable_is_a_declaration() {
    assert_eq!(
        body(
            "if (o instanceof Foo f) { f.go(); }",
            VarMode::ContextFree,
            true
        ),
        r####"["{#}", "{", ["if##", "if", ["(#)", "(", ["#instanceof##", {$"o"}, "instanceof", {"Foo"}, {$"f"#0}], ")"], ["{#}", "{", ["#;", ["#.#", {$"f"#0}, ".", ["#()", {"go"}, "(", ")"]], ";"], "}"]], "}"]"####
    );
}

#[test]
fn annotation_keys_are_never_vars() {
    assert_eq!(
        compat("@SuppressWarnings(value = \"x\") int n = k;"),
        r####"["{#}", "{", ["#;", ["#int#", ["@#(#)", "@", {"SuppressWarnings"}, "(", ["#=#", {"value"}, "=", {"\"x\""}], ")"], "int", ["#=#", {$"n"}, "=", {$"k"}]], ";"], "}"]"####
    );
}

#[test]
fn common_jdk_receivers_are_globals() {
    assert_eq!(
        body(
            "Optional<String> o = Optional.empty(); Other.go(o);",
            VarMode::ContextFree,
            true
        ),
        r####"["{##}", "{", ["#;", ["##", ["##", {"Optional"}, ["<#>", "<", {"String"}, ">"]], ["#=#", {$"o"#0}, "=", ["#.#", {"Optional"}, ".", ["#()", {"empty"}, "(", ")"]]]], ";"], ["#;", ["#.#", {$"Other"}, ".", ["#(#)", {"go"}, "(", {$"o"#0}, ")"]], ";"], "}"]"####
    );
}

#[test]
fn spec_records_the_normalizer_version() {
    assert_eq!(java().adapter_version, "java-1");
    assert!(java().decl_name_parents.contains(&"inferred_parameters"));
    assert!(java()
        .non_var_fields
        .contains(&("element_value_pair", "key")));
}

// ---------------------------------------------------------------- Step group 3

fn reference_dir() -> PathBuf {
    std::env::var_os("AROMA_REFERENCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ref/aroma-paper-artifacts")
        })
}

fn reference_file(name: &str) -> Option<String> {
    let path = reference_dir().join("reference/data").join(name);
    match std::fs::read_to_string(&path) {
        Ok(s) => Some(s),
        Err(_) => {
            eprintln!(
                "skipped: reference artifacts not present ({})",
                path.display()
            );
            None
        }
    }
}

#[test]
fn compat_spt_of_example_query_equals_the_reference_json() {
    let (Some(java_src), Some(json)) = (
        reference_file("example_query.java"),
        reference_file("example_query.json"),
    ) else {
        return;
    };
    let record: serde_json::Value = serde_json::from_str(&json).expect("JSON record");
    let expected = Spt::from_reference_json(&record["ast"]).expect("reference ast loads");
    let ours = unit_spts(&java_src, VarMode::Compat);
    assert_eq!(ours.len(), 1);
    // labels, keywords, leaf texts and var marks, in order
    assert_eq!(sexpr(&ours[0], false), sexpr(&expected, false));
    let labels = |s: &Spt| -> Vec<String> { s.nodes.iter().map(|n| n.label.to_string()).collect() };
    assert_eq!(labels(&ours[0]), labels(&expected));
    let leaves = |s: &Spt| -> Vec<(String, bool, u32)> {
        s.leaves
            .iter()
            .map(|l| (l.text.to_string(), l.is_var, l.line))
            .collect()
    };
    assert_eq!(leaves(&ours[0]), leaves(&expected));
    // the reference featurizer sees the same bag (shipped default config)
    assert_eq!(
        featurize(&ours[0], &FeatureConfig::REFERENCE, None),
        featurize(&expected, &FeatureConfig::REFERENCE, None)
    );
}

// ---------------------------------------------------------------- Step group 4

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/java")
}

fn fixtures() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(fixture_dir())
        .expect("fixture dir")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "java"))
        .map(|p| {
            let stem = p.file_stem().expect("stem").to_string_lossy().into_owned();
            (stem, std::fs::read_to_string(&p).expect("fixture text"))
        })
        .collect();
    out.sort();
    out
}

/// Snapshot text of one fixture: per unit the reference-shaped (compat) tree, the
/// default (D18) tree with SymbolIds, and the default features of every leaf.
fn describe(src: &str) -> String {
    let compat = unit_spts(src, VarMode::Compat);
    let default = unit_spts(src, VarMode::ContextFree);
    let mut out = String::new();
    for (i, (c, d)) in compat.iter().zip(&default).enumerate() {
        out.push_str(&format!(
            "unit {i}\ncompat:  {}\ndefault: {}\nfeatures:\n",
            sexpr(c, false),
            sexpr(d, true)
        ));
        let per_leaf = featurize(d, &FeatureConfig::DEFAULT, None);
        for (leaf, feats) in d.leaves.iter().zip(per_leaf) {
            out.push_str(&format!(
                "  {} {}\n",
                q(&leaf.text),
                serde_json::to_string(&feats).expect("features")
            ));
        }
    }
    out
}

fn snapshot(name: &str, text: &str) {
    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_path("fixtures/java/snapshots");
    settings.set_prepend_module_to_snapshot(false);
    settings.set_omit_expression(true);
    settings.bind(|| insta::assert_snapshot!(name, text));
}

#[test]
fn thirty_construct_fixtures_match_their_snapshots() {
    let all = fixtures();
    assert_eq!(
        all.len(),
        30,
        "{:?}",
        all.iter().map(|f| &f.0).collect::<Vec<_>>()
    );
    for (name, src) in &all {
        snapshot(&format!("java_{name}"), &describe(src));
    }
}

// ---------------------------------------------------------------- Step group 5

/// (text, is_var, bytes) of every leaf in source order.
fn marks(spt: &Spt) -> Vec<(String, bool, std::ops::Range<u32>)> {
    spt.leaves
        .iter()
        .map(|l| (l.text.to_string(), l.is_var, l.bytes.clone()))
        .collect()
}

#[test]
fn regrouping_keeps_token_text_and_leaf_marks() {
    let spec = java();
    for (name, src) in fixtures() {
        let tree = parse(spec, &src, &Limits::DEFAULT).expect("parses");
        let globals = Globals::new(spec, declared_types(&tree, &src, spec));
        for unit in extract_units(&tree, &src, spec, &Limits::DEFAULT) {
            let plain = Spt::build(to_raw(
                Identity.normalize(unit.body.clone()),
                spec,
                &globals,
                VarMode::ContextFree,
            ))
            .expect("non-empty");
            let regrouped =
                unit_spt(&unit, spec, &globals, VarMode::ContextFree).expect("non-empty");
            // only `>>`/`>>>`/`<<` are re-tokenized, so the concatenated text is equal
            assert_eq!(
                plain.tokens(None).concat(),
                regrouped.tokens(None).concat(),
                "{name}"
            );
            let (p, r) = (marks(&plain), marks(&regrouped));
            if name == "21_method_refs" {
                // the bare receiver `seen` of `seen::contains` becomes an operand
                let flipped: Vec<&str> = p
                    .iter()
                    .zip(&r)
                    .filter(|(a, b)| a.1 != b.1)
                    .map(|(a, _)| a.0.as_str())
                    .collect();
                assert_eq!(flipped, vec!["seen"]);
                let strip = |v: &[(String, bool, std::ops::Range<u32>)]| -> Vec<(String, std::ops::Range<u32>)> {
                    v.iter().map(|(t, _, b)| (t.clone(), b.clone())).collect()
                };
                assert_eq!(strip(&p), strip(&r));
            } else {
                assert_eq!(p, r, "{name}");
            }
        }
    }
}

#[test]
fn deep_nesting_normalizes_without_recursion() {
    let src = format!(
        "class T {{ void m() {{ int x = {}y{}; }} }}",
        "f(".repeat(3000),
        ")".repeat(3000)
    );
    let spts = unit_spts(&src, VarMode::ContextFree);
    assert_eq!(spts.len(), 1);
    assert_eq!(spts[0].leaves[0].text.as_ref(), "x");
}
