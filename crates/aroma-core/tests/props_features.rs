//! Properties of `Spt::build` and `featurize` over generated `Raw` trees, plus
//! hardening cases for malformed trees and reference JSON.

use aroma_core::spt::{Elem, Leaf, Node, Raw, Spt, SymbolId};
use proptest::prelude::*;

const SPELLINGS: [&str; 6] = ["a", "b", "c", "Foo", "bar", "x"];
const KEYWORDS: [&str; 9] = [".", "(", ")", "{", "}", "=", ";", "if", "new"];

fn arb_leaf() -> impl Strategy<Value = Raw> {
    (
        prop::sample::select(SPELLINGS.to_vec()),
        any::<bool>(),
        prop::option::of(0u32..3),
        1u32..20,
    )
        .prop_map(|(text, is_var, binding, line)| {
            Raw::Leaf(Leaf {
                text: text.into(),
                is_var,
                binding: binding.map(SymbolId),
                bytes: line * 10..line * 10 + 1,
                line,
            })
        })
}

fn arb_raw() -> impl Strategy<Value = Raw> {
    let atom = prop_oneof![
        3 => arb_leaf(),
        2 => prop::sample::select(KEYWORDS.to_vec()).prop_map(|k| Raw::Keyword(k.into())),
    ];
    atom.prop_recursive(5, 64, 5, |inner| {
        prop::collection::vec(inner, 0..5).prop_map(Raw::List)
    })
}

fn arb_root() -> impl Strategy<Value = Raw> {
    prop::collection::vec(arb_raw(), 0..6).prop_map(Raw::List)
}

fn raw_tokens(r: &Raw, out: &mut Vec<String>) {
    match r {
        Raw::Keyword(k) => out.push(k.clone()),
        Raw::Leaf(l) => out.push(l.text.to_string()),
        Raw::List(v) => v.iter().for_each(|c| raw_tokens(c, out)),
    }
}

fn raw_leaves(r: &Raw, out: &mut Vec<Leaf>) {
    match r {
        Raw::Keyword(_) => {}
        Raw::Leaf(l) => out.push(l.clone()),
        Raw::List(v) => v.iter().for_each(|c| raw_leaves(c, out)),
    }
}

fn arb_spt() -> impl Strategy<Value = Spt> {
    arb_root().prop_filter_map("tree without tokens", Spt::build)
}

proptest! {
    #[test]
    fn build_preserves_token_order_and_leaves(raw in arb_root()) {
        let mut tokens = Vec::new();
        raw_tokens(&raw, &mut tokens);
        let mut leaves = Vec::new();
        raw_leaves(&raw, &mut leaves);
        match Spt::build(raw) {
            None => prop_assert!(tokens.is_empty()),
            Some(spt) => {
                prop_assert_eq!(spt.tokens(None), tokens.iter().map(String::as_str).collect::<Vec<_>>());
                prop_assert_eq!(&spt.leaves, &leaves);
            }
        }
    }

    #[test]
    fn build_leaves_no_unary_wrappers_and_consistent_labels(spt in arb_spt()) {
        for (id, node) in spt.nodes.iter().enumerate() {
            let expected: String = node
                .children
                .iter()
                .map(|c| match c {
                    Elem::Keyword(k) => k.to_string(),
                    Elem::Leaf(_) | Elem::Tree(_) => "#".to_string(),
                })
                .collect();
            prop_assert_eq!(&*node.label, expected.as_str());
            if id as u32 == spt.root {
                prop_assert!(!node.children.is_empty());
                prop_assert!(!(node.children.len() == 1 && matches!(node.children[0], Elem::Tree(_))));
            } else {
                prop_assert!(node.children.len() >= 2, "unary node {} {:?}", id, node);
            }
        }
        // every node except the root is referenced exactly once
        let mut refs = vec![0u32; spt.nodes.len()];
        for node in &spt.nodes {
            for c in &node.children {
                if let Elem::Tree(t) = c {
                    refs[*t as usize] += 1;
                }
            }
        }
        for (id, n) in refs.iter().enumerate() {
            prop_assert_eq!(*n, u32::from(id as u32 != spt.root));
        }
    }
}

