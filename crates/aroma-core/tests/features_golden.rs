//! Feature goldens on the reference example (`example_query.json`, note 04 §4b) and
//! stability snapshots of feature ids and config fingerprints.
//!
//! The reference artifacts are CC BY-NC-SA and never copied into this repo: tests read
//! `AROMA_REFERENCE_DIR` (default `<workspace>/ref/aroma-paper-artifacts`, filled by
//! `cargo run -p xtask -- fetch-reference`) and return early when it is absent.
//!
//! Why the legacy 55-line `example_features.txt` is not asserted in full: note 04 §4
//! shows it was produced by another revision of `similar.py` with N_SIBLINGS = 2,
//! sibling pairs attributed ONCE (not to both endpoints), no `>>>` usage features, and
//! a broader wrapper-skip rule (`^[#(){}]*$`, which also skips `##` and `#()`). Pair
//! attribution and the skip regex are not knobs of the frozen `FeatureConfig`, so no
//! configuration of this featurizer can produce that file. Only its token features
//! (the lines without `>`), which every revision shares, are compared.

use aroma_core::features::{
    feature_id, featurize, leaf_bags, unit_bag, FeatureConfig, UsageKey, FEATURE_SCHEMA,
    FEATURE_SEED,
};
use aroma_core::lang::Lang;
use aroma_core::multiset::Bag;
use aroma_core::spt::Spt;
use std::path::PathBuf;

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

fn example_spt() -> Option<Spt> {
    let text = reference_file("example_query.json")?;
    let record: serde_json::Value =
        serde_json::from_str(&text).expect("example_query.json is JSON");
    Some(Spt::from_reference_json(&record["ast"]).expect("example ast loads"))
}

fn sorted(v: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut v: Vec<String> = v.into_iter().collect();
    v.sort();
    v
}

/// Note 04 §4b, leaf-major as `similar.py` builds `feature_list` (both copies of a pair
/// feature are listed at the later endpoint there). One line per leaf.
#[rustfmt::skip]
const SHIPPED_DEFAULT_67: [&str; 67] = [
    "StringBuffer", "##1>StringBuffer", "#;1>StringBuffer",
    "#VAR", "#=#1>#VAR", "##2>#VAR", "#;1>#VAR", "StringBuffer>>#VAR", "StringBuffer>>#VAR",
    "StringBuffer", "#()1>StringBuffer", "new#2>StringBuffer", "#=#3>StringBuffer", "#VAR>>StringBuffer", "#VAR>>StringBuffer",
    "Iterator", "##1>Iterator", "#;1>Iterator", "StringBuffer>>Iterator", "StringBuffer>>Iterator",
    "#VAR", "#=#1>#VAR", "##2>#VAR", "#;1>#VAR", "Iterator>>#VAR", "Iterator>>#VAR",
    "#VAR", "#.#1>#VAR", "#=#3>#VAR", "##2>#VAR", "#VAR>>#VAR", "#VAR>>#VAR",
    "iterator", "#()1>iterator", "#.#3>iterator", "#=#3>iterator", "#VAR>>iterator", "#VAR>>iterator",
    "#VAR", "#.#1>#VAR", "while##2>#VAR", "#=#1>>>hasNext", "#=#1>>>hasNext", "iterator>>#VAR", "iterator>>#VAR",
    "hasNext", "#()1>hasNext", "#.#3>hasNext", "while##2>hasNext", "#VAR>>hasNext", "#VAR>>hasNext",
    "#VAR", "#.#1>#VAR", "#;1>#VAR", "while##3>#VAR", "hasNext>>>next", "hasNext>>>next", "#=#1>>>next", "#=#1>>>next", "hasNext>>#VAR", "hasNext>>#VAR",
    "next", "#()1>next", "#.#3>next", "#;1>next", "#VAR>>next", "#VAR>>next",
];

#[test]
fn reference_leaf_stream_and_var_flags() {
    let Some(spt) = example_spt() else { return };
    let texts: Vec<&str> = spt.leaves.iter().map(|l| &*l.text).collect();
    assert_eq!(
        texts,
        [
            "StringBuffer",
            "strBuf",
            "StringBuffer",
            "Iterator",
            "itr",
            "list",
            "iterator",
            "itr",
            "hasNext",
            "itr",
            "next"
        ]
    );
    let vars: Vec<bool> = spt.leaves.iter().map(|l| l.is_var).collect();
    assert_eq!(
        vars,
        [false, true, false, false, true, true, false, true, false, true, false]
    );
    let lines: Vec<u32> = spt.leaves.iter().map(|l| l.line).collect();
    assert_eq!(lines, [4, 4, 4, 5, 5, 5, 5, 6, 6, 7, 7]);
    assert!(spt
        .leaves
        .iter()
        .all(|l| l.binding.is_none() && l.bytes == (0..0)));
}

