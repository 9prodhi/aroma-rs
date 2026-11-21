//! D34 evidence survives XML edits; D18 uses the same config on both paths.
use aroma_core::features::{featurize, unit_bag, FeatureConfig, LabelMode, ParentSlot, VarUsage};
use aroma_core::lang::Lang;
use aroma_core::multiset::Bag;
use aroma_core::spt::Spt;
use aroma_lang::classify::{unit_spt, VarMode};
use aroma_lang::globals::Globals;
use aroma_lang::spec::{sfmeta, spec_for, LangSpec};
use aroma_lang::table::{extract_units, parse, Limits};
use proptest::prelude::*;

#[derive(Clone, Debug)]
struct Element {
    tag: &'static str,
    text: String,
    children: Vec<Element>,
}

impl Element {
    fn xml(&self) -> String {
        format!(
            "<{}>{}{}</{}>",
            self.tag,
            self.text,
            self.children.iter().map(Self::xml).collect::<String>(),
            self.tag
        )
    }

    fn shuffle(&mut self, seed: &mut u64) {
        for child in &mut self.children {
            child.shuffle(seed);
        }
        for i in (1..self.children.len()).rev() {
            *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let j = (*seed as usize) % (i + 1);
            self.children.swap(i, j);
        }
    }
}

fn element() -> impl Strategy<Value = Element> {
    let leaf = (
        prop::sample::select(vec![
            "description",
            "label",
            "name",
            "fullName",
            "apiName",
            "type",
            "fieldText",
            "leftValueReference",
            "formula",
            "expression",
        ]),
        "[a-zA-Z][a-zA-Z0-9_]{0,12}",
        0u8..5,
    )
        .prop_map(|(tag, value, kind)| Element {
            tag,
            text: match (tag, kind) {
                (_, 0) => String::new(),
                ("fieldText", _) => "{!local} {!local}".into(),
                ("leftValueReference", _) => "local".into(),
                ("formula" | "expression", _) => format!("IF(local &gt; 0, {value}__c, local)"),
                _ => value,
            },
            children: Vec::new(),
        });
    leaf.prop_recursive(3, 40, 5, |inner| {
        (
            prop::sample::select(vec!["fields", "rules", "conditions", "values"]),
            prop::collection::vec(inner, 0..5),
        )
            .prop_map(|(tag, children)| Element {
                tag,
                text: String::new(),
                children,
            })
    })
}

fn screen(children: &[Element]) -> String {
    // Keep binding environments stable across edits; changing a declaration can
    // legitimately reclassify references (the edit properties concern child order).
    format!(
        "<Flow><screens><variables><name>local</name></variables>{}</screens></Flow>",
        children.iter().map(Element::xml).collect::<String>()
    )
}

fn tree_with_spec(src: &str, spec: &LangSpec) -> Spt {
    let tree = parse(spec, src, &Limits::DEFAULT).unwrap();
    assert!(!tree.root_node().has_error(), "{src}");
    let units = extract_units(&tree, src, spec, &Limits::DEFAULT);
    unit_spt(&units[0], spec, &Globals::default(), VarMode::ContextFree).unwrap()
}

fn tree(src: &str) -> Spt {
    tree_with_spec(src, &sfmeta::SPEC)
}

fn config() -> FeatureConfig {
    (sfmeta::SPEC.feature_config)(FeatureConfig::DEFAULT)
}

fn bag(src: &str) -> Bag {
    unit_bag(Lang::SfMeta, &featurize(&tree(src), &config(), None))
}

fn included(a: &Bag, b: &Bag) -> bool {
    a.intersect(b) == *a
}

proptest! {
    #[test]
    fn shuffled_metadata_children_have_identical_feature_bags(
        children in prop::collection::vec(element(), 1..9), seed in any::<u64>()
    ) {
        let mut root = Element { tag: "screens", text: String::new(), children: children.clone() };
        let mut seed = seed;
        root.shuffle(&mut seed);
        prop_assert_eq!(bag(&screen(&children)), bag(&screen(&root.children)));
    }

    #[test]
    fn inserting_metadata_child_preserves_original_feature_bag(
        children in prop::collection::vec(element(), 0..8), extra in element(), slot in any::<usize>()
    ) {
        let before = bag(&screen(&children));
        let mut edited = children;
        edited.insert(slot % (edited.len() + 1), extra);
        prop_assert!(included(&before, &bag(&screen(&edited))));
    }

    #[test]
    fn deleting_metadata_child_removes_only_incident_features(
        mut children in prop::collection::vec(element(), 1..9), slot in any::<usize>()
    ) {
        let index = slot % children.len();
        let original = screen(&children);
        let start = "<Flow><screens><variables><name>local</name></variables>".len()
            + children[..index].iter().map(|e| e.xml().len()).sum::<usize>();
        let end = start + children[index].xml().len();
        let spt = tree(&original);
        let mask: Vec<_> = spt.leaves.iter().map(|leaf|
            !(start <= leaf.bytes.start as usize && (leaf.bytes.end as usize) <= end)
        ).collect();
        let full = featurize(&spt, &config(), None);
        let surviving = featurize(&spt, &config(), Some(&mask));
        // The mask removes only endpoints inside the deleted XML span. Every
        // retained token/parent feature stays; only incident usage links vanish.
        for ((old, new), keep) in full.iter().zip(&surviving).zip(&mask) {
            if *keep {
                let unary = |fs: &Vec<String>| fs.iter().filter(|f| !f.contains(">>>")).cloned().collect::<Vec<_>>();
                prop_assert_eq!(unary(old), unary(new));
            }
        }
        children.remove(index);
        let after = bag(&screen(&children));
        prop_assert_eq!(&after, &unit_bag(Lang::SfMeta, &surviving));
        prop_assert!(included(&after, &unit_bag(Lang::SfMeta, &full)));
    }
}