#[test]
fn root_wrapping_a_single_subtree_is_unwrapped() {
    let leaf = |t: &str| {
        Raw::Leaf(Leaf {
            text: t.into(),
            is_var: false,
            binding: None,
            bytes: 0..1,
            line: 1,
        })
    };
    // List[List[List[a, "=", b]]] collapses to the `#=#` node itself.
    let raw = Raw::List(vec![Raw::List(vec![Raw::List(vec![
        leaf("a"),
        Raw::Keyword("=".into()),
        leaf("b"),
    ])])]);
    let spt = Spt::build(raw).expect("has tokens");
    assert_eq!(spt.nodes.len(), 1);
    assert_eq!(spt.label(spt.root), "#=#");
    assert_eq!(spt.tokens(None), ["a", "=", "b"]);
    // A leafless `{}` is kept as a sub-tree (reference exception to rule B).
    let raw = Raw::List(vec![
        Raw::Keyword("do".into()),
        Raw::List(vec![Raw::Keyword("{".into()), Raw::Keyword("}".into())]),
        leaf("x"),
    ]);
    let spt = Spt::build(raw).expect("has tokens");
    assert_eq!(spt.label(spt.root), "do##");
    assert_eq!(spt.nodes[1].label.as_ref(), "{}");
    assert!(Spt::build(Raw::List(vec![Raw::List(vec![])])).is_none());
}

#[test]
fn tokens_and_leftmost_leaf_tolerate_dangling_ids() {
    let spt = Spt {
        nodes: vec![Node {
            label: "###".into(),
            children: vec![Elem::Leaf(0), Elem::Tree(7), Elem::Leaf(9)],
        }],
        leaves: vec![Leaf {
            text: "a".into(),
            is_var: true,
            binding: None,
            bytes: 0..1,
            line: 1,
        }],
        root: 0,
    };
    assert_eq!(spt.tokens(None), ["a"]);
    assert_eq!(spt.tokens(Some(&[])), ["...", "..."]);
    assert_eq!(spt.leftmost_leaf(&Elem::Tree(0), None), Some(0));
    assert_eq!(spt.leftmost_leaf(&Elem::Tree(7), None), None);
    assert_eq!(spt.leftmost_leaf(&Elem::Leaf(9), None), None);
    assert_eq!(spt.leftmost_leaf(&Elem::Leaf(0), Some(&[])), None);
}

#[test]
fn reference_json_loader_rejects_malformed_input() {
    use serde_json::json;
    for bad in [
        json!(null),
        json!("{#}"),
        json!([]),
        json!(["{}"]),
        json!([1, {"token": "a", "leaf": true}]),
        json!(["#", {"leaf": true}]),
        json!(["#", 3]),
        json!(["##", null, {"token": "a", "leaf": true}]),
        json!({"token": "x"}),
    ] {
        assert!(Spt::from_reference_json(&bad).is_none(), "{bad}");
    }
}

#[test]
fn reference_json_loader_reads_leaf_var_and_line() {
    let ok = serde_json::json!(["#;", ["#.#", {"token": "a", "leaf": true, "var": false, "line": 3}, {"token": "."},
                          ["#()", {"token": "f", "leaf": true}, {"token": "("}, {"token": ")"}]],
                    {"token": ";"}]);
    let spt = Spt::from_reference_json(&ok).expect("well-formed");
    assert!(
        spt.leaves[0].is_var,
        "presence of \"var\" marks a variable, as in similar.py"
    );
    assert_eq!(spt.leaves[0].line, 3);
    assert!(!spt.leaves[1].is_var);
    assert_eq!(spt.leaves[1].line, 0);
    assert_eq!(spt.tokens(None), ["a", ".", "f", "(", ")", ";"]);
    let labels: Vec<&str> = spt.nodes.iter().map(|n| &*n.label).collect();
    assert_eq!(labels, ["#;", "#.#", "#()"]);
}

