//! Partial-query wrapping (T07): wrapper choice, damage, rebased spans, D18 invariant.

use aroma_core::features::{featurize, FeatureConfig};
use aroma_core::lang::Lang;
use aroma_core::spt::Spt;
use aroma_lang::classify::{unit_spt, VarMode};
use aroma_lang::globals::Globals;
use aroma_lang::kraw::KRaw;
use aroma_lang::snippet::parse_snippet;
use aroma_lang::spec::{spec_for, LangSpec};
use aroma_lang::table::{declared_types, extract_units, parse, Limits};

fn java() -> &'static LangSpec {
    spec_for(Lang::Java).unwrap()
}

fn apex() -> &'static LangSpec {
    spec_for(Lang::Apex).unwrap()
}

fn snippet(spec: &LangSpec, text: &str) -> aroma_lang::snippet::SnippetParse {
    let globals = Globals::new(spec, Vec::<String>::new());
    parse_snippet(spec, text, &globals, VarMode::ContextFree, &Limits::DEFAULT)
}

/// (text, bytes, line) of every leaf.
fn leaf_spans(spt: &Spt) -> Vec<(String, std::ops::Range<u32>, u32)> {
    spt.leaves
        .iter()
        .map(|l| (l.text.to_string(), l.bytes.clone(), l.line))
        .collect()
}

#[test]
fn statements_snippet_keeps_the_method_body_root() {
    let text = "int s = a + b;\nreturn s;";
    let p = snippet(java(), text);
    assert_eq!(p.wrapper, "statements");
    assert_eq!(p.damage, 0.0);
    let spt = p.spt.unwrap();
    assert_eq!(spt.label(spt.root), "{##}");
    assert_eq!(
        spt.tokens(None),
        vec!["{", "int", "s", "=", "a", "+", "b", ";", "return", "s", ";", "}"]
    );
    assert_eq!(
        leaf_spans(&spt),
        vec![
            ("s".to_string(), 4..5, 1),
            ("a".to_string(), 8..9, 1),
            ("b".to_string(), 12..13, 1),
            ("s".to_string(), 22..23, 2),
        ]
    );
    let s: Vec<_> = spt.leaves.iter().map(|l| (l.is_var, l.binding)).collect();
    assert_eq!(s[0].1, s[3].1);
    assert!(s[0].1.is_some());
    assert_eq!(s[1], (true, None)); // undeclared in the snippet
}

#[test]
fn unclosed_block_is_closed_by_the_wrapper() {
    let p = snippet(java(), "if (ready) {\n  run(job);");
    assert_eq!(p.wrapper, "statements");
    assert_eq!(p.damage, 0.0);
    assert_eq!(
        p.spt.unwrap().tokens(None),
        vec!["{", "if", "(", "ready", ")", "{", "run", "(", "job", ")", ";", "}", "}"]
    );
}

#[test]
fn nested_unclosed_blocks_are_closed_before_wrapping() {
    // Pasted partial code usually leaves several blocks open (smoke test, 2026-11-20).
    let open = "List<Database.SaveResult> rs = Database.insert(rows, false);\n\
                for (Database.SaveResult r : rs) {\n    if (!r.isSuccess()) {\n        log(r);";
    let closed = format!("{open}\n    }}\n}}");
    let p = snippet(apex(), open);
    assert_eq!(p.wrapper, "statements");
    assert_eq!(p.damage, 0.0);
    let q = snippet(apex(), &closed);
    let (a, b) = (p.spt.unwrap(), q.spt.unwrap());
    assert_eq!(a.tokens(None), b.tokens(None));
    let cfg = FeatureConfig::DEFAULT;
    assert_eq!(featurize(&a, &cfg, None), featurize(&b, &cfg, None));

    // Java: an open call argument list inside two open blocks.
    let p = snippet(
        java(),
        "while (it.hasNext()) {\n  if (x > 0) {\n    total += f(x",
    );
    assert_eq!(p.damage, 0.0);
    assert!(p.spt.is_some());
}

#[test]
fn an_unclosed_try_is_closed_with_an_empty_finally() {
    // A try needs catch/finally; `try { .. }` alone is a syntax error (Java recall misses,
    // 2026-11-20: 10 of 12 ended inside an unclosed try).
    let java_text =
        "ZipFile zf = null;\ntry {\n    zf = new ZipFile(f);\n    if (zf.size() > 0) {\n        use(zf);";
    let p = snippet(java(), java_text);
    assert_eq!(p.wrapper, "statements");
    assert_eq!(p.damage, 0.0);
    let spt = p.spt.unwrap();
    let t = spt.tokens(None);
    assert!(t.contains(&"finally"), "{t:?}");
    let apex_text = "Savepoint sp = Database.setSavepoint();\nTRY {\n    insert rows;";
    let p = snippet(apex(), apex_text);
    assert_eq!(p.damage, 0.0);
    // a word merely ending in "try" is not the keyword
    let p = snippet(java(), "retry {\n  x();");
    let has_finally = p.spt.is_some_and(|s| s.tokens(None).contains(&"finally"));
    assert!(!has_finally);
}

