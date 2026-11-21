//! T23: BM25 identifier channel — tokenizer, BM25 scoring, lexical search, RRF.

use aroma_core::features::FEATURE_SEED;
use aroma_core::lexical::{
    bm25_scores, identifier_terms, idf, is_identifier, lexical_search, mean_doc_len,
    query_term_ids, rrf, spt_terms, term_frequency, term_id, unit_terms, LexicalSource, MemLexical,
    TermRule, TERM_SEED,
};
use aroma_core::spt::{Leaf, Raw, Spt};

fn terms(text: &str) -> Vec<String> {
    identifier_terms(text)
}

fn leaf(text: &str) -> Raw {
    Raw::Leaf(Leaf {
        text: text.into(),
        is_var: false,
        binding: None,
        bytes: 0..0,
        line: 1,
    })
}

fn tree(leaves: &[&str]) -> Spt {
    let mut items = Vec::new();
    for l in leaves {
        items.push(leaf(l));
        items.push(Raw::Keyword(";".into()));
    }
    Spt::build(Raw::List(items)).unwrap()
}

mod tokenizer {
    use super::*;

    #[test]
    fn camel_case_keeps_whole_and_parts() {
        assert_eq!(terms("saveResult"), ["saveresult", "save", "result"]);
        assert_eq!(terms("isSuccess"), ["issuccess", "is", "success"]);
    }

    #[test]
    fn acronym_runs_split_before_the_last_capital() {
        assert_eq!(terms("HTTPRequest"), ["httprequest", "http", "request"]);
        assert_eq!(terms("getID"), ["getid", "get", "id"]);
        assert_eq!(terms("URL"), ["url"]);
    }

    #[test]
    fn underscore_dollar_and_digit_boundaries_split() {
        assert_eq!(
            terms("MAX_RETRY_COUNT"),
            ["max_retry_count", "max", "retry", "count"]
        );
        assert_eq!(terms("npsp__Batch__c"), ["npsp__batch__c", "npsp", "batch"]);
        assert_eq!(terms("log4j"), ["log4j", "log"]);
        assert_eq!(terms("Outer$Inner"), ["outer$inner", "outer", "inner"]);
        assert_eq!(terms("utf8Decoder"), ["utf8decoder", "utf", "decoder"]);
    }

    #[test]
    fn folded_apex_spelling_has_no_camel_split() {
        assert_eq!(terms("saveresult"), ["saveresult"]);
        assert_eq!(terms("account_id"), ["account_id", "account", "id"]);
    }

    #[test]
    fn short_numeric_and_stop_word_terms_are_dropped() {
        assert!(terms("x").is_empty());
        assert!(terms("i").is_empty());
        assert_eq!(terms("x1Name"), ["x1name", "name"]);
        assert!(terms("this").is_empty());
        assert!(terms("null").is_empty());
        assert!(terms("true").is_empty());
        assert!(terms("String").contains(&"string".to_string()));
        assert_eq!(terms("newList"), ["newlist", "list"]);
    }

    #[test]
    fn literals_are_never_terms() {
        for lit in [
            "\"secret token\"",
            "'Done'",
            "'a'",
            "42",
            "0x1F",
            "3.14",
            "1L",
            "\"\"\"text block\"\"\"",
            ".5",
            "LAST_N_DAYS:30",
        ] {
            assert!(!is_identifier(lit), "{lit}");
            assert!(terms(lit).is_empty(), "{lit}");
        }
        assert!(is_identifier("_tmp") && is_identifier("$x") && is_identifier("café"));
    }

    #[test]
    fn unit_terms_count_each_leaf_once_per_term() {
        let spt = tree(&[
            "saveResult",
            "results",
            "saveResult",
            "'literal value'",
            "42",
        ]);
        let tf = spt_terms(&spt, TermRule::Identifier);
        let got: Vec<(&str, u32)> = tf.iter().map(|(k, &v)| (k.as_str(), v)).collect();
        assert_eq!(
            got,
            [
                ("result", 2),
                ("results", 1),
                ("save", 2),
                ("saveresult", 2)
            ]
        );
        let ids = unit_terms(&spt, TermRule::Identifier);
        assert!(ids.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(ids.iter().map(|&(_, n)| n).sum::<u32>(), 7);
        assert!(ids.contains(&(term_id("saveresult"), 2)));
    }

    #[test]
    fn term_ids_use_the_shifted_feature_seed() {
        assert_eq!(TERM_SEED, FEATURE_SEED ^ 1);
        assert_eq!(
            term_id("database"),
            xxhash_rust::xxh3::xxh3_64_with_seed(b"database", FEATURE_SEED ^ 1)
        );
        assert_ne!(term_id("database"), term_id("update"));
    }
}

mod bm25 {
    use super::*;

    /// Three documents (ids 0, 1, 2) over terms a..d:
    ///   0: a a b     dl 3
    ///   1: b c       dl 2
    ///   2: c c c d   dl 4      N = 3, avgdl = 3
    fn three_docs() -> MemLexical {
        let (a, b, c, d) = (term_id("aa"), term_id("bb"), term_id("cc"), term_id("dd"));
        let mut m = MemLexical::default();
        let sorted = |mut v: Vec<(u64, u32)>| {
            v.sort_unstable();
            v
        };
        let d0 = sorted(vec![(a, 2), (b, 1)]);
        let d1 = sorted(vec![(b, 1), (c, 1)]);
        let d2 = sorted(vec![(c, 3), (d, 1)]);
        m.insert(0, &d0);
        m.insert(1, &d1);
        m.insert(2, &d2);
        m
    }

