//! T13: Apex normalizer (order-insensitive SELECT, stable DML labels, sObject
//! initializers, typed `when` bindings, trigger roots) and refined `LangSpec` tables.
//!
//! `sexpr` renders an SPT as `["label", child, …]` with keywords as JSON strings,
//! leaves as `{"text"}`, `#VAR` leaves as `{$"text"}` and, with bindings, `{$"text"#id}`.

use aroma_core::features::{featurize, FeatureConfig};
use aroma_core::lang::Lang;
use aroma_core::spt::{Elem, NodeId, Spt};
use aroma_lang::classify::{to_raw, unit_spt, VarMode};
use aroma_lang::globals::Globals;
use aroma_lang::kraw::KRaw;
use aroma_lang::normalize::{Identity, Normalizer};
use aroma_lang::spec::{spec_for, LangSpec};
use aroma_lang::table::{declared_types, extract_units, parse, Limits};
use std::path::PathBuf;

fn apex() -> &'static LangSpec {
    spec_for(Lang::Apex).unwrap()
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

/// SPT of every unit in `src` with the given globals.
fn spts_with(src: &str, globals: &Globals, mode: VarMode) -> Vec<Spt> {
    let spec = apex();
    let tree = parse(spec, src, &Limits::DEFAULT).expect("parses");
    extract_units(&tree, src, spec, &Limits::DEFAULT)
        .iter()
        .map(|u| unit_spt(u, spec, globals, mode).expect("non-empty unit"))
        .collect()
}

/// SPT of every unit in `src` (globals = builtins + the file's declared types).
fn unit_spts(src: &str, mode: VarMode) -> Vec<Spt> {
    let spec = apex();
    let tree = parse(spec, src, &Limits::DEFAULT).expect("parses");
    let globals = Globals::new(spec, declared_types(&tree, src, spec));
    spts_with(src, &globals, mode)
}

/// Default-mode `sexpr` (with SymbolIds) of method `m` of class `OrderService`.
fn body(body: &str) -> String {
    let src = format!("public class OrderService {{ public void m() {{ {body} }} }}");
    let spts = unit_spts(&src, VarMode::ContextFree);
    sexpr(spts.first().expect("one unit"), true)
}

// ---------------------------------------------------------------- Step group 1

#[test]
fn select_items_are_sorted_and_commas_reinterleaved() {
    assert_eq!(
        body("List<Account> r = [SELECT Name, Owner.Name, Id FROM Account];"),
        r####"["{#}", "{", ["##;", ["##", {"list"}, ["<#>", "<", {"account"}, ">"]], ["#=#", {$"r"#0}, "=", ["[#]", "[", ["##", ["select#,#,#", "select", {"id"}, ",", {"name"}, ",", ["#.#", {"owner"}, ".", {"name"}]], ["from#", "from", {"account"}]], "]"]], ";"], "}"]"####
    );
}

#[test]
fn select_order_does_not_change_the_tree_or_features() {
    let a = "public class A { public void m() { List<Contact> cs = [SELECT LastName, Email, Id FROM Contact WHERE AccountId = :acc]; } }";
    let b = "public class A { public void m() { List<Contact> cs = [SELECT Id, LastName, Email FROM Contact WHERE AccountId = :acc]; } }";
    let (sa, sb) = (
        unit_spts(a, VarMode::ContextFree),
        unit_spts(b, VarMode::ContextFree),
    );
    assert_eq!(sexpr(&sa[0], true), sexpr(&sb[0], true));
    assert_eq!(
        featurize(&sa[0], &FeatureConfig::DEFAULT, None),
        featurize(&sb[0], &FeatureConfig::DEFAULT, None)
    );
}

fn tok(kind: &'static str, named: bool, text: &str, at: u32) -> KRaw {
    KRaw::Token {
        kind,
        named,
        text: text.into(),
        bytes: at..at + text.len() as u32,
        line: 1,
        field: None,
    }
}

fn field(name: &str, at: u32) -> KRaw {
    KRaw::Node {
        kind: "field_identifier",
        field: None,
        children: vec![tok("identifier", true, name, at)],
    }
}