#[test]
fn reference_labels_are_kept_verbatim_in_preorder() {
    let Some(spt) = example_spt() else { return };
    let labels: Vec<&str> = spt.nodes.iter().map(|n| &*n.label).collect();
    assert_eq!(
        labels,
        [
            "{###}", "#;", "##", "#=#", "new#", "#()", "#;", "##", "#=#", "#.#", "#()", "while##",
            "(#)", "#.#", "#()", "{#}", "#;", "#.#", "#()"
        ]
    );
    assert_eq!(spt.label(spt.root), "{###}");
}

#[test]
fn shipped_default_config_yields_the_67_feature_multiset() {
    let Some(spt) = example_spt() else { return };
    let per_leaf = featurize(&spt, &FeatureConfig::REFERENCE, None);
    let got = sorted(per_leaf.into_iter().flatten());
    let want = sorted(SHIPPED_DEFAULT_67.iter().map(|s| s.to_string()));
    assert_eq!(got.len(), 67);
    assert_eq!(got, want);
}

#[test]
fn per_leaf_attribution_puts_pairs_on_both_endpoints() {
    let Some(spt) = example_spt() else { return };
    let per_leaf = featurize(&spt, &FeatureConfig::REFERENCE, None);
    let want: [&[&str]; 11] = [
        &[
            "StringBuffer",
            "##1>StringBuffer",
            "#;1>StringBuffer",
            "StringBuffer>>#VAR",
        ],
        &[
            "#VAR",
            "#=#1>#VAR",
            "##2>#VAR",
            "#;1>#VAR",
            "StringBuffer>>#VAR",
            "#VAR>>StringBuffer",
        ],
        &[
            "StringBuffer",
            "#()1>StringBuffer",
            "new#2>StringBuffer",
            "#=#3>StringBuffer",
            "#VAR>>StringBuffer",
            "StringBuffer>>Iterator",
        ],
        &[
            "Iterator",
            "##1>Iterator",
            "#;1>Iterator",
            "StringBuffer>>Iterator",
            "Iterator>>#VAR",
        ],
        &[
            "#VAR",
            "#=#1>#VAR",
            "##2>#VAR",
            "#;1>#VAR",
            "Iterator>>#VAR",
            "#VAR>>#VAR",
            "#=#1>>>hasNext",
            "#=#1>>>next",
        ],
        &[
            "#VAR",
            "#.#1>#VAR",
            "#=#3>#VAR",
            "##2>#VAR",
            "#VAR>>#VAR",
            "#VAR>>iterator",
        ],
        &[
            "iterator",
            "#()1>iterator",
            "#.#3>iterator",
            "#=#3>iterator",
            "#VAR>>iterator",
            "iterator>>#VAR",
        ],
        &[
            "#VAR",
            "#.#1>#VAR",
            "while##2>#VAR",
            "#=#1>>>hasNext",
            "iterator>>#VAR",
            "#VAR>>hasNext",
            "hasNext>>>next",
        ],
        &[
            "hasNext",
            "#()1>hasNext",
            "#.#3>hasNext",
            "while##2>hasNext",
            "#VAR>>hasNext",
            "hasNext>>#VAR",
        ],
        &[
            "#VAR",
            "#.#1>#VAR",
            "#;1>#VAR",
            "while##3>#VAR",
            "hasNext>>>next",
            "#=#1>>>next",
            "hasNext>>#VAR",
            "#VAR>>next",
        ],
        &["next", "#()1>next", "#.#3>next", "#;1>next", "#VAR>>next"],
    ];
    assert_eq!(per_leaf.len(), want.len());
    for (i, (got, want)) in per_leaf.iter().zip(want.iter()).enumerate() {
        assert_eq!(got, want, "leaf {i}");
    }
}

#[test]
fn default_config_equals_reference_on_the_example() {
    // No upper-case variable and no bindings: D18 keying falls back to spelling.
    let Some(spt) = example_spt() else { return };
    assert_eq!(
        featurize(&spt, &FeatureConfig::DEFAULT, None),
        featurize(&spt, &FeatureConfig::REFERENCE, None)
    );
}

