//! Stage 3 (T06): D18 `#VAR` classification, SymbolIds, Apex case folding, globals.

use aroma_core::features::{featurize, FeatureConfig};
use aroma_core::lang::Lang;
use aroma_core::spt::{Raw, Spt};
use aroma_lang::classify::{to_raw, unit_spt, VarMode};
use aroma_lang::globals::Globals;
use aroma_lang::kraw::KRaw;
use aroma_lang::spec::{spec_for, LangSpec};
use aroma_lang::table::{declared_types, extract_units, parse, Limits};

fn java() -> &'static LangSpec {
    spec_for(Lang::Java).unwrap()
}

fn apex() -> &'static LangSpec {
    spec_for(Lang::Apex).unwrap()
}

/// SPT of the first unit in `src`, globals = builtins + the file's declared types.
fn first_spt(spec: &LangSpec, src: &str, mode: VarMode) -> Spt {
    let tree = parse(spec, src, &Limits::DEFAULT).unwrap();
    let globals = Globals::new(spec, declared_types(&tree, src, spec));
    let units = extract_units(&tree, src, spec, &Limits::DEFAULT);
    unit_spt(&units[0], spec, &globals, mode).unwrap()
}

/// (text, is_var) of every leaf in source order.
fn marks(spt: &Spt) -> Vec<(String, bool)> {
    spt.leaves
        .iter()
        .map(|l| (l.text.to_string(), l.is_var))
        .collect()
}

/// (text, binding) of every `#VAR` leaf in source order.
fn bindings(spt: &Spt) -> Vec<(String, Option<u32>)> {
    spt.leaves
        .iter()
        .filter(|l| l.is_var)
        .map(|l| (l.text.to_string(), l.binding.map(|b| b.0)))
        .collect()
}

fn owned(v: &[(&str, bool)]) -> Vec<(String, bool)> {
    v.iter().map(|(t, b)| (t.to_string(), *b)).collect()
}

#[test]
fn java_d18_classification_table() {
    let src = "\
class Repo {
  void m(int p) {
    int x = p;
    x.foo(y);
    System.out.println(x);
    Repo.make();
    Other.make();
    this.f = g;
    String s = \"Abc\";
    if (flag) { return; }
  }
}
";
    let spt = first_spt(java(), src, VarMode::ContextFree);
    assert_eq!(
        marks(&spt),
        owned(&[
            ("x", true),        // declared local
            ("p", true),        // parameter use: expression position
            ("x", true),        // receiver
            ("foo", false),     // method name
            ("y", true),        // argument
            ("System", false),  // builtin global receiver
            ("out", false),     // accessed field
            ("println", false), // method name
            ("x", true),
            ("Repo", false), // repository type
            ("make", false),
            ("Other", true), // unknown receiver: D18 has no upper-case exemption
            ("make", false),
            ("f", false), // this.f
            ("g", true),
            ("String", false), // type_identifier
            ("s", true),
            ("\"Abc\"", false), // literal
            ("flag", true),
        ])
    );
}

#[test]
fn java_compat_marking_ignores_globals() {
    let src = "class Repo { void m() { System.out.println(x); Repo.make(); this.f = g; } }";
    let spt = first_spt(java(), src, VarMode::Compat);
    assert_eq!(
        marks(&spt),
        owned(&[
            ("System", true), // reference: primary IDENTIFIER; similar.py un-vars upper-case
            ("out", false),
            ("println", false),
            ("x", true),
            ("Repo", true),
            ("make", false),
            ("f", false),
            ("g", true),
        ])
    );
    assert!(spt.leaves.iter().all(|l| l.binding.is_none()));
}

#[test]
fn java_literals_and_keywords() {
    let src =
        "class A { void m() { Object o = null; boolean t = true; char c = 'q'; long n = 0x1F; } }";
    let spt = first_spt(java(), src, VarMode::ContextFree);
    assert_eq!(
        marks(&spt),
        owned(&[
            ("Object", false),
            ("o", true),
            ("t", true),
            ("true", false),
            ("c", true),
            ("'q'", false),
            ("n", true),
            ("0x1F", false),
        ])
    );
    let toks = spt.tokens(None);
    assert!(toks.contains(&"null"), "{toks:?}");
    assert!(toks.contains(&"boolean"), "{toks:?}");
}

