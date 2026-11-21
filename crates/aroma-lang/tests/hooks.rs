//! Shared adapter hooks stay inert until their language specs are implemented.
use aroma_core::lang::Lang;
use aroma_lang::spec::{lang_for_path, resolve_query_lang, spec_for};

#[test]
fn uppercase_apex_suffix_routes_to_apex() {
    assert_eq!(lang_for_path("Foo.CLS", Lang::SPECCED), Some(Lang::Apex));
}

#[test]
fn routing_only_admits_specced_enabled_languages() {
    for (path, lang) in [
        ("src/A.java", Lang::Java),
        ("src/A.JAVA", Lang::Java),
        ("classes/A.CLS", Lang::Apex),
        ("triggers/A.trigger", Lang::Apex),
        ("scripts/test.apex", Lang::Apex),
        ("pages/Accounts.page", Lang::Vf),
        ("pages/Account.component", Lang::Vf),
        ("lwc/a/a.js", Lang::LwcJs),
        ("lwc/a/a.html", Lang::LwcHtml),
        ("aura/card/cardController.js", Lang::AuraJs),
        ("aura/C/C.cmp", Lang::Aura),
        ("aura/A/A.app", Lang::Aura),
    ] {
        assert_eq!(lang_for_path(path, &Lang::ALL), Some(lang));
        assert_eq!(lang_for_path(path, &[]), None);
    }
    for path in [
        "a.cls-meta.xml",
        "pages/Accounts.page-meta.xml",
        "java",
        "x.java/README",
        ".java",
    ] {
        assert_eq!(lang_for_path(path, &Lang::ALL), None);
    }
    for &lang in &Lang::ALL {
        assert_eq!(
            spec_for(lang).is_some(),
            Lang::SPECCED.contains(&lang),
            "{lang:?}"
        );
    }
}

#[test]
fn query_aliases_report_the_resolved_unsupported_language() {
    assert_eq!(resolve_query_lang("java", ""), Ok(Lang::Java));
    assert_eq!(resolve_query_lang("apex", ""), Ok(Lang::Apex));
    assert_eq!(resolve_query_lang("lwc", "run()"), Ok(Lang::LwcJs));
    assert_eq!(resolve_query_lang("lwc", ""), Ok(Lang::LwcJs));
    assert_eq!(
        resolve_query_lang("lwc", " \n<template>"),
        Ok(Lang::LwcHtml)
    );
    assert_eq!(
        resolve_query_lang("aura", "\t<aura:component>"),
        Ok(Lang::Aura)
    );
    assert_eq!(
        resolve_query_lang("aura", "({run: function() {}})"),
        Ok(Lang::AuraJs)
    );
    // VF now has an adapter: markup and expression snippets both resolve.
    assert_eq!(resolve_query_lang("vf", "<apex:page>"), Ok(Lang::Vf));
    let err = resolve_query_lang("unknown", "").unwrap_err();
    for lang in Lang::ALL {
        assert!(err.contains(lang.as_str()));
    }
}

use aroma_core::lexical::{spt_terms, TermRule};
use aroma_lang::classify::{unit_spt, VarMode};
use aroma_lang::globals::Globals;
use aroma_lang::kraw::KRaw;
use aroma_lang::spec::{java, LangSpec};
use aroma_lang::table::{
    extract_units, parse, table_unit, to_kraw, AdapterDiag, Limits, UnitShape,
};
use tree_sitter::Node;

fn leading_unit<'t>(spec: &LangSpec, node: Node<'t>, src: &[u8]) -> Option<UnitShape<'t>> {
    let mut shape = table_unit(spec, node, src)?;
    let mut cursor = node.walk();
    shape.leading = node
        .children(&mut cursor)
        .filter(|n| n.kind() == "modifiers")
        .collect();
    Some(shape)
}

#[test]
fn unit_hook_includes_leading_evidence_and_its_source_span() {
    let spec = LangSpec {
        unit: leading_unit,
        ..java::SPEC
    };
    let src = "class A {\n  @Deprecated\n  public void run() { int value = 1; }\n}";
    let limits = Limits::DEFAULT;
    let tree = parse(&spec, src, &limits).unwrap();
    let units = extract_units(&tree, src, &spec, &limits);
    let [unit] = units.as_slice() else {
        panic!("expected one unit")
    };
    assert_eq!(unit.bytes.start as usize, src.find('@').unwrap());
    assert_eq!(unit.bytes.end as usize, src.rfind(" }").unwrap() + 2);
    assert_eq!(unit.lines, 2..4);
    assert_eq!(unit.damage, 0.0);
    let KRaw::Node {
        kind: "unit",
        children,
        ..
    } = &unit.body
    else {
        panic!("expected unit wrapper")
    };
    assert_eq!(children.len(), 2);
    assert!(matches!(
        &children[0],
        KRaw::Node {
            kind: "modifiers",
            ..
        }
    ));
    let default = extract_units(&tree, src, &java::SPEC, &limits);
    assert_eq!(children[1], default[0].body);
}