#[test]
fn a_snippet_cut_inside_a_construct_keeps_its_complete_lines() {
    // Five lines copied from a method, ending inside a SOQL query (recall miss, 2026-11-20).
    let text = "// Instantiate the RestResponse\nRestResponse response = RestContext.response;\n\
                try {\n    List<Account> accounts = [\n        SELECT Id, Name, Phone, Website";
    let p = snippet(apex(), text);
    let spt = p
        .spt
        .expect("the complete leading statements still form a tree");
    let tokens = spt.tokens(None);
    assert!(tokens.contains(&"restcontext"), "{tokens:?}");
    // the dropped tail is user text that was not used: it still counts as damage
    assert!(p.damage > 0.0 && p.damage < 1.0, "damage {}", p.damage);
    assert!(spt
        .leaves
        .iter()
        .all(|l| (l.bytes.end as usize) <= text.len()));

    // A snippet that parses as a whole is never trimmed.
    let whole = snippet(apex(), "Integer n = 1;\nn++;");
    assert_eq!(whole.damage, 0.0);
    assert_eq!(
        whole.spt.unwrap().leaves.iter().map(|l| l.line).max(),
        Some(2)
    );
}

#[test]
fn brackets_in_strings_and_comments_are_not_counted() {
    let p = snippet(
        apex(),
        "String s = '{ ( [';\n// } ) ]\n/* { */ if (s != null) {\n  run(s);",
    );
    assert_eq!(p.wrapper, "statements");
    assert_eq!(p.damage, 0.0);
    let p = snippet(
        java(),
        "String s = \"{\\\"(\";\nchar c = '{';\nif (ok) {\n  go(s, c);",
    );
    assert_eq!(p.damage, 0.0);
}

#[test]
fn full_method_uses_the_member_wrapper() {
    let text =
        "/** Adds. */\n@Override\npublic int add(int a, int b) {\n  int s = a + b;\n  return s;\n}";
    let p = snippet(java(), text);
    assert_eq!(p.wrapper, "member");
    assert_eq!(p.damage, 0.0);
    let spt = p.spt.unwrap();
    assert_eq!(
        spt.tokens(None),
        vec!["{", "int", "s", "=", "a", "+", "b", ";", "return", "s", ";", "}"]
    );
    assert_eq!(spt.leaves[0].line, 4);
    let at = spt.leaves[0].bytes.clone();
    assert_eq!(&text[at.start as usize..at.end as usize], "s");

    // tree-sitter-sfapex accepts a method as a statement; the member wrapper still wins
    let apex_text = "public void reset(List<Account> rows) {\n  rows.clear();\n}";
    let p = snippet(apex(), apex_text);
    assert_eq!(p.wrapper, "member");
    assert_eq!(
        p.spt.unwrap().tokens(None),
        vec!["{", "rows", ".", "clear", "(", ")", ";", "}"]
    );
}

#[test]
fn expression_snippet_drops_the_wrapper_parentheses() {
    let p = snippet(java(), "count++");
    assert_eq!(p.wrapper, "expression");
    assert_eq!(p.damage, 0.0);
    let spt = p.spt.unwrap();
    assert_eq!(spt.label(spt.root), "#++");
    assert_eq!(spt.tokens(None), vec!["count", "++"]);
    assert_eq!(leaf_spans(&spt), vec![("count".to_string(), 0..5, 1)]);

    let p = snippet(apex(), "count++");
    assert_eq!(p.wrapper, "expression");
    assert_eq!(p.spt.unwrap().tokens(None), vec!["count", "++"]);
}

#[test]
fn unbalanced_braces_report_damage() {
    let p = snippet(java(), "foo(); } bar();");
    assert!(p.damage > Limits::DEFAULT.max_damage, "{p:?}");
    assert!(p.damage <= 1.0);
    assert_eq!(p.wrapper, "statements");
    // `bar();` ends up outside the wrapper method: 6 of 15 bytes are damaged
    assert_eq!(p.damage, 6.0 / 15.0);
    assert_eq!(
        p.spt.unwrap().tokens(None),
        vec!["{", "foo", "(", ")", ";", "}"]
    );
}

#[test]
fn empty_and_comment_only_text_have_no_tree() {
    for text in ["", "   \n", "// only a comment", "/* a */ // b"] {
        for spec in [java(), apex()] {
            let p = snippet(spec, text);
            assert!(p.spt.is_none(), "{text:?}");
            assert_eq!(p.damage, 0.0, "{text:?}");
            assert_eq!(p.wrapper, "none", "{text:?}");
        }
    }
}