#[test]
fn apex_soql_names_are_never_vars_and_binds_are() {
    let src = "\
public class Repo {
  public void m(Id acctId) {
    List<Account> rows = [SELECT Id, Name FROM Account WHERE Id = :acctId];
    for (Account a : rows) { System.debug(a.Name); }
    Repo.run(rows);
  }
}
";
    let spt = first_spt(apex(), src, VarMode::ContextFree);
    assert_eq!(
        marks(&spt),
        owned(&[
            ("list", false),
            ("account", false),
            ("rows", true),
            ("id", false),      // field_identifier
            ("name", false),    // field_identifier
            ("account", false), // storage_identifier
            ("id", false),
            ("acctid", true), // :bind
            ("account", false),
            ("a", true),
            ("rows", true),
            ("system", false),
            ("debug", false),
            ("a", true),
            ("name", false),
            ("repo", false), // repository type (folded)
            ("run", false),
            ("rows", true),
        ])
    );
}

#[test]
fn in_unit_declaration_beats_known_global() {
    // D28 (amends D18): `pattern` is in APEX_BUILTIN_GLOBALS, but the local declared here
    // is #VAR at its declaration and at its SOQL bind, with one binding. The accessed
    // field `obj.pattern` stays a non-var (rule 2 comes before resolution).
    let src = "\
public class A {
  public void m(Holder obj) {
    String pattern = 'a%';
    List<Account> xs = [SELECT Id FROM Account WHERE Name LIKE :pattern];
    System.debug(obj.pattern);
  }
}
";
    let spt = first_spt(apex(), src, VarMode::ContextFree);
    let pattern: Vec<(bool, Option<u32>)> = spt
        .leaves
        .iter()
        .filter(|l| &*l.text == "pattern")
        .map(|l| (l.is_var, l.binding.map(|b| b.0)))
        .collect();
    assert_eq!(pattern, [(true, Some(0)), (true, Some(0)), (false, None)]);
    // Without an in-unit declaration the same bind is a known global, not a var.
    let undeclared = "public class A { public void m() { List<Account> xs = [SELECT Id FROM Account WHERE Name LIKE :pattern]; } }";
    let spt = first_spt(apex(), undeclared, VarMode::ContextFree);
    let pattern: Vec<bool> = spt
        .leaves
        .iter()
        .filter(|l| &*l.text == "pattern")
        .map(|l| l.is_var)
        .collect();
    assert_eq!(pattern, [false]);
}

#[test]
fn apex_case_folding_spares_strings() {
    let src = "\
public class A {
  public void m() {
    Integer ACC = 1;
    ACC = acc + Acc;
    String s = 'Abc';
    IF (ACC > 0) { RETURN; }
  }
}
";
    let spt = first_spt(apex(), src, VarMode::ContextFree);
    let vars = bindings(&spt);
    assert_eq!(
        vars,
        vec![
            ("acc".to_string(), Some(0)),
            ("acc".to_string(), Some(0)),
            ("acc".to_string(), Some(0)),
            ("acc".to_string(), Some(0)),
            ("s".to_string(), Some(1)),
            ("acc".to_string(), Some(0)),
        ]
    );
    let toks = spt.tokens(None);
    assert!(toks.contains(&"'Abc'"), "{toks:?}");
    assert!(toks.contains(&"if"), "{toks:?}");
    assert!(toks.contains(&"return"), "{toks:?}");
    assert!(!toks
        .iter()
        .any(|t| t.chars().any(|c| c.is_ascii_uppercase()) && *t != "'Abc'"));
}

#[test]
fn apex_case_variants_featurize_identically() {
    let lower = "public class A { public void m() { integer total = 0; for (account a : accts) { total += a.amount; } } }";
    let upper = "PUBLIC CLASS A { PUBLIC VOID M() { INTEGER TOTAL = 0; FOR (ACCOUNT A : ACCTS) { TOTAL += A.AMOUNT; } } }";
    let a = first_spt(apex(), lower, VarMode::ContextFree);
    let b = first_spt(apex(), upper, VarMode::ContextFree);
    assert_eq!(
        featurize(&a, &FeatureConfig::DEFAULT, None),
        featurize(&b, &FeatureConfig::DEFAULT, None)
    );
}