#[test]
fn deep_reference_json_is_rejected_without_overflow() {
    let mut v = serde_json::json!(["#", {"token": "a", "leaf": true}]);
    for _ in 0..1500 {
        v = serde_json::Value::Array(vec![serde_json::json!("#"), v]);
    }
    assert!(Spt::from_reference_json(&v).is_none());
}

#[test]
fn collapse_rules_a_b_c_match_the_reference_cases() {
    // T00's in-module rule A/B/C cases, kept here because Step group 1 replaces spt.rs.
    let kw = |s: &str| Raw::Keyword(s.to_string());
    let leaf = |t: &str, line: u32| {
        Raw::Leaf(Leaf {
            text: t.into(),
            is_var: true,
            binding: None,
            bytes: 0..0,
            line,
        })
    };
    let labels =
        |spt: &Spt| -> Vec<String> { spt.nodes.iter().map(|n| n.label.to_string()).collect() };
    // Rule A: [[[x]] ;] -> "#;" with the leaf directly under the root.
    let spt = Spt::build(Raw::List(vec![
        Raw::List(vec![Raw::List(vec![leaf("x", 1)])]),
        kw(";"),
    ]))
    .unwrap();
    assert_eq!(labels(&spt), ["#;"]);
    assert_eq!(
        spt.nodes[0].children,
        [Elem::Leaf(0), Elem::Keyword(";".into())]
    );
    // Rule A keeps a collapsed keyword's text in the parent label.
    let spt = Spt::build(Raw::List(vec![
        Raw::List(vec![kw("int")]),
        leaf("x", 1),
        kw(";"),
    ]))
    .unwrap();
    assert_eq!(labels(&spt), ["int#;"]);
    // Rule B: a leafless list is spliced into its parent.
    let spt = Spt::build(Raw::List(vec![
        leaf("StringBuffer", 1),
        Raw::List(vec![kw("("), kw(")")]),
    ]))
    .unwrap();
    assert_eq!(labels(&spt), ["#()"]);
    assert_eq!(spt.nodes[0].children.len(), 3);
    // Rule B exempts `{}`.
    let spt = Spt::build(Raw::List(vec![
        kw("while"),
        Raw::List(vec![kw("("), leaf("x", 1), kw(")")]),
        Raw::List(vec![kw("{"), kw("}")]),
    ]))
    .unwrap();
    assert_eq!(labels(&spt), ["while##", "(#)", "{}"]);
    // Rule C: lists with leaves become sub-trees, numbered in pre-order.
    let spt = Spt::build(Raw::List(vec![
        kw("{"),
        Raw::List(vec![
            Raw::List(vec![leaf("a", 2), kw("="), leaf("b", 2)]),
            kw(";"),
        ]),
        Raw::List(vec![
            Raw::List(vec![leaf("f", 3), kw("("), kw(")")]),
            kw(";"),
        ]),
        kw("}"),
    ]))
    .unwrap();
    assert_eq!(labels(&spt), ["{##}", "#;", "#=#", "#;", "#()"]);
    assert_eq!(
        spt.tokens(None),
        ["{", "a", "=", "b", ";", "f", "(", ")", ";", "}"]
    );
    // leftmost_leaf skips hidden leaves; hidden leaves print as `...`.
    let spt = Spt::build(Raw::List(vec![
        leaf("x", 1),
        kw("."),
        Raw::List(vec![leaf("foo", 1), kw("("), leaf("y", 1), kw(")")]),
    ]))
    .unwrap();
    assert_eq!(labels(&spt), ["#.#", "#(#)"]);
    let call = spt.nodes[0].children[2].clone();
    assert_eq!(spt.leftmost_leaf(&call, None), Some(1));
    let mask = [true, false, true];
    assert_eq!(spt.leftmost_leaf(&call, Some(&mask)), Some(2));
    assert_eq!(spt.tokens(Some(&mask)), ["x", ".", "...", "(", "y", ")"]);
}

