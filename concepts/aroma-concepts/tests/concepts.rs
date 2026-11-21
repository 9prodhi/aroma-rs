use aroma_concepts::features::{featurize, FeatureConfig, Vocab};
use aroma_concepts::multiset::{jaccard, Bag};
use aroma_concepts::pipeline::{Corpus, Params};
use aroma_concepts::prune::{prune, Objective};
use aroma_concepts::spt::Spt;
use aroma_concepts::ts_adapter::{units, Java};

// Aroma paper running example: Luan et al., "Aroma: Code Recommendation via
// Structural Code Search" (OOPSLA 2019), arXiv:1812.01158; iterator-loop example.
const EXAMPLE: &str = r#"class  C{
    public void m() {

        StringBuffer strBuf = new StringBuffer();
        Iterator itr = list.iterator();
        while (itr.hasNext()) {
            itr.next();
        }
    }
}"#;

fn java_units(src: &str) -> Vec<Spt> {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_java::LANGUAGE.into()).expect("java grammar ABI");
    let tree = parser.parse(src, None).expect("parse");
    units(&tree, src.as_bytes(), &Java).into_iter().map(|(_, s)| s).collect()
}

fn bag(spt: &Spt, vocab: &mut Vocab) -> Bag {
    vocab.unit_bag(&featurize(spt, &FeatureConfig::REFERENCE, None))
}

#[test]
fn example_query_leaves_match_reference_stream() {
    let spt = &java_units(EXAMPLE)[0];
    let leaves: Vec<&str> = spt.leaves.iter().map(|l| &*l.text).collect();
    // Same non-keyword stream as the reference JSON (example_query.json).
    assert_eq!(
        leaves,
        ["StringBuffer", "strBuf", "StringBuffer", "Iterator", "itr", "list", "iterator", "itr", "hasNext", "itr", "next"]
    );
    let vars: Vec<&str> = spt.leaves.iter().filter(|l| l.is_var).map(|l| &*l.text).collect();
    assert_eq!(vars, ["strBuf", "itr", "list", "itr", "itr"]);
    assert!(spt.nodes.iter().any(|n| &*n.label == "while##"), "labels: {:?}", spt.nodes.iter().map(|n| &n.label).collect::<Vec<_>>());
}

#[test]
fn renaming_locals_does_not_change_features() {
    let renamed = EXAMPLE.replace("strBuf", "sb").replace("itr", "it");
    let mut vocab = Vocab::default();
    let a = bag(&java_units(EXAMPLE)[0], &mut vocab);
    let b = bag(&java_units(&renamed)[0], &mut vocab);
    assert_eq!(a, b);
}

#[test]
fn pruning_a_unit_against_itself_keeps_every_leaf() {
    let spt = &java_units(EXAMPLE)[0];
    let mut vocab = Vocab::default();
    let q = bag(spt, &mut vocab);
    for obj in [Objective::Jaccard, Objective::IntersectionSize] {
        let p = prune(&[&q], spt, None, &FeatureConfig::REFERENCE, &mut vocab, obj);
        assert!(p.kept.iter().all(|&k| k), "{obj:?}");
        assert_eq!(jaccard(&q, &p.bag), 1.0);
    }
}

#[test]
fn recommend_runs_end_to_end() {
    let corpus_src = r#"class K {
        void a() { Iterator itr = list.iterator(); while (itr.hasNext()) { Object o = itr.next(); log(o); } close(list); }
        void b() { Iterator it = items.iterator(); while (it.hasNext()) { Object x = it.next(); log(x); } close(items); }
        void c() { Iterator i = xs.iterator(); while (i.hasNext()) { Object y = i.next(); log(y); } close(xs); }
        void d() { int total = 0; for (int k = 0; k < 10; k++) { total += k; } print(total); }
    }"#;
    let cfg = FeatureConfig::REFERENCE;
    let mut corpus = Corpus::default();
    for spt in java_units(corpus_src) {
        corpus.add(spt, &cfg);
    }
    let query = &java_units("class Q { void q() { Iterator itr = list.iterator(); while (itr.hasNext()) { itr.next(); } } }")[0];
    let recs = corpus.recommend(query, &cfg, &Params::REFERENCE);
    assert!(!recs.is_empty(), "three near-identical iterator loops should form a cluster");
    for r in &recs {
        assert!(r.members.len() >= 2, "reference never emits singleton clusters");
        assert!(!r.members.contains(&3), "the unrelated for-loop method must not cluster");
    }
}
