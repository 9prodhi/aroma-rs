//! Stage 1 (T06): parse caps, ERROR/MISSING policy, comments, depth cap, unit extraction.

use aroma_core::lang::Lang;
use aroma_core::unit::UnitKind;
use aroma_lang::kraw::{KRaw, ERR_TOKEN};
use aroma_lang::spec::{spec_for, LangSpec};
use aroma_lang::table::{
    declared_types, extract_units, parse, to_kraw, AdapterDiag, AdapterError, Limits,
};

fn java() -> &'static LangSpec {
    spec_for(Lang::Java).unwrap()
}

fn apex() -> &'static LangSpec {
    spec_for(Lang::Apex).unwrap()
}

/// Stage-1 tree of the first method body in `src`, with its diagnostics.
fn first_body(spec: &LangSpec, src: &str, limits: &Limits) -> (KRaw, AdapterDiag) {
    let tree = parse(spec, src, limits).unwrap();
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        if n.kind() == "method_declaration" {
            let body = n.child_by_field_name("body").unwrap();
            let mut diag = AdapterDiag::default();
            let k = to_kraw(body, src.as_bytes(), spec, limits, &mut diag).unwrap();
            return (k, diag);
        }
        let mut c = n.walk();
        let kids: Vec<_> = n.children(&mut c).collect();
        stack.extend(kids.into_iter().rev());
    }
    panic!("no method in {src}");
}

/// Same tree with byte spans and lines zeroed, to compare shapes.
fn shape(k: &KRaw) -> KRaw {
    match k {
        KRaw::Token {
            kind,
            named,
            text,
            field,
            ..
        } => KRaw::Token {
            kind,
            named: *named,
            text: text.clone(),
            bytes: 0..0,
            line: 0,
            field: *field,
        },
        KRaw::Node {
            kind,
            field,
            children,
        } => KRaw::Node {
            kind,
            field: *field,
            children: children.iter().map(shape).collect(),
        },
    }
}

/// Every token in source order (explicit stack).
fn tokens(k: &KRaw) -> Vec<&KRaw> {
    let mut out = Vec::new();
    let mut stack = vec![k];
    while let Some(n) = stack.pop() {
        match n {
            KRaw::Token { .. } => out.push(n),
            KRaw::Node { children, .. } => stack.extend(children.iter().rev()),
        }
    }
    out
}

fn max_depth(k: &KRaw) -> usize {
    let mut best = 0;
    let mut stack = vec![(k, 0usize)];
    while let Some((n, d)) = stack.pop() {
        best = best.max(d);
        if let KRaw::Node { children, .. } = n {
            stack.extend(children.iter().map(|c| (c, d + 1)));
        }
    }
    best
}

fn tok(kind: &'static str, named: bool, text: &str, bytes: std::ops::Range<u32>) -> KRaw {
    KRaw::Token {
        kind,
        named,
        text: text.into(),
        bytes,
        line: 1,
        field: None,
    }
}

#[test]
fn limits_default_values() {
    let l = Limits::DEFAULT;
    assert_eq!(l.max_depth, 1000);
    assert_eq!(l.max_file_bytes, 2 * 1024 * 1024);
    assert_eq!(l.parse_timeout_micros, 2_000_000);
    assert_eq!(l.max_damage, 0.10);
}

#[test]
fn oversized_file_is_refused_before_parsing() {
    let src = "class A { void m() { } }";
    let limits = Limits {
        max_file_bytes: 10,
        ..Limits::DEFAULT
    };
    match parse(java(), src, &limits) {
        Err(AdapterError::TooLarge(n)) => assert_eq!(n, src.len()),
        other => panic!("expected TooLarge, got {other:?}"),
    }
    assert_eq!(
        AdapterError::TooLarge(24).to_string(),
        "file too large: 24 bytes"
    );
}

#[test]
fn parse_timeout_halts_the_parser() {
    let mut src = String::from("class A { void m() {\n");
    for i in 0..20_000 {
        src.push_str(&format!("  int v{i} = v{i} + {i};\n"));
    }
    src.push_str("} }\n");
    let limits = Limits {
        parse_timeout_micros: 1,
        ..Limits::DEFAULT
    };
    assert!(matches!(
        parse(java(), &src, &limits),
        Err(AdapterError::Timeout)
    ));
    // The same file parses when the budget is not the bottleneck. A generous budget keeps
    // this load-independent (the 2 s D11 default is pinned in `limits_default_values`; under
    // heavy CI load a 20k-line parse can take > 2 s of wall time for < 1 s of CPU).
    let generous = Limits {
        parse_timeout_micros: 120_000_000,
        ..Limits::DEFAULT
    };
    assert!(parse(java(), &src, &generous).is_ok());
    assert_eq!(AdapterError::Timeout.to_string(), "parse timed out");
}