#[test]
fn leading_evidence_damage_uses_sum_of_selected_node_bytes() {
    let spec = LangSpec {
        unit: leading_unit,
        ..java::SPEC
    };
    let src = "class A { @Deprecated public void run() { int broken = ; } }";
    let limits = Limits::DEFAULT;
    let tree = parse(&spec, src, &limits).unwrap();
    let method = tree
        .root_node()
        .named_child(0)
        .unwrap()
        .child_by_field_name("body")
        .unwrap()
        .named_child(0)
        .unwrap();
    let shape = leading_unit(&spec, method, src.as_bytes()).unwrap();
    let mut diag = AdapterDiag::default();
    let mut bytes = shape.body.byte_range().len();
    to_kraw(shape.body, src.as_bytes(), &spec, &limits, &mut diag);
    for node in shape.leading {
        bytes += node.byte_range().len();
        to_kraw(node, src.as_bytes(), &spec, &limits, &mut diag);
    }
    assert!(diag.damaged_bytes > 0);
    let units = extract_units(&tree, src, &spec, &limits);
    assert_eq!(
        units[0].damage,
        f64::from(diag.damaged_bytes) / bytes as f64
    );
}

#[test]
fn java_identifier_terms_and_non_metadata_feature_configs_are_unchanged() {
    let src = "class A { void run() { Account.Owner.getName(); int Status__c = 1; } }";
    let limits = Limits::DEFAULT;
    let tree = parse(&java::SPEC, src, &limits).unwrap();
    let units = extract_units(&tree, src, &java::SPEC, &limits);
    let globals = Globals::new(&java::SPEC, Vec::new());
    let spt = unit_spt(&units[0], &java::SPEC, &globals, VarMode::ContextFree).unwrap();
    let mut expected = std::collections::BTreeMap::new();
    for leaf in &spt.leaves {
        for term in aroma_core::lexical::identifier_terms(&leaf.text) {
            *expected.entry(term).or_insert(0) += 1;
        }
    }
    assert_eq!(spt_terms(&spt, java::SPEC.term_rule), expected);
    assert_eq!(java::SPEC.term_rule, TermRule::Identifier);
    for config in [
        aroma_core::features::FeatureConfig::REFERENCE,
        aroma_core::features::FeatureConfig::DEFAULT,
    ] {
        for &lang in Lang::SPECCED {
            if lang == Lang::SfMeta {
                assert_eq!(
                    (aroma_lang::spec::sfmeta::LEXICAL_SPEC.feature_config)(config),
                    config
                );
                assert_ne!((spec_for(lang).unwrap().feature_config)(config), config);
            } else {
                assert_eq!((spec_for(lang).unwrap().feature_config)(config), config);
            }
        }
    }
}

#[test]
fn schema_two_indexes_require_rebuilding() {
    use aroma_core::profile::ProfileName;
    use aroma_core::store::StoreError;
    let expected =
        aroma_lang::build::expected_manifest(Lang::SPECCED, ProfileName::Reference, false);
    let mut previous = expected.clone();
    previous.schema_version = 2;
    let Err(StoreError::Incompatible(message)) = previous.check_compatible(&expected) else {
        panic!("schema 2 must be incompatible");
    };
    assert!(message.contains("schema_version: index 2, expected 3"));
}

#[test]
fn javascript_policies_follow_spec_tables_instead_of_language_identity() {
    use aroma_core::features::{featurize, FeatureConfig};
    use aroma_lang::snippet::parse_snippet;
    for lang in [Lang::LwcJs, Lang::AuraJs] {
        let original = spec_for(lang).unwrap();
        let synthetic = LangSpec {
            lang: Lang::Java,
            ..*original
        };
        let text = if lang == Lang::LwcJs {
            "save(cmp) { if (ready) { var Map = cmp; } consume(Map); }"
        } else {
            "save: function(cmp) { if (ready) { var Map = cmp; } consume(Map); }"
        };
        let parse = |spec: &LangSpec| {
            parse_snippet(
                spec,
                text,
                &Globals::new(spec, Vec::new()),
                VarMode::ContextFree,
                &Limits::DEFAULT,
            )
        };
        let expected = parse(original);
        let actual = parse(&synthetic);
        assert_eq!(actual.wrapper, "member");
        assert_eq!(actual.damage, 0.0);
        let expected = expected.spt.unwrap();
        let actual = actual.spt.unwrap();
        assert_eq!(actual.leaves, expected.leaves);
        assert_eq!(
            featurize(&actual, &FeatureConfig::DEFAULT, None),
            featurize(&expected, &FeatureConfig::DEFAULT, None)
        );
    }
}