#[test]
fn binding_and_spans_survive_build_and_postcard() {
    let l = Leaf {
        text: "i".into(),
        is_var: true,
        binding: Some(SymbolId(4)),
        bytes: 10..11,
        line: 3,
    };
    let spt = Spt::build(Raw::List(vec![
        Raw::Leaf(l.clone()),
        Raw::Keyword("++".into()),
    ]))
    .unwrap();
    assert_eq!(spt.leaves, [l]);
    let bytes = postcard::to_stdvec(&spt).unwrap();
    assert_eq!(postcard::from_bytes::<Spt>(&bytes).unwrap(), spt);
}

// ---- featurizer properties (Step group 2) ----

use aroma_core::features::{featurize, leaf_bags, unit_bag, FeatureConfig, UsageKey};
use aroma_core::lang::Lang;
use aroma_core::multiset::Bag;
use std::collections::{BTreeSet, HashMap};

/// Spellings that some `#.#` usage context reads verbatim (leftmost leaf two slots to
/// the right of a child of a `#.#` node). Renaming them would legitimately change
/// features, exactly as in the reference.
fn context_spellings(spt: &Spt) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for node in &spt.nodes {
        if &*node.label != "#.#" {
            continue;
        }
        for target in node.children.iter().skip(2) {
            if let Some(l) = spt.leftmost_leaf(target, None) {
                out.insert(spt.leaves[l as usize].text.to_string());
            }
        }
    }
    out
}

fn leaf(text: &str, is_var: bool, binding: Option<u32>) -> Leaf {
    Leaf {
        text: text.into(),
        is_var,
        binding: binding.map(SymbolId),
        bytes: 0..0,
        line: 1,
    }
}

proptest! {
    #[test]
    fn alpha_renaming_var_leaves_preserves_features(spt in arb_spt()) {
        let protected = context_spellings(&spt);
        let mut renamed = spt.clone();
        for l in renamed.leaves.iter_mut() {
            let lower = l.text.chars().next().is_some_and(char::is_lowercase);
            if l.is_var && lower && !protected.contains(&*l.text) {
                l.text = format!("r{}", l.text).into();
            }
        }
        for cfg in [FeatureConfig::DEFAULT, FeatureConfig::REFERENCE] {
            prop_assert_eq!(featurize(&spt, &cfg, None), featurize(&renamed, &cfg, None));
        }
    }

    #[test]
    fn distinct_symbols_never_share_usage_features(spt in arb_spt()) {
        let mut shadowed = spt.clone();
        for (i, l) in shadowed.leaves.iter_mut().enumerate() {
            l.binding = Some(SymbolId(1000 + i as u32));
        }
        let features = featurize(&shadowed, &FeatureConfig::DEFAULT, None);
        prop_assert!(features.iter().flatten().all(|f| !f.contains(">>>")));
    }

    #[test]
    fn spelling_derived_symbols_equal_spelling_keys(spt in arb_spt()) {
        let mut ids: HashMap<String, u32> = HashMap::new();
        let mut keyed = spt.clone();
        for l in keyed.leaves.iter_mut() {
            let next = ids.len() as u32;
            l.binding = Some(SymbolId(*ids.entry(l.text.to_string()).or_insert(next)));
        }
        let spelling = FeatureConfig { usage_key: UsageKey::Spelling, ..FeatureConfig::DEFAULT };
        prop_assert_eq!(
            featurize(&keyed, &FeatureConfig::DEFAULT, None),
            featurize(&keyed, &spelling, None)
        );
    }

    #[test]
    fn masks_hide_leaves_and_never_panic(spt in arb_spt(), mask in prop::collection::vec(any::<bool>(), 0..40)) {
        let per_leaf = featurize(&spt, &FeatureConfig::DEFAULT, Some(&mask));
        prop_assert_eq!(per_leaf.len(), spt.leaves.len());
        for (i, fs) in per_leaf.iter().enumerate() {
            let visible = mask.get(i).copied().unwrap_or(false);
            prop_assert_eq!(fs.is_empty(), !visible);
        }
        let all_true = vec![true; spt.leaves.len()];
        prop_assert_eq!(featurize(&spt, &FeatureConfig::DEFAULT, Some(&all_true)), featurize(&spt, &FeatureConfig::DEFAULT, None));
    }

    #[test]
    fn first_feature_is_the_token_and_pairs_come_in_twos(spt in arb_spt()) {
        for cfg in [FeatureConfig::DEFAULT, FeatureConfig::REFERENCE] {
            let per_leaf = featurize(&spt, &cfg, None);
            for (l, fs) in spt.leaves.iter().zip(&per_leaf) {
                let upper = l.text.chars().next().is_some_and(char::is_uppercase);
                let var = l.is_var && !(cfg.uppercase_is_global && upper);
                prop_assert_eq!(fs[0].as_str(), if var { "#VAR" } else { &*l.text });
            }
            let pairs = per_leaf.iter().flatten().filter(|f| f.contains(">>")).count();
            prop_assert_eq!(pairs % 2, 0);
            let bag = unit_bag(Lang::Java, &per_leaf);
            let total = leaf_bags(Lang::Java, &per_leaf).iter().fold(Bag::default(), |a, b| a.sum(b));
            prop_assert_eq!(bag.len(), per_leaf.iter().map(|f| f.len() as u64).sum::<u64>());
            prop_assert_eq!(bag, total);
        }
    }

    #[test]
    fn parent_features_never_exceed_n_parents(spt in arb_spt(), n_parents in 0usize..5) {
        let cfg = FeatureConfig { n_parents, ..FeatureConfig::DEFAULT };
        for fs in featurize(&spt, &cfg, None) {
            let parents = fs.iter().filter(|f| f.contains('>') && !f.contains(">>")).count();
            prop_assert!(parents <= n_parents);
        }
    }
}