#[test]
fn structural_knobs_and_lexical_token_stream_are_separate() {
    let cfg = config();
    assert_eq!(cfg.parent_slot, ParentSlot::ChildLabel);
    assert_eq!(cfg.label_mode, LabelMode::Fixed);
    assert_eq!(cfg.var_usage, VarUsage::Positionless);
    assert_eq!(cfg.n_siblings, 0);
    let source = "<CustomField><type>Text</type><label>Customer</label></CustomField>";
    assert_eq!(tree(source).leaves[0].text.as_ref(), "Customer");
    let default_spec = spec_for(Lang::SfMeta).unwrap();
    assert_eq!((default_spec.feature_config)(FeatureConfig::DEFAULT), cfg);
    let lexical = tree_with_spec(source, &sfmeta::LEXICAL_SPEC);
    assert_eq!(lexical.leaves[0].text.as_ref(), "Text");
    assert_eq!(lexical.leaves[1].text.as_ref(), "Customer");
    assert_eq!(sfmeta::LEXICAL_SPEC.adapter_version, "sfmeta-1");
}

#[test]
fn flow_fields_and_validation_rules_keep_evidence_after_optional_children() {
    for (root, body, extra) in [
        ("CustomField", "<fullName>Total__c</fullName><formula>Amount__c * 2</formula><type>Number</type>", "<description>Double the amount</description>"),
        ("ValidationRule", "<fullName>RequireName</fullName><errorConditionFormula>AND(ISNEW(), ISBLANK(Name))</errorConditionFormula><errorMessage>Enter name</errorMessage>", "<active>true</active>"),
        ("Flow", "<screens><name>Details</name><fields><name>Customer</name><fieldText>Customer name</fieldText></fields></screens>", "<description>Collect details</description>"),
    ] {
        let before = format!("<{root}>{body}</{root}>");
        let after = if root == "Flow" {
            before.replace("<screens>", &format!("<screens>{extra}"))
        } else {
            format!("<{root}>{extra}{body}</{root}>")
        };
        assert!(included(&bag(&before), &bag(&after)), "{before}");
    }
}

#[test]
fn repeated_unnamed_children_and_all_identity_tags_sort_deterministically() {
    let children = "<fields><label>Zed</label></fields><fields><label>Alpha</label></fields><fields><apiName>Beta</apiName></fields><fields><name>Gamma</name></fields><fields><fullName>Delta</fullName></fields>";
    let source = format!("<Flow><screens>{children}</screens></Flow>");
    let spt = tree(&source);
    assert_eq!(
        spt.leaves
            .iter()
            .map(|l| l.text.as_ref())
            .collect::<Vec<_>>(),
        ["Alpha", "Zed", "Beta", "Delta", "Gamma"]
    );
    let both = "<Flow><screens><fields><name>A</name><fullName>Z</fullName></fields><fields><name>Z</name><fullName>A</fullName></fields></screens></Flow>";
    assert_eq!(tree(both).leaves[0].text.as_ref(), "A");
}

#[test]
fn positionless_usage_links_survive_insertions_beyond_the_old_window() {
    let base = "<Flow><screens><variables><name>local</name></variables><a><fieldText>{!local}</fieldText></a><z><leftValueReference>local</leftValueReference></z></screens></Flow>";
    let extra = "<m><formula>local + local + local + local</formula></m>";
    let edited = base.replace("<z>", &format!("{extra}<z>"));
    let before = featurize(&tree(base), &config(), None);
    assert!(before.iter().flatten().any(|f| f.contains(">>>")));
    let with_siblings = featurize(
        &tree(base),
        &FeatureConfig {
            n_siblings: 1,
            ..config()
        },
        None,
    );
    assert_eq!(
        with_siblings.iter().map(Vec::len).sum::<usize>()
            - before.iter().map(Vec::len).sum::<usize>(),
        2 * (tree(base).leaves.len() - 1)
    );
    assert!(included(&bag(base), &bag(&edited)));
    // In particular, an empty optional child must not leak its tag into a parent label.
    assert!(included(
        &bag(base),
        &bag(&base.replace("<z>", "<description/><z>"))
    ));
}
