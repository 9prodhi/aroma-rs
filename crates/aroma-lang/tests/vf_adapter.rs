use aroma_core::lang::Lang;
use aroma_core::unit::UnitKind;
use aroma_lang::classify::{unit_spt, VarMode};
use aroma_lang::globals::Globals;
use aroma_lang::spec::{lang_for_path, spec_for};
use aroma_lang::table::{extract_units, parse, Limits};

#[test]
fn vf_routes_page_and_component_files() {
    assert!(spec_for(Lang::Vf).is_some());
    for path in [
        "force-app/main/default/pages/Account.page",
        "src/pages/MyComp.component",
        "pages/Accounts.PAGE",
    ] {
        assert_eq!(lang_for_path(path, Lang::SPECCED), Some(Lang::Vf));
    }
    for path in [
        "pages/Accounts.page-meta.xml",
        "pages/Accounts.component-meta.xml",
        "pages/Accounts.html",
    ] {
        assert_eq!(lang_for_path(path, Lang::SPECCED), None);
    }
}

#[test]
fn vf_extracts_page_and_binds_only_loop_vars() {
    let spec = spec_for(Lang::Vf).expect("VF adapter");
    let src = r#"<apex:page controller="Ctr"><apex:repeat value="{!rows}" var="row"><apex:outputText value="{!row.Name}"/><apex:outputText value="{!rows}"/></apex:repeat></apex:page>"#;
    let tree = parse(spec, src, &Limits::DEFAULT).unwrap();
    assert!(
        !tree.root_node().has_error(),
        "{}",
        tree.root_node().to_sexp()
    );
    let units = extract_units(&tree, src, spec, &Limits::DEFAULT);
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].kind, UnitKind::Page);
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
    // The controller property `{!rows}` is a separate leaf (spelled "rows") and never #VAR.
    let collection = spt
        .leaves
        .iter()
        .find(|l| l.text.as_ref() == "rows")
        .expect("collection property present");
    assert!(!collection.is_var);
    assert!(spt
        .leaves
        .iter()
        .filter(|l| ["Ctr", "Name"].contains(&l.text.as_ref()))
        .all(|l| !l.is_var));
}

#[test]
fn vf_script_and_style_stay_one_literal_token() {
    let spec = spec_for(Lang::Vf).unwrap();
    let src = r#"<apex:page><script>function {!Name}(){ let x = '{!y}'; }</script><style>.x{color:red}</style></apex:page>"#;
    let tree = parse(spec, src, &Limits::DEFAULT).unwrap();
    assert!(!tree.root_node().has_error());
    let unit = &extract_units(&tree, src, spec, &Limits::DEFAULT)[0];
    let spt = unit_spt(unit, spec, &Globals::default(), VarMode::ContextFree).unwrap();
    // The script body is ONE literal leaf; the merge syntax inside is not an expression.
    let scripts: Vec<_> = spt
        .leaves
        .iter()
        .filter(|l| l.text.contains("{!Name}") || l.text.contains("{!y}"))
        .collect();
    assert_eq!(scripts.len(), 1); // "{!Name}" and "{!y}" are inside one raw_text.
    assert!(scripts.iter().all(|l| !l.is_var));
}

#[test]
fn vf_half_pasted_page_is_repaired() {
    let spec = spec_for(Lang::Vf).unwrap();
    let src = "<apex:page controller=\"Ctr\">\\n<apex:form>\\n<apex:pageBlock title=\"A\">\\n<apex:outputText value=\"{!x}\"/>";
    let q = aroma_lang::snippet::parse_snippet(
        spec,
        src,
        &Globals::default(),
        VarMode::ContextFree,
        &Limits::DEFAULT,
    );
    assert_eq!(q.damage, 0.0, "repair closes the open tags");
    let spt = q.spt.expect("repaired page");
    assert!(spt
        .tokens(None)
        .iter()
        .any(|t| t.contains("apex:pageBlock")));
    assert!(spt
        .leaves
        .iter()
        .filter(|l| l.text.as_ref() == "x")
        .all(|l| !l.is_var));
}

fn build_fixture(root: &std::path::Path) -> aroma_lang::build::BuildReport {
    aroma_lang::build::build_index(&aroma_lang::build::BuildOptions {
        root: root.to_owned(),
        langs: vec![Lang::Vf],
        profile: aroma_core::profile::ProfileName::Reference,
        compat: false,
        lock_timeout: std::time::Duration::ZERO,
        jobs: Some(1),
    })
    .unwrap()
}