#[test]
fn shadowed_names_get_distinct_symbol_ids() {
    let src = "\
class A {
  void m(int p) {
    { int x = 1; use(x); }
    { int x = 2; use(x, p); }
    for (int i = 0; i < 3; i++) { use(i); }
    for (int i = 0; i < 3; i++) { use(i); }
    try { use(p); } catch (Exception e) { use(e); }
  }
}
";
    let spt = first_spt(java(), src, VarMode::ContextFree);
    let got = bindings(&spt);
    let want: Vec<(String, Option<u32>)> = [
        ("x", Some(0)),
        ("x", Some(0)),
        ("x", Some(1)),
        ("x", Some(1)),
        ("p", None), // parameter: declared outside the unit body
        ("i", Some(2)),
        ("i", Some(2)),
        ("i", Some(2)),
        ("i", Some(2)),
        ("i", Some(3)),
        ("i", Some(3)),
        ("i", Some(3)),
        ("i", Some(3)),
        ("p", None),
        ("e", Some(4)),
        ("e", Some(4)),
    ]
    .iter()
    .map(|(t, b)| (t.to_string(), *b))
    .collect();
    assert_eq!(got, want);
}

#[test]
fn inner_declaration_shadows_outer_until_its_scope_ends() {
    let src = "\
class A {
  void m() {
    int v = 0;
    Runnable r = new Runnable() { int v = 1; public void run() { v++; } };
    v--;
  }
}
";
    let spt = first_spt(java(), src, VarMode::ContextFree);
    let v: Vec<Option<u32>> = bindings(&spt)
        .into_iter()
        .filter(|(t, _)| t == "v")
        .map(|(_, b)| b)
        .collect();
    // outer decl 0, anonymous-class field decl 2 (r is 1), its use, then the outer use
    assert_eq!(v, vec![Some(0), Some(2), Some(2), Some(0)]);
}

#[test]
fn err_token_is_a_keyword_and_never_folded() {
    let k = KRaw::Node {
        kind: "block",
        field: None,
        children: vec![
            KRaw::Token {
                kind: "ERROR",
                named: false,
                text: "#ERR".into(),
                bytes: 0..3,
                line: 1,
                field: None,
            },
            KRaw::Token {
                kind: "identifier",
                named: true,
                text: "Foo".into(),
                bytes: 4..7,
                line: 1,
                field: None,
            },
        ],
    };
    let globals = Globals::new(apex(), Vec::<String>::new());
    let Raw::List(items) = to_raw(k, apex(), &globals, VarMode::ContextFree) else {
        panic!("expected a list");
    };
    assert!(matches!(&items[0], Raw::Keyword(t) if t == "#ERR"));
    match &items[1] {
        Raw::Leaf(l) => {
            assert_eq!(&*l.text, "foo");
            assert!(l.is_var);
            assert_eq!(l.binding, None);
            assert_eq!(l.bytes, 4..7);
        }
        other => panic!("expected a leaf, got {other:?}"),
    }
}

#[test]
fn globals_builtins_repo_types_and_suffixes() {
    let g = Globals::new(apex(), vec!["AccountService".to_string()]);
    assert!(g.contains(apex(), "System"));
    assert!(g.contains(apex(), "accountservice"));
    assert!(g.contains(apex(), "ACCOUNTSERVICE"));
    assert!(g.contains(apex(), "Invoice__c"));
    assert!(g.contains(apex(), "Parent__r"));
    assert!(!g.contains(apex(), "accts"));
    let j = Globals::new(java(), vec!["Repo".to_string()]);
    assert!(j.contains(java(), "System"));
    assert!(j.contains(java(), "Repo"));
    assert!(!j.contains(java(), "system"));
    assert!(!j.contains(java(), "repo"));
    let back: Globals = serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
    assert_eq!(back, g);
    // an empty set still honours the spec's suffixes
    assert!(!Globals::default().contains(apex(), "system"));
    assert!(Globals::default().contains(apex(), "invoice__c"));
}

#[test]
fn deep_unit_builds_and_featurizes_without_overflow() {
    let src = format!(
        "class A {{ void m() {{ int x = {}y{}; }} }}",
        "(".repeat(5000),
        ")".repeat(5000)
    );
    let spt = first_spt(java(), &src, VarMode::ContextFree);
    let per_leaf = featurize(&spt, &FeatureConfig::DEFAULT, None);
    // `x` survives; `y` is inside the cut subtree
    assert_eq!(marks(&spt), owned(&[("x", true)]));
    assert!(!per_leaf[0].is_empty());
}