#[test]
fn shadowed_locals_link_by_symbol_not_spelling() {
    // `x = x; x = x` under one `#;#` node: two declarations of `x` with different symbols.
    let spt = Spt {
        nodes: vec![
            Node {
                label: "#;#".into(),
                children: vec![Elem::Tree(1), Elem::Keyword(";".into()), Elem::Tree(2)],
            },
            Node {
                label: "#=#".into(),
                children: vec![Elem::Leaf(0), Elem::Keyword("=".into()), Elem::Leaf(1)],
            },
            Node {
                label: "#=#".into(),
                children: vec![Elem::Leaf(2), Elem::Keyword("=".into()), Elem::Leaf(3)],
            },
        ],
        leaves: vec![
            leaf("x", true, Some(0)),
            leaf("x", true, Some(0)),
            leaf("x", true, Some(1)),
            leaf("x", true, Some(1)),
        ],
        root: 0,
    };
    let usage = |cfg: &FeatureConfig| -> Vec<Vec<String>> {
        featurize(&spt, cfg, None)
            .into_iter()
            .map(|fs| fs.into_iter().filter(|f| f.contains(">>>")).collect())
            .collect()
    };
    assert_eq!(
        usage(&FeatureConfig::DEFAULT),
        [
            vec!["#=#1>>>#=#3"],
            vec!["#=#1>>>#=#3"],
            vec!["#=#1>>>#=#3"],
            vec!["#=#1>>>#=#3"]
        ]
    );
    // Spelling keys (reference) chain all four uses: each links its two predecessors.
    assert_eq!(
        usage(&FeatureConfig::REFERENCE),
        [
            vec!["#=#1>>>#=#3", "#=#1>>>#=#1"],
            vec!["#=#1>>>#=#3", "#=#3>>>#=#1", "#=#3>>>#=#3"],
            vec!["#=#3>>>#=#1", "#=#1>>>#=#1", "#=#1>>>#=#3"],
            vec!["#=#1>>>#=#3", "#=#3>>>#=#3"],
        ]
    );
}