#[test]
fn d18_vf_fixture_repo_query_features_equal_index_for_200_units() {
    use aroma_core::features::{featurize, FeatureConfig};
    let repo = tempfile::tempdir().unwrap();
    let spec = spec_for(Lang::Vf).expect("VF adapter");
    for i in 0..200 {
        let dir = repo.path().join("force-app/main/default/pages");
        std::fs::create_dir_all(&dir).unwrap();
        let src = format!(
            r#"<apex:page controller="Ctr{i}"><apex:pageBlock title="Block {i}">
<apex:repeat value="{{!rows}}" var="row"><apex:outputText value="{{!row.Name}}"/>
<apex:outputText value="{{!IF(row.visible, row.Description, 'none')}}"/></apex:repeat>
<apex:outputText value="{{!$Label.c.{label}{i}}}"/>
<apex:commandButton action="{{!save{i}}}" value="Save {{i}}"/>
</apex:pageBlock></apex:page>"#,
            label = if i % 2 == 0 { "Yes" } else { "No" },
        );
        std::fs::write(dir.join(format!("Page{i}.page")), src).unwrap();
    }
    assert_eq!(build_fixture(repo.path()).units, 200);
    let globals = Globals::default();
    let mut checked = 0;
    for i in 0..200 {
        let path = repo
            .path()
            .join(format!("force-app/main/default/pages/Page{i}.page"));
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
    assert_eq!(checked, 200);
}

#[test]
fn vf_thirty_six_constructs_have_stable_labels_and_expression_leaves() {
    let spec = spec_for(Lang::Vf).expect("VF adapter");
    for (fragment, label, leaf) in [
        (
            "<apex:outputText value=\"{!greeting}\"/>",
            "apex:outputText",
            "greeting",
        ),
        (
            "<apex:repeat value=\"{!rows}\" var=\"row\"><apex:outputText value=\"{!row.Name}\"/></apex:repeat>",
            "apex:repeat",
            "row",
        ),
        (
            "<apex:pageBlockTable value=\"{!items}\" var=\"it\"><apex:column value=\"{!it.Id}\"/></apex:pageBlockTable>",
            "apex:pageBlockTable",
            "it",
        ),
        (
            "<apex:dataTable value=\"{!items}\" var=\"d\"><apex:column value=\"{!d.Name}\"/></apex:dataTable>",
            "apex:dataTable",
            "d",
        ),
        (
            "<apex:dataList value=\"{!items}\" var=\"l\"><apex:outputText value=\"{!l.Name}\"/></apex:dataList>",
            "apex:dataList",
            "l",
        ),
        (
            "<apex:outputText value=\"{!IF(cond, a, b)}\"/>",
            "IF",
            "cond",
        ),
        (
            "<apex:outputText value=\"{!AND(a, b)}\"/>",
            "AND",
            "a",
        ),
        (
            "<apex:outputText value=\"{!$Label.c.MyLabel}\"/>",
            "value",
            "MyLabel",
        ),
        (
            "<apex:outputText value=\"{!$Resource.cssFile}\"/>",
            "value",
            "cssFile",
        ),
        (
            "<apex:commandButton action=\"{!save}\" value=\"Save\"/>",
            "apex:commandButton",
            "save",
        ),
        (
            "<apex:includeScript value=\"/resource/MyScript\"/>",
            "apex:includeScript",
            "\"/resource/MyScript\"",
        ),
        (
            "<apex:form><apex:inputText value=\"{!search}\"/></apex:form>",
            "apex:form",
            "search",
        ),
        (
            "<apex:pageBlock title=\"Block\"><apex:outputText value=\"{!x}\"/></apex:pageBlock>",
            "apex:pageBlock",
            "x",
        ),
        (
            "<apex:outputPanel id=\"panel\">Hello {!name} world</apex:outputPanel>",
            "apex:outputPanel",
            "name",
        ),
        (
            "<apex:outputText value=\"{!account.Owner.Name}\"/>",
            "value",
            "account",
        ),
        (
            "<apex:outputText value=\"{!v.count > 1 ? 'many' : 'one'}\"/>",
            "value",
            "'many'",
        ),
        (
            "<apex:outputText value=\"{!Name}&amp;{!Title}\"/>",
            "apex:outputText",
            "Title",
        ),
        (
            "<apex:page controller=\"Ctr\"><apex:sectionHeader title=\"{!t}\"/></apex:page>",
            "apex:sectionHeader",
            "t",
        ),
        (
            "<apex:pageMessages rendered=\"{!show}\"/>",
            "apex:pageMessages",
            "show",
        ),
        (
            "<apex:pageBlockSection title=\"Details\"><apex:outputField value=\"{!x}\"/></apex:pageBlockSection>",
            "apex:pageBlockSection",
            "\"Details\"",
        ),
        (
            "<apex:selectList value=\"{!pick}\" size=\"1\"><apex:selectOption value=\"A\"/></apex:selectList>",
            "apex:selectList",
            "pick",
        ),
        (
            "<apex:outputText value=\"{!records[0].Name}\"/>",
            "apex:outputText",
            "records",
        ),
        (
            "<apex:variable value=\"{!i}\" var=\"idx\"><apex:outputText value=\"{!idx}\"/></apex:variable>",
            "apex:variable",
            "idx",
        ),
        (
            "<apex:outputText value=\"{!pageTitle}\" style=\"color:red\"/>",
            "style",
            "pageTitle",
        ),
        (
            "<apex:outputText value=\"{!label.Company}\" escape=\"false\"/>",
            "escape",
            "Company",
        ),
        (
            "<apex:outputText value=\"{!IF(a, b, IF(c, d, e))}\"/>",
            "IF",
            "e",
        ),
        (
            "<apex:outputText value=\"{!NOT(visible)}\"/>",
            "NOT",
            "visible",
        ),
        (
            "<apex:outputText value=\"{!url}\" onclick=\"go()\"/>",
            "onclick",
            "\"go()\"",
        ),
        (
            "<apex:outputText value=\"{!TEXT(value)}\"/>",
            "TEXT",
            "value",
        ),
        (
            "<apex:outputText value=\"{!a + b * c}\"/>",
            "value",
            "c",
        ),
        (
            "<apex:outputText value=\"{!v.items[0].Id}\"/>",
            "apex:outputText",
            "items",
        ),
        (
            "<apex:commandLink action=\"{!go}\">Link</apex:commandLink>",
            "apex:commandLink",
            "go",
        ),
        (
            "<apex:actionFunction name=\"doIt\" action=\"{!act}\"/>",
            "apex:actionFunction",
            "act",
        ),
        (
            "<apex:outputText value=\"{!URLFOR($Page.home, null)}\"/>",
            "value",
            "home",
        ),
        (
            "<apex:outputText value=\"{!case.Status}\" styleClass=\"slds-text\"/>",
            "styleClass",
            "Status",
        ),
        (
            "<apex:outputText value=\"{!account.Name}\" rendered=\"{!show}\"/>",
            "rendered",
            "show",
        ),
    ] {
        let src = format!("<apex:page>{fragment}</apex:page>");
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

#[test]
fn recursive_split_finds_middle_lines_in_3000_leaf_vf_page() {
    let repo = tempfile::tempdir().unwrap();
    let dir = repo.path().join("pages");
    std::fs::create_dir_all(&dir).unwrap();
    let mut src = String::from("<apex:page>\n<apex:form>\n");
    for block in 0..12 {
        src.push_str("<apex:pageBlockSection>\n");
        // 80 rows * ~3 named leaves = ~240 leaves per section, so each section is a
        // MarkupBlock and the >2000-leaf form is split into its section children.
        for row in 0..80 {
            src.push_str(&format!(
                "<apex:outputText value=\"block{block}row{row}\"/>\n"
            ));
        }
        src.push_str("</apex:pageBlockSection>\n");
    }
    src.push_str("</apex:form>\n</apex:page>");
    std::fs::write(dir.join("big.page"), &src).unwrap();
    let spec = spec_for(Lang::Vf).expect("VF adapter");
    let tree = parse(spec, &src, &Limits::DEFAULT).unwrap();
    let units = extract_units(&tree, &src, spec, &Limits::DEFAULT);
    assert!(units.len() > 1);
    assert!(units.iter().any(|u| u.kind == UnitKind::MarkupBlock));
    build_fixture(repo.path());
    let query = (20..25)
        .map(|row| format!("<apex:outputText value=\"block6row{row}\"/>\n"))
        .collect::<String>();
    let index = aroma_lang::index::Index::open(repo.path()).unwrap();
    let result = index.query(&query, &aroma_lang::index::QueryOptions::new(Lang::Vf));
    let hit = result
        .similar
        .iter()
        .find(|h| h.kind == UnitKind::MarkupBlock && h.text.contains("block6row20"))
        .expect("a split MarkupBlock holds the middle snippet");
    // The specific section containing block6 is a tight match, not the whole page.
    assert!(
        hit.lines[0] >= 490 && hit.lines[1] <= 580,
        "section hit lines: {:?}",
        hit.lines
    );
}