#[test]
fn damaged_select_keeps_its_source_order() {
    let select = KRaw::Node {
        kind: "select_clause",
        field: Some("select_clause"),
        children: vec![
            tok("SELECT", false, "SELECT", 0),
            field("Name", 7),
            tok(",", false, ",", 11),
            tok("ERROR", false, "#ERR", 13),
            tok(",", false, ",", 16),
            field("Id", 18),
        ],
    };
    assert_eq!(apex().normalizer.normalize(select.clone()), select);
    let clean = KRaw::Node {
        kind: "select_clause",
        field: None,
        children: vec![
            tok("SELECT", false, "SELECT", 0),
            field("Name", 7),
            tok(",", false, ",", 11),
            field("Id", 13),
        ],
    };
    assert_eq!(
        apex().normalizer.normalize(clean),
        KRaw::Node {
            kind: "select_clause",
            field: None,
            children: vec![
                tok("SELECT", false, "SELECT", 0),
                field("Id", 13),
                tok(",", false, ",", 11),
                field("Name", 7),
            ],
        }
    );
}

#[test]
fn dml_labels_are_stable() {
    assert_eq!(
        body("insert rows; INSERT rows; Insert rows; upsert rows Account.Id; insert as user rows;"),
        r####"["{#####}", "{", ["#;", ["insert#", "insert", {$"rows"}], ";"], ["#;", ["insert#", "insert", {$"rows"}], ";"], ["#;", ["insert#", "insert", {$"rows"}], ";"], ["#;", ["upsert##", "upsert", {$"rows"}, ["#.#", {"account"}, ".", {"id"}]], ";"], ["#;", ["insertasuser#", "insert", "as", "user", {$"rows"}], ";"], "}"]"####
    );
}

#[test]
fn dml_expression_children_are_flat() {
    let src = "public class A { public void m() { delete as system stale; } }";
    let spec = apex();
    let tree = parse(spec, src, &Limits::DEFAULT).expect("parses");
    let unit = extract_units(&tree, src, spec, &Limits::DEFAULT)
        .into_iter()
        .next()
        .expect("one unit");
    let normalized = spec.normalizer.normalize(unit.body);
    let mut stack = vec![&normalized];
    let mut dml = None;
    while let Some(k) = stack.pop() {
        if let KRaw::Node { kind, children, .. } = k {
            if *kind == "dml_expression" {
                dml = Some(children);
                break;
            }
            stack.extend(children.iter());
        }
    }
    let texts: Vec<String> = dml
        .expect("a dml_expression")
        .iter()
        .map(|c| match c {
            KRaw::Token { text, .. } => text.clone(),
            KRaw::Node { kind, .. } => format!("<{kind}>"),
        })
        .collect();
    assert_eq!(texts, vec!["delete", "as", "system", "stale"]);
}

#[test]
fn binds_join_usage_chains() {
    assert_eq!(
        body("Id x = y; List<Account> r = [SELECT Id FROM Account WHERE Id = :x]; update r;"),
        r####"["{###}", "{", ["##;", {"id"}, ["#=#", {$"x"#0}, "=", {$"y"}], ";"], ["##;", ["##", {"list"}, ["<#>", "<", {"account"}, ">"]], ["#=#", {$"r"#1}, "=", ["[#]", "[", ["###", ["select#", "select", {"id"}], ["from#", "from", {"account"}], ["where#", "where", ["#=#", {"id"}, "=", [":#", ":", {$"x"#0}]]]], "]"]], ";"], ["#;", ["update#", "update", {$"r"#1}], ";"], "}"]"####
    );
}

#[test]
fn sobject_initializer_names_are_fields() {
    assert_eq!(
        body("Account a = new Account(Name = n);"),
        r####"["{#}", "{", ["##;", {"account"}, ["#=#", {$"a"#0}, "=", ["new##", "new", {"account"}, ["(#)", "(", ["#=#", {"name"}, "=", {$"n"}], ")"]]], ";"], "}"]"####
    );
}