#[test]
fn masked_example_drops_hidden_leaves_and_relinks_chains() {
    // Keep only the three `itr` uses (leaves 4, 7, 9): siblings and usages re-link.
    let Some(spt) = example_spt() else { return };
    let mut mask = vec![false; spt.leaves.len()];
    for i in [4, 7, 9] {
        mask[i] = true;
    }
    let per_leaf = featurize(&spt, &FeatureConfig::REFERENCE, Some(&mask));
    for (i, fs) in per_leaf.iter().enumerate() {
        if !mask[i] {
            assert!(fs.is_empty(), "hidden leaf {i} emitted {fs:?}");
        }
    }
    // `hasNext` and `next` are hidden, so the `#.#` contexts of the 2nd and 3rd `itr`
    // have no visible leaf: no usage context, no `>>>` (reference `get_leftmost_leaf`
    // skips the " ... " dummies). Sibling links re-form between the visible leaves.
    assert_eq!(
        per_leaf[4],
        ["#VAR", "#=#1>#VAR", "##2>#VAR", "#;1>#VAR", "#VAR>>#VAR"]
    );
    assert_eq!(
        per_leaf[7],
        [
            "#VAR",
            "#.#1>#VAR",
            "while##2>#VAR",
            "#VAR>>#VAR",
            "#VAR>>#VAR"
        ]
    );
    assert_eq!(
        per_leaf[9],
        [
            "#VAR",
            "#.#1>#VAR",
            "#;1>#VAR",
            "while##3>#VAR",
            "#VAR>>#VAR"
        ]
    );
    assert_eq!(
        spt.tokens(Some(&mask)),
        [
            "{", "...", "...", "=", "new", "...", "(", ")", ";", "...", "itr", "=", "...", ".",
            "...", "(", ")", ";", "while", "(", "itr", ".", "...", "(", ")", ")", "{", "itr", ".",
            "...", "(", ")", ";", "}", "}"
        ]
    );
}

#[test]
fn legacy_example_features_share_the_token_features() {
    let Some(spt) = example_spt() else { return };
    let Some(legacy) = reference_file("example_features.txt") else {
        return;
    };
    let legacy_lines: Vec<&str> = legacy.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(legacy_lines.len(), 55, "pinned legacy file has 55 features");
    let legacy_tokens = sorted(
        legacy_lines
            .iter()
            .filter(|l| !l.contains('>'))
            .map(|s| s.to_string()),
    );
    let ours = sorted(
        featurize(&spt, &FeatureConfig::REFERENCE, None)
            .into_iter()
            .flatten()
            .filter(|f| !f.contains('>')),
    );
    assert_eq!(ours, legacy_tokens);
}

#[test]
fn unit_bag_is_the_sum_of_leaf_bags() {
    let Some(spt) = example_spt() else { return };
    let per_leaf = featurize(&spt, &FeatureConfig::REFERENCE, None);
    let total = leaf_bags(Lang::Java, &per_leaf)
        .iter()
        .fold(Bag::default(), |acc, b| acc.sum(b));
    let bag = unit_bag(Lang::Java, &per_leaf);
    assert_eq!(bag, total);
    assert_eq!(bag.len(), 67);
    assert_eq!(bag.distinct().count(), 42);
    assert_eq!(bag.get(feature_id(Lang::Java, "#VAR")), 5);
    assert_eq!(
        bag.get(feature_id(Lang::Apex, "#VAR")),
        0,
        "languages never share ids"
    );
}

#[test]
fn feature_ids_are_stable_snapshots() {
    assert_eq!(FEATURE_SEED, 0x4152_4f4d_415f_7631);
    assert_eq!(FEATURE_SCHEMA, 1);
    assert_eq!(feature_id(Lang::Java, "#VAR"), 0x0ff8_5531_2e6c_bd0f);
    assert_eq!(feature_id(Lang::Apex, "#VAR"), 0xdb6a_0de1_bb9c_b7c1);
    assert_eq!(
        feature_id(Lang::Java, "StringBuffer"),
        0xe7b9_38db_5032_44c7
    );
    assert_eq!(
        feature_id(Lang::Java, "#VAR"),
        xxhash_rust::xxh3::xxh3_64_with_seed(b"java\x1f#VAR", FEATURE_SEED)
    );
}

#[test]
fn config_fingerprints_are_stable_and_distinct() {
    assert_eq!(
        FeatureConfig::REFERENCE.fingerprint(),
        xxhash_rust::xxh3::xxh3_64(b"v1|3|1|2|true|true|Spelling")
    );
    assert_eq!(
        FeatureConfig::DEFAULT.fingerprint(),
        xxhash_rust::xxh3::xxh3_64(b"v1|3|1|2|true|false|Symbol")
    );
    assert_eq!(
        FeatureConfig::REFERENCE.fingerprint(),
        0xb959_b1c2_a8a1_4b12
    );
    assert_eq!(FeatureConfig::DEFAULT.fingerprint(), 0xfbd1_7e83_627d_6d92);
    let spelling_default = FeatureConfig {
        usage_key: UsageKey::Spelling,
        ..FeatureConfig::DEFAULT
    };
    assert_ne!(
        spelling_default.fingerprint(),
        FeatureConfig::DEFAULT.fingerprint()
    );
}
