use aroma_core::lang::Lang;
use aroma_core::unit::UnitKind;
use aroma_lang::classify::{unit_spt, VarMode};
use aroma_lang::globals::Globals;
use aroma_lang::spec::{lang_for_path, spec_for};
use aroma_lang::table::{extract_units, parse, Limits};

#[test]
fn aura_routes_bundle_markup_files_and_rejects_non_markup() {
    assert!(spec_for(Lang::Aura).is_some());
    for path in [
        "force-app/main/default/aura/MyBundle/MyBundle.cmp",
        "src/main/aura/App/App.app",
        "src/main/aura/MyEvent/MyEvent.evt",
        "src/main/aura/MyInterface/MyInterface.intf",
        "src/main/aura/MyDesign/MyDesign.design",
        "src/main/aura/MyTokens/MyTokens.tokens",
        "src/main/aura/X/X.CMP",
    ] {
        assert_eq!(
            lang_for_path(path, Lang::SPECCED),
            Some(Lang::Aura),
            "{path}"
        );
    }
    for path in [
        "src/main/aura/X/X.cmp-meta.xml",
        "src/main/aura/X/X.app-meta.xml",
        "src/main/aura/X/X.css",
        "src/main/aura/X/X.svg",
        "src/main/aura/X/X.auradoc",
        "src/main/aura/X/X.svg-meta.xml",
        "aura/X.cmp",
        "src/main/aura/noBundleDir.cmp",
    ] {
        assert_eq!(lang_for_path(path, Lang::SPECCED), None, "{path}");
    }
    // Controller/helper/renderer JS belongs to the `aurajs` adapter, never to markup.
    for path in [
        "src/main/aura/X/XController.js",
        "src/main/aura/X/XHelper.js",
        "src/main/aura/X/XRenderer.js",
    ] {
        assert_eq!(
            lang_for_path(path, Lang::SPECCED),
            Some(Lang::AuraJs),
            "{path}"
        );
    }
}

#[test]
fn aura_extracts_component_and_binds_only_loop_vars() {
    let spec = spec_for(Lang::Aura).expect("Aura adapter");
    let src = r#"<aura:component>
<aura:attribute name="rows" type="Object[]"/>
<aura:iteration var="row" indexVar="i" items="{!v.rows}">
  <aura:if isTrue="{!v.show}">
    <ui:outputText value="{!row.Name}"/>
    <ui:outputText value="{!v.rows}"/>
  </aura:if>
</aura:iteration>
</aura:component>"#;
    let tree = parse(spec, src, &Limits::DEFAULT).unwrap();
    assert!(
        !tree.root_node().has_error(),
        "{}",
        tree.root_node().to_sexp()
    );
    let units = extract_units(&tree, src, spec, &Limits::DEFAULT);
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].kind, UnitKind::Component);
    let spt = unit_spt(
        &units[0],
        spec,
        &Globals::new(spec, vec![]),
        VarMode::ContextFree,
    )
    .unwrap();
    let rows: Vec<_> = spt
        .leaves
        .iter()
        .filter(|l| l.text.as_ref() == "row")
        .collect();
    assert_eq!(rows.len(), 2);
    assert!(rows[0].is_var && rows[0].binding.is_some());
    assert!(rows[1].is_var && rows[1].binding == rows[0].binding);
    // `v` and `c` are builtins, never #VAR.
    let v = spt.leaves.iter().find(|l| l.text.as_ref() == "v").unwrap();
    assert!(!v.is_var);
    let rows_prop = spt
        .leaves
        .iter()
        .find(|l| l.text.as_ref() == "rows")
        .expect("collection property present");
    assert!(!rows_prop.is_var);
}