#[test]
fn trigger_root_is_the_block() {
    let src =
        "trigger T on Account (before insert) { for (Account a : Trigger.new) { a.Name = 'x'; } }";
    let spts = unit_spts(src, VarMode::ContextFree);
    assert_eq!(
        sexpr(&spts[0], true),
        r####"["{#}", "{", ["for(##:#)#", "for", "(", {"account"}, {$"a"#0}, ":", ["#.#", {"trigger"}, ".", {"new"}], ")", ["{#}", "{", ["#;", ["#=#", ["#.#", {$"a"#0}, ".", {"name"}], "=", {"'x'"}], ";"], "}"]], "}"]"####
    );
}

#[test]
fn mixed_case_sources_featurize_identically() {
    let lower = "public class Svc { public void sync(list<account> accts) { for (account a : [select name, id from account where id in :accts]) { a.name = 'Keep Case'; } update accts; } }";
    let mixed = "PUBLIC CLASS svc { Public Void Sync(List<Account> ACCTS) { FOR (Account A : [SELECT Id, Name FROM ACCOUNT WHERE ID IN :Accts]) { a.NAME = 'Keep Case'; } UPDATE Accts; } }";
    let (a, b) = (
        unit_spts(lower, VarMode::ContextFree),
        unit_spts(mixed, VarMode::ContextFree),
    );
    assert_eq!(sexpr(&a[0], true), sexpr(&b[0], true));
    assert_eq!(
        featurize(&a[0], &FeatureConfig::DEFAULT, None),
        featurize(&b[0], &FeatureConfig::DEFAULT, None)
    );
    // string literals are never folded
    assert!(a[0].leaves.iter().any(|l| &*l.text == "'Keep Case'"));
}

// ---------------------------------------------------------------- Step group 2

#[test]
fn typed_when_binds_a_symbol() {
    assert_eq!(
        body("switch on rec { when Account a { a.Name = n; } }"),
        r####"["{#}", "{", ["switchon##", "switch", "on", {$"rec"}, ["{#}", "{", ["##", ["when#", "when", ["##", {"account"}, {$"a"#0}]], ["{#}", "{", ["#;", ["#=#", ["#.#", {$"a"#0}, ".", {"name"}], "=", {$"n"}], ";"], "}"]], "}"]], "}"]"####
    );
}

#[test]
fn boolean_literals_are_single_leaves() {
    assert_eq!(
        body("Boolean b = TRUE;"),
        r####"["{#}", "{", ["##;", {"boolean"}, ["#=#", {$"b"#0}, "=", {"true"}], ";"], "}"]"####
    );
}

#[test]
fn soql_function_names_are_not_vars() {
    assert_eq!(
        body("List<AggregateResult> r = [SELECT COUNT(Id) c FROM Account];"),
        r####"["{#}", "{", ["##;", ["##", {"list"}, ["<#>", "<", {"aggregateresult"}, ">"]], ["#=#", {$"r"#0}, "=", ["[#]", "[", ["##", ["select#", "select", ["##", ["#(#)", {"count"}, "(", {"id"}, ")"], {"c"}]], ["from#", "from", {"account"}]], "]"]], ";"], "}"]"####
    );
}

#[test]
fn static_receiver_of_a_repo_class_stays_literal_only_when_global() {
    let src = "public class OrderService { public void m() { OrderService.submit(orders); Billing.charge(orders); } }";
    let with_repo = Globals::new(apex(), vec!["OrderService".to_string()]);
    let builtins_only = Globals::new(apex(), Vec::<String>::new());
    let marks = |g: &Globals| -> Vec<(String, bool)> {
        spts_with(src, g, VarMode::ContextFree)[0]
            .leaves
            .iter()
            .map(|l| (l.text.to_string(), l.is_var))
            .collect()
    };
    let owned = |v: &[(&str, bool)]| -> Vec<(String, bool)> {
        v.iter().map(|(t, b)| (t.to_string(), *b)).collect()
    };
    assert_eq!(
        marks(&with_repo),
        owned(&[
            ("orderservice", false),
            ("submit", false),
            ("orders", true),
            ("billing", true),
            ("charge", false),
            ("orders", true),
        ])
    );
    assert_eq!(
        marks(&builtins_only),
        owned(&[
            ("orderservice", true),
            ("submit", false),
            ("orders", true),
            ("billing", true),
            ("charge", false),
            ("orders", true),
        ])
    );
}