#[test]
fn dot_context_out_of_range_never_panics() {
    // `#.#` whose var child has no slot two to the right (truncated / malformed tree).
    let spt = Spt {
        nodes: vec![
            Node {
                label: "##".into(),
                children: vec![Elem::Tree(1), Elem::Tree(2)],
            },
            Node {
                label: "#.#".into(),
                children: vec![Elem::Leaf(0), Elem::Keyword(".".into())],
            },
            Node {
                label: "#.#".into(),
                children: vec![Elem::Leaf(1), Elem::Keyword(".".into())],
            },
        ],
        leaves: vec![leaf("a", true, None), leaf("a", true, None)],
        root: 0,
    };
    let per_leaf = featurize(&spt, &FeatureConfig::REFERENCE, None);
    assert_eq!(per_leaf[0], ["#VAR", "#.#1>#VAR", "##1>#VAR", "#VAR>>#VAR"]);
    assert_eq!(per_leaf[1], ["#VAR", "#.#1>#VAR", "##2>#VAR", "#VAR>>#VAR"]);
}

#[test]
fn dot_context_on_keyword_or_last_slot_yields_no_usage_feature() {
    let spt = Spt {
        nodes: vec![
            Node {
                label: "##".into(),
                children: vec![Elem::Tree(1), Elem::Tree(2)],
            },
            // target slot holds a keyword
            Node {
                label: "#.#".into(),
                children: vec![
                    Elem::Leaf(0),
                    Elem::Keyword(".".into()),
                    Elem::Keyword("x".into()),
                ],
            },
            // var leaf is the right-most child of a `#.#`
            Node {
                label: "#.#".into(),
                children: vec![Elem::Leaf(1), Elem::Keyword(".".into()), Elem::Leaf(2)],
            },
        ],
        leaves: vec![
            leaf("a", true, None),
            leaf("b", false, None),
            leaf("a", true, None),
        ],
        root: 0,
    };
    let per_leaf = featurize(&spt, &FeatureConfig::REFERENCE, None);
    assert!(
        per_leaf.iter().flatten().all(|f| !f.contains(">>>")),
        "{per_leaf:?}"
    );
}

#[test]
fn dangling_ids_and_short_masks_never_panic_in_featurize() {
    let spt = Spt {
        nodes: vec![Node {
            label: "###".into(),
            children: vec![Elem::Leaf(0), Elem::Tree(7), Elem::Leaf(9)],
        }],
        leaves: vec![leaf("a", true, None)],
        root: 0,
    };
    assert_eq!(
        featurize(&spt, &FeatureConfig::DEFAULT, None),
        [vec!["#VAR", "###1>#VAR"]]
    );
    assert_eq!(
        featurize(&spt, &FeatureConfig::DEFAULT, Some(&[])),
        [Vec::<String>::new()]
    );
    let bad_root = Spt { root: 5, ..spt };
    assert_eq!(
        featurize(&bad_root, &FeatureConfig::DEFAULT, None),
        [Vec::<String>::new()]
    );
}

#[test]
fn reference_json_dot_context_reads_the_method_name() {
    let ok = serde_json::json!(["#;", ["#.#", {"token": "a", "leaf": true, "var": true}, {"token": "."},
                          ["#()", {"token": "f", "leaf": true}, {"token": "("}, {"token": ")"}]],
                    {"token": ";"}, ["#.#", {"token": "a", "leaf": true, "var": true}, {"token": "."},
                          ["#()", {"token": "g", "leaf": true}, {"token": "("}, {"token": ")"}]]]);
    let spt = Spt::from_reference_json(&ok).expect("well-formed");
    let per_leaf = featurize(&spt, &FeatureConfig::REFERENCE, None);
    assert_eq!(
        per_leaf[0],
        ["#VAR", "#.#1>#VAR", "#;1>#VAR", "#VAR>>f", "f>>>g"]
    );
    assert_eq!(
        per_leaf[2],
        [
            "#VAR",
            "#.#1>#VAR",
            "#;3>#VAR",
            "f>>>g",
            "f>>#VAR",
            "#VAR>>g"
        ]
    );
}