#[test]
fn aura_hash_expressions_are_unbound_and_not_var() {
    let spec = spec_for(Lang::Aura).unwrap();
    let src = r#"<aura:component>
<div>
  <p>{#v.greeting}</p>
  <p class="x">{#IF(cond, 'a', 'b')}</p>
  <a onclick="{!c.handleClick}">Go</a>
</div>
</aura:component>"#;
    let tree = parse(spec, src, &Limits::DEFAULT).unwrap();
    assert!(!tree.root_node().has_error());
    let unit = &extract_units(&tree, src, spec, &Limits::DEFAULT)[0];
    let spt = unit_spt(unit, spec, &Globals::default(), VarMode::ContextFree).unwrap();
    let v = spt.leaves.iter().find(|l| l.text.as_ref() == "v").unwrap();
    assert!(!v.is_var, "`v` is a builtin");
    let c = spt.leaves.iter().find(|l| l.text.as_ref() == "c").unwrap();
    assert!(!c.is_var, "`c` is a builtin");
    let greeting = spt
        .leaves
        .iter()
        .find(|l| l.text.as_ref() == "greeting")
        .unwrap();
    assert!(!greeting.is_var);
}

#[test]
fn aura_half_pasted_component_is_repaired() {
    let spec = spec_for(Lang::Aura).unwrap();
    let src = r#"<aura:component>
<aura:attribute name="x" type="String"/>
<div>
<aura:iteration var="item" items="{!v.items}">
<p>{#v.item.Name}</p>"#;
    let q = aroma_lang::snippet::parse_snippet(
        spec,
        src,
        &Globals::default(),
        VarMode::ContextFree,
        &Limits::DEFAULT,
    );
    assert_eq!(q.damage, 0.0, "repair closes the open tags");
    let spt = q.spt.expect("repaired component");
    assert!(spt
        .tokens(None)
        .iter()
        .any(|t| t.contains("aura:iteration")));
    assert!(spt
        .leaves
        .iter()
        .any(|l| l.text.as_ref() == "item" && l.is_var));
}

#[test]
fn aura_entities_stay_opaque_and_never_expand() {
    let spec = spec_for(Lang::Aura).unwrap();
    let src = r#"<aura:component>
<ui:outputText value="a &amp; b"/>
<p>caf&eacute;</p>
<p>{#v.x} &lt; {#v.y}</p>
</aura:component>"#;
    let tree = parse(spec, src, &Limits::DEFAULT).unwrap();
    assert!(!tree.root_node().has_error());
    let unit = &extract_units(&tree, src, spec, &Limits::DEFAULT)[0];
    let spt = unit_spt(unit, spec, &Globals::default(), VarMode::ContextFree).unwrap();
    let tokens = spt.tokens(None);
    // Entities are opaque: the attribute value stays one literal with `&amp;` intact
    // and the text `&eacute;` / `&lt;` are never decoded.
    assert!(tokens.iter().any(|t| t.contains("&amp;")), "{tokens:?}");
    assert!(tokens.contains(&"&eacute;"), "{tokens:?}");
    assert!(tokens.contains(&"&lt;"), "{tokens:?}");
}

#[test]
fn aura_cdata_and_comments_are_handled() {
    let spec = spec_for(Lang::Aura).unwrap();
    let src = r#"<aura:component>
<!-- a comment with <tags> and {!notAnExpr} -->
<aura:attribute name="x" type="String"/>
<![CDATA[ raw <text> &amp; {!notParsed} ]]>
</aura:component>"#;
    let tree = parse(spec, src, &Limits::DEFAULT).unwrap();
    assert!(!tree.root_node().has_error());
    let unit = &extract_units(&tree, src, spec, &Limits::DEFAULT)[0];
    let spt = unit_spt(unit, spec, &Globals::default(), VarMode::ContextFree).unwrap();
    // The comment's braces are not expressions.
    assert!(spt
        .leaves
        .iter()
        .filter(|l| l.text.as_ref() == "notAnExpr" || l.text.as_ref() == "notParsed")
        .all(|l| !l.is_var));
}

#[test]
fn aura_recursive_split_splits_oversized_components() {
    let repo = tempfile::tempdir().unwrap();
    let dir = repo.path().join("force-app/main/default/aura/big");
    std::fs::create_dir_all(&dir).unwrap();
    let mut src = String::from("<aura:component>\n<div>\n");
    // ~3 named leaves per row * 120 = ~360 leaves per section, >200 threshold.
    for section in 0..10 {
        src.push_str("<aura:iteration var=\"row\" indexVar=\"i\" items=\"{!v.rows}\">\n");
        for row in 0..12 {
            src.push_str(&format!(
                "<ui:outputText value=\"section{section}row{row}\"/>\n"
            ));
        }
        src.push_str("</aura:iteration>\n");
    }
    src.push_str("</div>\n</aura:component>");
    std::fs::write(dir.join("big.cmp"), &src).unwrap();
    let spec = spec_for(Lang::Aura).expect("Aura adapter");
    let tree = parse(spec, &src, &Limits::DEFAULT).unwrap();
    assert!(!tree.root_node().has_error());
    let units = extract_units(&tree, &src, spec, &Limits::DEFAULT);
    // The root is one component plus split blocks.
    assert!(units.len() > 1);
    assert!(units.iter().any(|u| u.kind == UnitKind::Component));
    assert!(units.iter().any(|u| u.kind == UnitKind::MarkupBlock));
}

#[test]
fn aura_loop_var_scoping_does_not_escape_siblings() {
    let spec = spec_for(Lang::Aura).unwrap();
    let src = r#"<aura:component>
<aura:iteration var="item" items="{!v.a}"><ui:outputText value="{!item.Name}"/></aura:iteration>
<ui:outputText value="{!item}"/>
</aura:component>"#;
    let tree = parse(spec, src, &Limits::DEFAULT).unwrap();
    assert!(!tree.root_node().has_error());
    let unit = &extract_units(&tree, src, spec, &Limits::DEFAULT)[0];
    let spt = unit_spt(unit, spec, &Globals::default(), VarMode::ContextFree).unwrap();
    let items: Vec<_> = spt
        .leaves
        .iter()
        .filter(|l| l.text.as_ref() == "item")
        .collect();
    // The declaration inside the iteration is a var; the sibling use is not bound.
    assert!(items[0].is_var);
    assert!(items[1].is_var);
    assert!(items.iter().filter(|l| l.binding.is_some()).count() == 2);
}

#[test]
fn aura_thirty_constructs_have_stable_labels_and_expression_leaves() {
    let spec = spec_for(Lang::Aura).expect("Aura adapter");
    for (fragment, label, leaf) in [
        (
            "<aura:attribute name=\"x\" type=\"String\"/>",
            "aura:attribute",
            "\"x\"",
        ),
        (
            "<aura:registerEvent name=\"onGo\" type=\"c:goEvt\"/>",
            "aura:registerEvent",
            "\"onGo\"",
        ),
        (
            "<aura:iteration var=\"row\" items=\"{!v.rows}\"><ui:outputText value=\"{!row.Name}\"/></aura:iteration>",
            "aura:iteration",
            "row",
        ),
        (
            "<aura:if isTrue=\"{!v.show}\"><ui:outputText value=\"{!v.x}\"/></aura:if>",
            "aura:if",
            "show",
        ),
        (
            "<aura:set attribute=\"x\" value=\"{!v.y}\"/>",
            "aura:set",
            "\"x\"",
        ),
        (
            "<aura:renderIf isTrue=\"{!v.cond}\"><p>Hi</p></aura:renderIf>",
            "aura:renderIf",
            "cond",
        ),
        (
            "<ui:button label=\"Save\" press=\"{!c.save}\"/>",
            "ui:button",
            "\"Save\"",
        ),
        (
            "<lightning:buttonIcon iconName=\"utility:delete\" onclick=\"{!c.del}\"/>",
            "lightning:buttonIcon",
            "\"utility:delete\"",
        ),
        (
            "<ui:outputText value=\"{!IF(cond, a, b)}\"/>",
            "IF",
            "cond",
        ),
        (
            "<ui:outputText value=\"{!AND(a, b)}\"/>",
            "AND",
            "a",
        ),
        (
            "<ui:outputText value=\"{!$Label.c.MyLabel}\"/>",
            "value",
            "MyLabel",
        ),
        (
            "<ui:outputText value=\"{!$Resource.cssFile}\"/>",
            "value",
            "cssFile",
        ),
        (
            "<ui:outputText value=\"{!v.count > 1 ? 'many' : 'one'}\"/>",
            "value",
            "'many'",
        ),
        (
            "<ui:outputText value=\"{!v.a > 1}\"/>",
            "value",
            "a",
        ),
        (
            "<ui:outputText value=\"{!c.handleClick}\"/>",
            "value",
            "handleClick",
        ),
        (
            "<ui:outputText value=\"{!v.records[0].Name}\"/>",
            "value",
            "records",
        ),
        (
            "<ui:outputText value=\"{!v.x}&amp;{!v.y}\"/>",
            "value",
            "y",
        ),
        (
            "<div class=\"title\">{#v.title}</div>",
            "div",
            "title",
        ),
        (
            "<div onclick=\"{!c.go}\">Click</div>",
            "div",
            "go",
        ),
        (
            "<p>{#v.message}</p>",
            "p",
            "message",
        ),
        (
            "<ui:outputText value=\"{!TEXT(value)}\"/>",
            "TEXT",
            "value",
        ),
        (
            "<ui:outputText value=\"{!a + b * c}\"/>",
            "value",
            "c",
        ),
        (
            "<aura:iteration var=\"r\" items=\"{!v.list}\"><ui:outputText value=\"{!r}\"/></aura:iteration>",
            "aura:iteration",
            "r",
        ),
        (
            "<aura:if isTrue=\"{!v.b}\"><aura:set attribute=\"x\" value=\"{!v.a}\"/></aura:if>",
            "aura:set",
            "a",
        ),
        (
            "<ui:outputText value=\"{!NOT(visible)}\"/>",
            "NOT",
            "visible",
        ),
        (
            "<ui:outputText value=\"{!account.Owner.Name}\"/>",
            "value",
            "account",
        ),
        (
            "<ui:outputText value=\"{!pageTitle}\" style=\"color:red\"/>",
            "style",
            "pageTitle",
        ),
        (
            "<ui:outputText value=\"{!url}\" onclick=\"go()\"/>",
            "onclick",
            "\"go()\"",
        ),
        (
            "<lightning:input type=\"number\" label=\"Qty\" value=\"{#v.qty}\"/>",
            "lightning:input",
            "qty",
        ),
        (
            "<ui:outputText value=\"{!label.Company}\" escape=\"false\"/>",
            "escape",
            "Company",
        ),
    ] {
        let src = format!("<aura:component>{fragment}</aura:component>");
        let tree = parse(spec, &src, &Limits::DEFAULT).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "{fragment}: {}",
            tree.root_node().to_sexp()
        );
        let units = extract_units(&tree, &src, spec, &Limits::DEFAULT);
        let spt = unit_spt(&units[0], spec, &Globals::default(), VarMode::ContextFree).unwrap();
        assert!(
            spt.nodes.iter().any(|n| n.label.contains(label)),
            "missing keyword label {label}: {fragment}"
        );
        assert!(
            spt.leaves.iter().any(|l| l.text.as_ref() == leaf),
            "missing leaf {leaf}: {fragment}: {:?}",
            spt.tokens(None)
        );
    }
}

fn build_fixture(root: &std::path::Path) -> aroma_lang::build::BuildReport {
    aroma_lang::build::build_index(&aroma_lang::build::BuildOptions {
        root: root.to_owned(),
        langs: vec![Lang::Aura],
        profile: aroma_core::profile::ProfileName::Reference,
        compat: false,
        lock_timeout: std::time::Duration::ZERO,
        jobs: Some(1),
    })
    .unwrap()
}

#[test]
fn d18_aura_fixture_repo_query_features_equal_index_for_120_units() {
    use aroma_core::features::{featurize, FeatureConfig};
    let repo = tempfile::tempdir().unwrap();
    let spec = spec_for(Lang::Aura).expect("Aura adapter");
    for i in 0..120 {
        let dir = repo
            .path()
            .join(format!("force-app/main/default/aura/Comp{i}"));
        std::fs::create_dir_all(&dir).unwrap();
        let src = format!(
            r#"<aura:component>
<aura:attribute name="label" type="String"/>
<aura:iteration var="row" indexVar="i" items="{{!v.rows{i}}}">
  <ui:outputText value="{{!row.Name}}"/>
  <ui:outputText value="{{!IF(row.visible, row.Description, 'none')}}"/>
  <ui:outputText value="{{!$Label.c.{label}{i}}}"/>
</aura:iteration>
<ui:button label="Save {{i}}" press="{{!c.save{i}}}"/>
</aura:component>"#,
            label = if i % 2 == 0 { "Yes" } else { "No" },
        );
        std::fs::write(dir.join(format!("Comp{i}.cmp")), src).unwrap();
    }
    assert_eq!(build_fixture(repo.path()).units, 120);
    let globals = Globals::default();
    let mut checked = 0;
    for i in 0..120 {
        let path = repo
            .path()
            .join(format!("force-app/main/default/aura/Comp{i}/Comp{i}.cmp"));
        let src = std::fs::read_to_string(path).unwrap();
        let tree = parse(spec, &src, &Limits::DEFAULT).unwrap();
        for unit in extract_units(&tree, &src, spec, &Limits::DEFAULT) {
            let indexed = unit_spt(&unit, spec, &globals, VarMode::ContextFree).unwrap();
            let body = &src[unit.bytes.start as usize..unit.bytes.end as usize];
            let q = aroma_lang::snippet::parse_snippet(
                spec,
                body,
                &globals,
                VarMode::ContextFree,
                &Limits::DEFAULT,
            );
            assert_eq!(q.damage, 0.0, "{body}");
            let queried = q.spt.unwrap();
            assert_eq!(queried.tokens(None), indexed.tokens(None), "{body}");
            let marks = |s: &aroma_core::spt::Spt| {
                s.leaves
                    .iter()
                    .map(|l| (l.text.to_string(), l.is_var, l.binding))
                    .collect::<Vec<_>>()
            };
            assert_eq!(marks(&queried), marks(&indexed));
            assert_eq!(
                featurize(&queried, &FeatureConfig::DEFAULT, None),
                featurize(&indexed, &FeatureConfig::DEFAULT, None),
                "{body}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 120);
}