#[test]
fn body_walk_keeps_tokens_kinds_and_fields() {
    let src = "class A { void m() { return x; } }";
    let (k, diag) = first_body(java(), src, &Limits::DEFAULT);
    assert_eq!(diag, AdapterDiag::default());
    let expected = KRaw::Node {
        kind: "block",
        field: None,
        children: vec![
            tok("{", false, "{", 19..20),
            KRaw::Node {
                kind: "return_statement",
                field: None,
                children: vec![
                    tok("return", false, "return", 21..27),
                    tok("identifier", true, "x", 28..29),
                    tok(";", false, ";", 29..30),
                ],
            },
            tok("}", false, "}", 31..32),
        ],
    };
    assert_eq!(k, expected);
}

#[test]
fn field_names_come_from_the_cursor() {
    let src = "class A { void m() { a.b(c); } }";
    let (k, _) = first_body(java(), src, &Limits::DEFAULT);
    let fields: Vec<(&str, Option<&str>)> = tokens(&k)
        .into_iter()
        .filter(|t| matches!(t, KRaw::Token { named: true, .. }))
        .map(|t| match t {
            KRaw::Token { text, field, .. } => (text.as_str(), *field),
            KRaw::Node { .. } => unreachable!(),
        })
        .collect();
    assert_eq!(
        fields,
        vec![("a", Some("object")), ("b", Some("name")), ("c", None)]
    );
}

#[test]
fn comments_never_become_children() {
    let plain = "class A { void m() { int x = 1; foo(x, 2); } }";
    let commented = "class A { void m() { /* a */ int x = /* b */ 1; // c\n foo(x, /* d */ 2); } }";
    let (a, da) = first_body(java(), plain, &Limits::DEFAULT);
    let (b, db) = first_body(java(), commented, &Limits::DEFAULT);
    assert_eq!(shape(&a), shape(&b));
    assert_eq!(da, db);
}

#[test]
fn error_subtree_becomes_one_err_token() {
    // tree-sitter-java wraps the statement without `;` in an ERROR node.
    let src = "class A { void m() { foo() } }";
    let (k, diag) = first_body(java(), src, &Limits::DEFAULT);
    assert_eq!(
        diag,
        AdapterDiag {
            error_nodes: 1,
            missing_nodes: 0,
            damaged_bytes: 5,
            depth_capped: false
        }
    );
    let expected = KRaw::Node {
        kind: "block",
        field: None,
        children: vec![
            tok("{", false, "{", 19..20),
            KRaw::Token {
                kind: "ERROR",
                named: false,
                text: ERR_TOKEN.into(),
                bytes: 21..26,
                line: 1,
                field: None,
            },
            tok("}", false, "}", 27..28),
        ],
    };
    assert_eq!(k, expected);
}

#[test]
fn missing_nodes_are_dropped_and_counted() {
    let src = "class A { void m() { return x } }";
    let (k, diag) = first_body(java(), src, &Limits::DEFAULT);
    assert_eq!(
        diag,
        AdapterDiag {
            error_nodes: 0,
            missing_nodes: 1,
            damaged_bytes: 0,
            depth_capped: false
        }
    );
    let texts: Vec<&str> = tokens(&k)
        .into_iter()
        .map(|t| match t {
            KRaw::Token { text, .. } => text.as_str(),
            KRaw::Node { .. } => unreachable!(),
        })
        .collect();
    assert_eq!(texts, vec!["{", "return", "x", "}"]);
    let units = extract_units(
        &parse(java(), src, &Limits::DEFAULT).unwrap(),
        src,
        java(),
        &Limits::DEFAULT,
    );
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].damage, 0.0);
}