    #[test]
    fn idf_is_rsj_with_plus_one_smoothing() {
        // ln(1 + (3 - 1 + 0.5) / (1 + 0.5)) = ln(8/3)
        assert!((idf(3, 1) - (8.0f64 / 3.0).ln()).abs() < 1e-12);
        // ln(1 + 1.5 / 2.5) = ln(1.6)
        assert!((idf(3, 2) - 1.6f64.ln()).abs() < 1e-12);
        assert!(idf(3, 3) > 0.0, "smoothing keeps idf positive");
    }

    #[test]
    fn hand_computed_three_document_example() {
        let m = three_docs();
        assert_eq!(m.n_units(), 3);
        assert_eq!(m.avgdl(), 3.0);
        let q = vec![term_id("aa"), term_id("cc"), term_id("zz")];
        let (mut hits, known) = bm25_scores(&m, &q);
        assert_eq!(known, 2);
        hits.sort_by_key(|h| h.unit);
        // doc 0: a, tf 2, dl 3: norm = 1.2 * (0.25 + 0.75 * 3/3) = 1.2
        //        idf(a) * 2 * 2.2 / (2 + 1.2) = ln(8/3) * 1.375
        let s0 = (8.0f64 / 3.0).ln() * 1.375;
        // doc 1: c, tf 1, dl 2: norm = 1.2 * (0.25 + 0.5) = 0.9 -> 2.2 / 1.9
        let s1 = 1.6f64.ln() * (2.2 / 1.9);
        // doc 2: c, tf 3, dl 4: norm = 1.2 * (0.25 + 1.0) = 1.5 -> 6.6 / 4.5
        let s2 = 1.6f64.ln() * (6.6 / 4.5);
        let got: Vec<f64> = hits.iter().map(|h| h.score).collect();
        for (g, w) in got.iter().zip([s0, s1, s2]) {
            assert!((g - w).abs() < 1e-12, "{got:?} vs {:?}", [s0, s1, s2]);
        }
        let ranked = lexical_search(&m, &q, 10, &|_| true);
        assert_eq!(
            ranked.hits.iter().map(|h| h.unit).collect::<Vec<_>>(),
            [0, 2, 1]
        );
    }

    #[test]
    fn search_skips_ineligible_units_and_backfills() {
        let m = three_docs();
        let q = vec![term_id("aa"), term_id("cc")];
        let calls = std::cell::RefCell::new(Vec::new());
        let out = lexical_search(&m, &q, 2, &|u| {
            calls.borrow_mut().push(u);
            u != 0
        });
        assert_eq!(out.hits.iter().map(|h| h.unit).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(*calls.borrow(), [0, 2, 1]);
        // The cap stops eligibility checks.
        calls.borrow_mut().clear();
        let out = lexical_search(&m, &q, 1, &|u| {
            calls.borrow_mut().push(u);
            true
        });
        assert_eq!(out.hits.len(), 1);
        assert_eq!(*calls.borrow(), [0]);
    }

    #[test]
    fn ties_rank_by_unit_id() {
        let t = term_id("same");
        let mut m = MemLexical::default();
        for u in [0u32, 1, 2] {
            m.insert(u, &[(t, 1)]);
        }
        let out = lexical_search(&m, &[t], 3, &|_| true);
        assert_eq!(
            out.hits.iter().map(|h| h.unit).collect::<Vec<_>>(),
            [0, 1, 2]
        );
    }

    #[test]
    fn unknown_terms_and_empty_index_score_nothing() {
        let m = MemLexical::default();
        assert_eq!(m.avgdl(), 0.0);
        let out = lexical_search(&m, &[term_id("aa")], 5, &|_| true);
        assert!(out.hits.is_empty());
        assert_eq!(out.known_query_terms, 0);
        assert_eq!(mean_doc_len(&[]), 0.0);
        assert_eq!(mean_doc_len(&[1, 2]), 1.5);
    }

    #[test]
    fn query_term_ids_are_sorted_and_tf_lookup_works() {
        let spt = tree(&["saveResult", "saveResult"]);
        let ids = query_term_ids(&spt_terms(&spt, TermRule::Identifier));
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(ids.len(), 3);
        let m = three_docs();
        assert_eq!(term_frequency(&m, term_id("cc"), 2), Some(3));
        assert_eq!(term_frequency(&m, term_id("cc"), 0), None);
    }

    #[test]
    fn rrf_uses_k_sixty() {
        assert_eq!(rrf(1), 1.0 / 61.0);
        // Two good ranks beat one top rank paired with a poor one.
        assert!(rrf(2) + rrf(3) > rrf(1) + rrf(10));
    }
}

#[test]
fn markup_terms_keep_compounds_and_split_names_and_expressions() {
    for (text, expected) in [
        (
            "lightning-datatable",
            vec!["lightning-datatable", "lightning", "datatable"],
        ),
        (
            "Account.Owner.Name",
            vec!["account.owner.name", "account", "owner", "name"],
        ),
        (
            "{!$Record.Status__c}",
            vec!["record.status__c", "record", "status", "status__c"],
        ),
    ] {
        let terms = TermRule::Markup.terms(text);
        for term in expected {
            assert!(
                terms.contains(&term.to_string()),
                "{text}: missing {term}: {terms:?}"
            );
        }
    }
    assert!(TermRule::Markup.terms("{!123}").is_empty());
    assert_eq!(
        TermRule::Markup.terms("getName"),
        TermRule::Identifier.terms("getName")
    );
}