#[test]
fn apex_statements_with_soql_are_folded() {
    let text = "List<Account> rows = [SELECT Id FROM Account WHERE Name = :n];\nupdate rows;";
    let p = snippet(apex(), text);
    assert_eq!(p.wrapper, "statements");
    let spt = p.spt.unwrap();
    let marks: Vec<(String, bool)> = spt
        .leaves
        .iter()
        .map(|l| (l.text.to_string(), l.is_var))
        .collect();
    let want: Vec<(String, bool)> = [
        ("list", false),
        ("account", false),
        ("rows", true),
        ("id", false),
        ("account", false),
        ("name", false),
        ("n", true),
        ("rows", true),
    ]
    .iter()
    .map(|(t, v)| (t.to_string(), *v))
    .collect();
    assert_eq!(marks, want);
}

/// Byte span covered by a stage-1 tree (first to last token).
fn kraw_span(k: &KRaw) -> std::ops::Range<usize> {
    let mut lo = usize::MAX;
    let mut hi = 0;
    let mut stack = vec![k];
    while let Some(n) = stack.pop() {
        match n {
            KRaw::Token { bytes, .. } => {
                lo = lo.min(bytes.start as usize);
                hi = hi.max(bytes.end as usize);
            }
            KRaw::Node { children, .. } => stack.extend(children.iter()),
        }
    }
    lo..hi
}

/// D18 invariant: every unit body pasted as a snippet featurizes exactly like the
/// indexed unit.
fn assert_query_path_equals_index_path(spec: &LangSpec, src: &str, expected_units: usize) {
    let tree = parse(spec, src, &Limits::DEFAULT).unwrap();
    let globals = Globals::new(spec, declared_types(&tree, src, spec));
    let units = extract_units(&tree, src, spec, &Limits::DEFAULT);
    assert_eq!(units.len(), expected_units);
    for unit in &units {
        let indexed = unit_spt(unit, spec, &globals, VarMode::ContextFree).unwrap();
        let body = &src[kraw_span(&unit.body)];
        let q = parse_snippet(spec, body, &globals, VarMode::ContextFree, &Limits::DEFAULT);
        assert_eq!(q.wrapper, "statements", "{body}");
        assert_eq!(q.damage, 0.0, "{body}");
        let queried = q.spt.unwrap();
        assert_eq!(queried.tokens(None), indexed.tokens(None), "{body}");
        let marks = |s: &Spt| -> Vec<(String, bool, Option<u32>)> {
            s.leaves
                .iter()
                .map(|l| (l.text.to_string(), l.is_var, l.binding.map(|b| b.0)))
                .collect()
        };
        assert_eq!(marks(&queried), marks(&indexed), "{body}");
        assert_eq!(
            featurize(&queried, &FeatureConfig::DEFAULT, None),
            featurize(&indexed, &FeatureConfig::DEFAULT, None),
            "{body}"
        );
    }
}

#[test]
fn d18_invariant_java_methods() {
    let src = r#"
package demo;

class Inventory {
    private final java.util.Map<String, Integer> stock = new java.util.HashMap<>();

    int total(java.util.List<Integer> xs) {
        int sum = 0;
        for (int x : xs) {
            if (x > 0) { sum += x; }
        }
        return sum;
    }

    String describe(Object o) {
        StringBuilder sb = new StringBuilder();
        try {
            sb.append(o.toString()).append(":").append(stock.size());
        } catch (RuntimeException e) {
            System.err.println(e.getMessage());
        }
        return sb.toString();
    }

    void restock(String item, int n) {
        Integer have = stock.get(item);
        { int n2 = n * 2; stock.put(item, have == null ? n2 : have + n2); }
        { int n2 = n; Inventory.log(item, n2); }
    }

    static void log(String s, int n) { }
}
"#;
    assert_query_path_equals_index_path(java(), src, 4);
}

#[test]
fn d18_invariant_apex_methods() {
    let src = r#"
public with sharing class AccountService {
    public static void touch(List<Account> rows) {
        for (Account a : rows) {
            a.Description = 'Touched ' + a.Name;
        }
        UPDATE rows;
    }

    public List<Contact> contactsFor(Id acctId) {
        List<Contact> cs = [SELECT Id, LastName FROM Contact WHERE AccountId = :acctId];
        if (cs.isEmpty()) { System.debug(LoggingLevel.WARN, 'none'); }
        return cs;
    }

    public Integer countOpen(Set<Id> ids) {
        Integer N = 0;
        for (Case c : [SELECT Id, Status FROM Case WHERE Id IN :ids]) {
            if (c.Status != 'Closed') { n++; }
        }
        AccountService.touch(new List<Account>());
        return n;
    }
}
"#;
    assert_query_path_equals_index_path(apex(), src, 3);
}