#[test]
fn depth_cap_replaces_the_deep_subtree() {
    // block(0) > local_variable_declaration(1) > variable_declarator(2) > 1st `(`(3)
    let src = "class A { void m() { int x = (((1))); } }";
    let limits = Limits {
        max_depth: 4,
        ..Limits::DEFAULT
    };
    let (k, diag) = first_body(java(), src, &limits);
    assert!(diag.depth_capped);
    assert_eq!(diag.error_nodes, 0);
    // the 3rd parenthesized_expression (depth 5) spans `(1)`
    assert_eq!(diag.damaged_bytes, 3);
    let texts: Vec<&str> = tokens(&k)
        .into_iter()
        .map(|t| match t {
            KRaw::Token { text, .. } => text.as_str(),
            KRaw::Node { .. } => unreachable!(),
        })
        .collect();
    assert_eq!(
        texts,
        vec!["{", "int", "x", "=", "(", "(", "#ERR", ")", ")", ";", "}"]
    );
}

#[test]
fn five_thousand_levels_do_not_overflow() {
    let src = format!(
        "class A {{ void m() {{ int x = {}1{}; }} }}",
        "(".repeat(5000),
        ")".repeat(5000)
    );
    let (k, diag) = first_body(java(), &src, &Limits::DEFAULT);
    assert!(diag.depth_capped);
    assert!(max_depth(&k) <= Limits::DEFAULT.max_depth + 1);
    // the k-th parenthesized_expression sits at depth k + 2: the 999th..5000th are cut,
    // 2 * 4002 + 1 bytes
    assert_eq!(diag.damaged_bytes, 8005);
    let tree = parse(java(), &src, &Limits::DEFAULT).unwrap();
    let units = extract_units(&tree, &src, java(), &Limits::DEFAULT);
    assert_eq!(units.len(), 1);
    assert!(units[0].damage > Limits::DEFAULT.max_damage);
}

#[test]
fn java_units_in_source_order_with_lines() {
    let src = "\
class A {
  abstract void none();
  void first() {
    run();
  }
  A() { }
  int second(int p) {
    Runnable r = new Runnable() {
      public void run() { go(); }
    };
    return p;
  }
}
";
    let tree = parse(java(), src, &Limits::DEFAULT).unwrap();
    let units = extract_units(&tree, src, java(), &Limits::DEFAULT);
    let got: Vec<(UnitKind, std::ops::Range<u32>, f64)> = units
        .iter()
        .map(|u| (u.kind, u.lines.clone(), u.damage))
        .collect();
    assert_eq!(
        got,
        vec![
            (UnitKind::Method, 3..6, 0.0),
            (UnitKind::Method, 7..13, 0.0),
            (UnitKind::Method, 9..10, 0.0),
        ]
    );
    let first = &units[0];
    assert_eq!(
        &src[first.bytes.start as usize..first.bytes.end as usize],
        "void first() {\n    run();\n  }"
    );
    assert_eq!(first.body.kind(), "block");
}

#[test]
fn apex_unit_kinds() {
    let src = "\
public class A {
  public Integer size { get { return 1; } set; }
  public A() { init(); }
  public void m() { go(); }
}
";
    let tree = parse(apex(), src, &Limits::DEFAULT).unwrap();
    let kinds: Vec<UnitKind> = extract_units(&tree, src, apex(), &Limits::DEFAULT)
        .iter()
        .map(|u| u.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![UnitKind::Accessor, UnitKind::Constructor, UnitKind::Method]
    );
    let trigger =
        "trigger T on Account (before insert) { for (Account a : Trigger.new) { a.Name = 'x'; } }";
    let tree = parse(apex(), trigger, &Limits::DEFAULT).unwrap();
    let units = extract_units(&tree, trigger, apex(), &Limits::DEFAULT);
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].kind, UnitKind::Trigger);
    assert_eq!(units[0].body.kind(), "trigger_body");
}

#[test]
fn unit_damage_is_damaged_bytes_over_body_bytes() {
    let src = "class A { void m() { foo() } }";
    let tree = parse(java(), src, &Limits::DEFAULT).unwrap();
    let units = extract_units(&tree, src, java(), &Limits::DEFAULT);
    assert_eq!(units.len(), 1);
    // ERROR `foo()` = 5 bytes of the 9-byte body `{ foo() }`
    assert_eq!(units[0].damage, 5.0 / 9.0);
}

#[test]
fn declared_types_in_source_order() {
    let src = "class A { interface B { } enum C { X } record D(int x) { } }";
    let tree = parse(java(), src, &Limits::DEFAULT).unwrap();
    assert_eq!(declared_types(&tree, src, java()), vec!["A", "B", "C", "D"]);
    let src = "public class AccountService { public enum Mode { FAST } }";
    let tree = parse(apex(), src, &Limits::DEFAULT).unwrap();
    assert_eq!(
        declared_types(&tree, src, apex()),
        vec!["accountservice", "mode"]
    );
}