#[test]
fn spec_records_the_normalizer_version() {
    assert_eq!(apex().adapter_version, "apex-1");
    assert!(apex().atomic_kinds.contains(&"boolean"));
    assert!(apex().decl_name_parents.contains(&"when_sobject_type"));
}

// ---------------------------------------------------------------- Step group 3

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/apex")
}

/// (file name, text) of every `.cls` / `.trigger` fixture, sorted by name.
fn fixtures() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(fixture_dir())
        .expect("fixture dir")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "cls" || e == "trigger"))
        .map(|p| {
            let name = p.file_name().expect("name").to_string_lossy().into_owned();
            (name, std::fs::read_to_string(&p).expect("fixture text"))
        })
        .collect();
    out.sort();
    out
}

/// Snapshot text: per unit the default (D18) tree with SymbolIds and the default
/// features of every leaf.
fn describe(src: &str) -> String {
    let mut out = String::new();
    for (i, spt) in unit_spts(src, VarMode::ContextFree).iter().enumerate() {
        out.push_str(&format!(
            "unit {i}\ntree: {}\nfeatures:\n",
            sexpr(spt, true)
        ));
        let per_leaf = featurize(spt, &FeatureConfig::DEFAULT, None);
        for (leaf, feats) in spt.leaves.iter().zip(per_leaf) {
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
    settings.set_snapshot_path("fixtures/apex/snapshots");
    settings.set_prepend_module_to_snapshot(false);
    settings.set_omit_expression(true);
    settings.bind(|| insta::assert_snapshot!(name, text));
}

#[test]
fn twenty_five_construct_fixtures_match_their_snapshots() {
    let all = fixtures();
    assert_eq!(
        all.len(),
        25,
        "{:?}",
        all.iter().map(|f| &f.0).collect::<Vec<_>>()
    );
    for (name, src) in &all {
        let stem = name.rsplit_once('.').map_or(name.as_str(), |(s, _)| s);
        snapshot(&format!("apex_{stem}"), &describe(src));
    }
}

// ---------------------------------------------------------------- Step group 4

/// (text, is_var, bytes) of every leaf, ordered by source position.
fn marks_by_position(spt: &Spt) -> Vec<(String, bool, std::ops::Range<u32>)> {
    let mut v: Vec<(String, bool, std::ops::Range<u32>)> = spt
        .leaves
        .iter()
        .map(|l| (l.text.to_string(), l.is_var, l.bytes.clone()))
        .collect();
    v.sort_by_key(|m| (m.2.start, m.2.end));
    v
}

#[test]
fn normalizer_only_permutes_select_items_and_anchors_field_names() {
    let spec = apex();
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
            let normalized =
                unit_spt(&unit, spec, &globals, VarMode::ContextFree).expect("non-empty");
            let (p, n) = (marks_by_position(&plain), marks_by_position(&normalized));
            assert_eq!(p.len(), n.len(), "{name}");
            let flipped: Vec<&str> = p
                .iter()
                .zip(&n)
                .filter(|(a, b)| a != b)
                .map(|(a, b)| {
                    assert_eq!((&a.0, &a.2), (&b.0, &b.2), "{name}");
                    a.0.as_str()
                })
                .collect();
            // only sObject initializer names (`new Account(Name = ..)`) stop being vars
            let expected: &[&str] = match name.as_str() {
                "22_custom_object_fields.cls" | "23_test_method.cls" => &["name"],
                _ => &[],
            };
            assert_eq!(flipped, expected, "{name}");
        }
    }
}

#[test]
fn deep_nesting_normalizes_without_recursion() {
    let src = format!(
        "public class A {{ public void m() {{ Integer x = {}y{}; }} }}",
        "f(".repeat(3000),
        ")".repeat(3000)
    );
    let spts = unit_spts(&src, VarMode::ContextFree);
    assert_eq!(spts.len(), 1);
    assert_eq!(spts[0].leaves[0].text.as_ref(), "integer");
}
