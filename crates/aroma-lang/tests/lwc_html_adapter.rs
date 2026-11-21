use aroma_core::lang::Lang;
use aroma_core::unit::UnitKind;
use aroma_lang::classify::{unit_spt, VarMode};
use aroma_lang::globals::Globals;
use aroma_lang::spec::{lang_for_path, spec_for};
use aroma_lang::table::{extract_units, parse, Limits};

#[test]
fn lwc_html_routes_only_direct_component_templates() {
    assert!(spec_for(Lang::LwcHtml).is_some());
    for path in [
        "lwc/card/card.html",
        "force-app/main/default/lwc/card/card.html",
    ] {
        assert_eq!(lang_for_path(path, Lang::SPECCED), Some(Lang::LwcHtml));
    }
    for path in [
        "card.html",
        "lwc/card/__tests__/card.html",
        "lwc/card/nested/card.html",
    ] {
        assert_eq!(lang_for_path(path, Lang::SPECCED), None);
    }
}

#[test]
fn lwc_html_extracts_templates_and_binds_only_loop_names() {
    let spec = spec_for(Lang::LwcHtml).expect("LWC HTML adapter");
    let src = r#"<template><p>{title}</p><template for:each={records} for:item="row"><c-row_card key={row.Id} value={row.Name}></c-row_card></template><p>{row}</p></template>"#;
    let tree = parse(spec, src, &Limits::DEFAULT).unwrap();
    assert!(
        !tree.root_node().has_error(),
        "{}",
        tree.root_node().to_sexp()
    );
    let units = extract_units(&tree, src, spec, &Limits::DEFAULT);
    assert_eq!(units.len(), 2);
    assert!(units.iter().all(|u| u.kind == UnitKind::Template));
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
    assert_eq!(rows.len(), 4);
    assert!(rows[..3]
        .iter()
        .all(|l| l.is_var && l.binding == rows[0].binding && l.binding.is_some()));
    assert!(!rows[3].is_var);
    assert!(spt
        .leaves
        .iter()
        .filter(|l| ["title", "records", "Id", "Name"].contains(&l.text.as_ref()))
        .all(|l| !l.is_var));
}

#[test]
fn markup_snippets_repair_open_tags_and_preserve_underscores() {
    let spec = spec_for(Lang::LwcHtml).expect("LWC HTML adapter");
    let src = "<template>\n<c-nav_card title={title}>\n<span>Hello {user.name}</span>";
    let q = aroma_lang::snippet::parse_snippet(
        spec,
        src,
        &Globals::default(),
        VarMode::ContextFree,
        &Limits::DEFAULT,
    );
    assert_eq!(q.damage, 0.0);
    let spt = q.spt.expect("repaired template");
    assert!(spt.tokens(None).iter().any(|t| t.contains("c-nav_card")));
    assert!(spt.leaves.iter().all(|l| !l.is_var));
}

#[test]
fn thirty_six_constructs_have_stable_labels_and_expression_leaves() {
    let spec = spec_for(Lang::LwcHtml).expect("LWC HTML adapter");
    for (fragment, label, leaf) in [
        (
            "<template lwc:if={visible}><p>Shown</p></template>",
            "lwc:if",
            "visible",
        ),
        (
            "<template lwc:elseif={ready}><p>Ready</p></template>",
            "lwc:elseif",
            "ready",
        ),
        (
            "<template lwc:else><p>Else</p></template>",
            "lwc:else",
            "Else",
        ),
        (
            "<template if:true={visible}><p>Yes</p></template>",
            "if:true",
            "visible",
        ),
        (
            "<template if:false={hidden}><p>No</p></template>",
            "if:false",
            "hidden",
        ),
        (
            "<template for:each={rows} for:item=\"row\"><p>{row.Id}</p></template>",
            "for:each",
            "rows",
        ),
        (
            "<template iterator:it={rows}><p>{it.value}</p></template>",
            "iterator:it",
            "rows",
        ),
        ("<slot name=\"actions\"></slot>", "slot", "\"actions\""),
        (
            "<button onclick={handleClick}>Go</button>",
            "onclick",
            "handleClick",
        ),
        ("<div lwc:ref=\"panel\">Panel</div>", "lwc:ref", "\"panel\""),
        (
            "<c-child lwc:spread={props}></c-child>",
            "lwc:spread",
            "props",
        ),
        ("<input value={name}>", "input", "name"),
        ("<br><span>Line</span>", "br", "Line"),
        ("<img src=\"image.png\">", "img", "\"image.png\""),
        ("<c-child value={record.Id}/>", "c-child", "record"),
        ("<div>Hello {name} world</div>", "div", "Hello"),
        ("<div>{first}{last}</div>", "div", "last"),
        (
            "<div title=\"Hello World\">Text</div>",
            "title",
            "\"Hello World\"",
        ),
        ("<p title='Case Kept'>Text</p>", "title", "'Case Kept'"),
        (
            "<lightning-card title={title}></lightning-card>",
            "lightning-card",
            "title",
        ),
        (
            "<lightning-input onchange={handleChange}></lightning-input>",
            "onchange",
            "handleChange",
        ),
        (
            "<div data-id=\"header\">Header</div>",
            "data-id",
            "\"header\"",
        ),
        (
            "<section aria-label=\"Details\">Details</section>",
            "aria-label",
            "\"Details\"",
        ),
        ("<a href={url}>Link</a>", "href", "url"),
        (
            "<p class=\"slds-text-heading\">Title</p>",
            "class",
            "\"slds-text-heading\"",
        ),
        ("<div style={styles}>Box</div>", "style", "styles"),
        (
            "<input disabled checked value={value}>",
            "disabled",
            "value",
        ),
        (
            "<div lwc:dom=\"manual\">Body</div>",
            "lwc:dom",
            "\"manual\"",
        ),
        (
            "<slot lwc:slot-bind={record}></slot>",
            "lwc:slot-bind",
            "record",
        ),
        (
            "<template lwc:slot-data=\"item\"><p>Slot</p></template>",
            "lwc:slot-data",
            "\"item\"",
        ),
        ("<div>{account.Owner.Name}</div>", "div", "Owner"),
        ("<p>Café {greeting}</p>", "p", "Café"),
        ("<!-- drop me --><div>{value}</div>", "div", "value"),
        (
            "<c-nav_card value={value}></c-nav_card>",
            "c-nav_card",
            "value",
        ),
        ("<div title=\"{name}\">Hi</div>", "title", "name"),
        ("<p>A &amp; B</p>", "p", "A"),
    ] {
        let src = format!("<template>{fragment}</template>");
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
        assert!(!spt.tokens(None).iter().any(|t| t.contains("drop me")));
    }
}

#[test]
fn iterator_bindings_shadow_and_do_not_escape_nested_templates() {
    let spec = spec_for(Lang::LwcHtml).expect("LWC HTML adapter");
    let src = r#"<template><p>{it}</p><template iterator:it={rows}><p>{it.value} {it.index} {it.first} {it.last}</p><template for:each={children} for:item="it"><p>{it.Id}</p></template><p>{it.value}</p></template><p>{it}</p></template>"#;
    let tree = parse(spec, src, &Limits::DEFAULT).unwrap();
    let units = extract_units(&tree, src, spec, &Limits::DEFAULT);
    let spt = unit_spt(&units[0], spec, &Globals::default(), VarMode::ContextFree).unwrap();
    let it: Vec<_> = spt
        .leaves
        .iter()
        .filter(|l| l.text.as_ref() == "it")
        .collect();
    assert!(!it[0].is_var);
    assert_eq!(it[1].binding.map(|b| b.0), Some(0));
    assert!(it[2..6].iter().all(|l| l.binding == it[1].binding));
    assert_eq!(it[6].binding.map(|b| b.0), Some(1));
    assert_eq!(it[7].binding, it[6].binding);
    assert_eq!(it[8].binding, it[1].binding);
    assert!(!it[9].is_var);
    // The inner unit cannot borrow the enclosing iterator's declaration (D18).
    let nested = unit_spt(&units[2], spec, &Globals::default(), VarMode::ContextFree).unwrap();
    assert_eq!(
        nested
            .leaves
            .iter()
            .find(|l| l.is_var)
            .unwrap()
            .binding
            .map(|b| b.0),
        Some(0)
    );
}

fn build_fixture(root: &std::path::Path) -> aroma_lang::build::BuildReport {
    aroma_lang::build::build_index(&aroma_lang::build::BuildOptions {
        root: root.to_owned(),
        langs: vec![Lang::LwcHtml],
        profile: aroma_core::profile::ProfileName::Reference,
        compat: false,
        lock_timeout: std::time::Duration::ZERO,
        jobs: Some(1),
    })
    .unwrap()
}

#[test]
fn d18_lwc_html_fixture_repo_query_features_equal_index_for_200_units() {
    use aroma_core::features::{featurize, FeatureConfig};
    let repo = tempfile::tempdir().unwrap();
    let spec = spec_for(Lang::LwcHtml).expect("LWC HTML adapter");
    for i in 0..40 {
        let dir = repo
            .path()
            .join(format!("force-app/main/default/lwc/card{i}"));
        std::fs::create_dir_all(&dir).unwrap();
        let src = format!(
            r#"<template>
<lightning-card title="Card {i}"><div class="slds-grid">
<template lwc:if={{visible}}><p>Hello {{title}}</p></template>
<template for:each={{records}} for:item="row"><c-row_card key={{row.Id}} value={{row.Name}}></c-row_card>
<template lwc:if={{row.visible}}><span>{{row.Description}}</span></template></template>
<template iterator:it={{rows}}><p>{{it.value}} {{it.index}}</p></template>
</div></lightning-card></template>"#
        );
        std::fs::write(dir.join(format!("card{i}.html")), src).unwrap();
    }
    assert_eq!(build_fixture(repo.path()).units, 200);
    let globals = Globals::default();
    let mut checked = 0;
    for i in 0..40 {
        let path = repo
            .path()
            .join(format!("force-app/main/default/lwc/card{i}/card{i}.html"));
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
fn recursive_split_finds_five_middle_lines_in_5000_leaf_template() {
    let repo = tempfile::tempdir().unwrap();
    let dir = repo.path().join("lwc/large");
    std::fs::create_dir_all(&dir).unwrap();
    let mut src = String::from("<template>\n<main>\n");
    for block in 0..20 {
        src.push_str("<section>\n");
        // 50 * (start name + attribute name + value + text + end name) = 250 leaves.
        for row in 0..50 {
            src.push_str(&format!(
                "<p title=\"block{block}row{row}\">Block{block}row{row}</p>\n"
            ));
        }
        src.push_str("</section>\n");
    }
    src.push_str("</main>\n</template>");
    std::fs::write(dir.join("large.html"), &src).unwrap();
    let spec = spec_for(Lang::LwcHtml).expect("LWC HTML adapter");
    let tree = parse(spec, &src, &Limits::DEFAULT).unwrap();
    let units = extract_units(&tree, &src, spec, &Limits::DEFAULT);
    assert_eq!(units.len(), 22);
    assert_eq!(
        units
            .iter()
            .filter(|u| u.kind == UnitKind::MarkupBlock)
            .count(),
        21
    );
    build_fixture(repo.path());
    let query = (20..25)
        .map(|row| format!("<p title=\"block10row{row}\">Block10row{row}</p>\n"))
        .collect::<String>();
    let index = aroma_lang::index::Index::open(repo.path()).unwrap();
    let result = index.query(&query, &aroma_lang::index::QueryOptions::new(Lang::LwcHtml));
    let hit = result.similar.first().expect("middle snippet found");
    assert_eq!(hit.kind, UnitKind::MarkupBlock);
    assert!(hit.text.contains("block10row20"), "{}", hit.text);
}

#[test]
fn preprocess_preserves_length_and_changes_only_tag_names() {
    use std::borrow::Cow;
    let spec = spec_for(Lang::LwcHtml).unwrap();
    let src = r#"<!-- <c_comment> --><template><c_my_card title="<c_quoted>" data-key='x_y'>Café text_under {record.X__c}</c_my_card></template>"#;
    let rewritten = (spec.preprocess)(src);
    assert_eq!(rewritten.len(), src.len());
    assert_eq!(
        rewritten,
        r#"<!-- <c_comment> --><template><c-my-card title="<c_quoted>" data-key='x_y'>Café text_under {record.X__c}</c-my-card></template>"#
    );
    let clean = "<template><p>{greeting}</p></template>";
    assert!(matches!((spec.preprocess)(clean), Cow::Borrowed(_)));
    for lang in [Lang::Java, Lang::Apex] {
        assert!(matches!((spec_for(lang).unwrap().preprocess)(src), Cow::Borrowed(s) if s == src));
    }
    let tree = parse(spec, src, &Limits::DEFAULT).unwrap();
    assert!(!tree.root_node().has_error());
    let unit = &extract_units(&tree, src, spec, &Limits::DEFAULT)[0];
    let spt = unit_spt(unit, spec, &Globals::default(), VarMode::ContextFree).unwrap();
    assert!(spt.tokens(None).contains(&"c_my_card"));
    for leaf in &spt.leaves {
        assert_eq!(
            &src[leaf.bytes.start as usize..leaf.bytes.end as usize],
            leaf.text.as_ref()
        );
    }
}

#[test]
#[should_panic(expected = "preprocessing must preserve byte length")]
fn parse_asserts_preprocessor_length_contract() {
    let spec = aroma_lang::spec::LangSpec {
        preprocess: |_| std::borrow::Cow::Borrowed("short"),
        ..aroma_lang::spec::lwc_html::SPEC
    };
    let _ = parse(&spec, "<template></template>", &Limits::DEFAULT);
}

#[test]
fn markup_empty_and_comment_only_snippets_have_no_evidence() {
    let spec = spec_for(Lang::LwcHtml).unwrap();
    for src in ["", " \n\t ", "<!-- comment <c_x> -->"] {
        let q = aroma_lang::snippet::parse_snippet(
            spec,
            src,
            &Globals::default(),
            VarMode::ContextFree,
            &Limits::DEFAULT,
        );
        assert_eq!(q.damage, 0.0);
        assert!(q.spt.is_none());
    }
}

#[test]
fn loop_collection_is_evaluated_before_its_own_declaration() {
    let spec = spec_for(Lang::LwcHtml).unwrap();
    for src in [
        r#"<template iterator:it={it}><p>{it.value}</p></template>"#,
        r#"<template for:item="it" for:each={it}><p>{it.Id}</p></template>"#,
    ] {
        let tree = parse(spec, src, &Limits::DEFAULT).unwrap();
        let unit = &extract_units(&tree, src, spec, &Limits::DEFAULT)[0];
        let spt = unit_spt(unit, spec, &Globals::default(), VarMode::ContextFree).unwrap();
        let collection_offset = src.find("{it}").unwrap() as u32 + 1;
        let collection = spt
            .leaves
            .iter()
            .find(|l| l.bytes.start == collection_offset)
            .unwrap();
        assert!(
            !collection.is_var,
            "collection is a component property: {src}"
        );
        assert!(spt
            .leaves
            .iter()
            .filter(|l| l.text.as_ref() == "it" && l.bytes.start != collection_offset)
            .all(|l| l.is_var && l.binding.is_some()));
    }
}

#[test]
fn compat_marks_loop_uses_but_keeps_component_properties() {
    let spec = spec_for(Lang::LwcHtml).unwrap();
    let src = r#"<template for:each={rows} for:item="row"><p>{row.Id} {title}</p></template>"#;
    let tree = parse(spec, src, &Limits::DEFAULT).unwrap();
    let unit = &extract_units(&tree, src, spec, &Limits::DEFAULT)[0];
    let spt = unit_spt(unit, spec, &Globals::default(), VarMode::Compat).unwrap();
    assert!(spt
        .leaves
        .iter()
        .filter(|l| l.text.as_ref() == "row")
        .all(|l| l.is_var));
    assert!(spt
        .leaves
        .iter()
        .filter(|l| l.text.as_ref() != "row")
        .all(|l| !l.is_var));
}

#[test]
fn bare_property_path_uses_expression_wrapper_with_original_spans() {
    let spec = spec_for(Lang::LwcHtml).unwrap();
    let src = "account.Owner.Name";
    let q = aroma_lang::snippet::parse_snippet(
        spec,
        src,
        &Globals::default(),
        VarMode::ContextFree,
        &Limits::DEFAULT,
    );
    assert_eq!(q.wrapper, "expression");
    assert_eq!(q.damage, 0.0);
    let spt = q.spt.unwrap();
    assert_eq!(
        spt.leaves
            .iter()
            .map(|l| l.text.as_ref())
            .collect::<Vec<_>>(),
        ["account", "Owner", "Name"]
    );
    for leaf in spt.leaves {
        assert_eq!(
            &src[leaf.bytes.start as usize..leaf.bytes.end as usize],
            leaf.text.as_ref()
        );
    }
}

#[test]
fn preprocess_skips_raw_text_cdata_and_declarations() {
    let spec = spec_for(Lang::LwcHtml).unwrap();
    for (source, expected) in [
        (
            "<template><style>.x{content:'<c_fake>'}</style><c_real/></template>",
            "<template><style>.x{content:'<c_fake>'}</style><c-real/></template>",
        ),
        (
            "<template><script>const x='<c_fake';</script><c_real/></template>",
            "<template><script>const x='<c_fake';</script><c-real/></template>",
        ),
        (
            "<template><textarea><c_fake></textarea><c_real/></template>",
            "<template><textarea><c_fake></textarea><c-real/></template>",
        ),
        (
            "<!DOCTYPE html PUBLIC '<c_fake>'><template><c_real/></template>",
            "<!DOCTYPE html PUBLIC '<c_fake>'><template><c-real/></template>",
        ),
        (
            "<template><![CDATA[<c_fake>]]><c_real/></template>",
            "<template><![CDATA[<c_fake>]]><c-real/></template>",
        ),
    ] {
        let result = (spec.preprocess)(source);
        assert_eq!(result.len(), source.len());
        assert_eq!(result, expected);
    }
}

#[test]
fn oversized_self_closing_element_produces_one_block() {
    let spec = spec_for(Lang::LwcHtml).unwrap();
    let attrs = (0..1001)
        .map(|i| format!(" a{i}=\"value\""))
        .collect::<String>();
    let src = format!("<template><c-card{attrs} /></template>");
    let tree = parse(spec, &src, &Limits::DEFAULT).unwrap();
    let units = extract_units(&tree, &src, spec, &Limits::DEFAULT);
    assert_eq!(units.len(), 2);
    assert_eq!(units[1].kind, UnitKind::MarkupBlock);
}

#[test]
fn missing_loop_value_does_not_turn_attribute_name_into_a_variable() {
    let spec = spec_for(Lang::LwcHtml).unwrap();
    let src = "<template for:item><p>{title}</p></template>";
    let q = aroma_lang::snippet::parse_snippet(
        spec,
        src,
        &Globals::default(),
        VarMode::ContextFree,
        &Limits::DEFAULT,
    );
    let spt = q.spt.unwrap();
    assert!(spt
        .leaves
        .iter()
        .all(|l| !l.is_var && l.text.as_ref() != "for:item"));
    assert!(spt.tokens(None).contains(&"for:item"));
}

#[test]
fn many_text_expressions_preserve_lines_and_offsets() {
    let spec = spec_for(Lang::LwcHtml).unwrap();
    let text = "line\n{x}\n".repeat(20_000);
    let raw = aroma_lang::kraw::KRaw::Token {
        kind: "text",
        named: true,
        text: text.clone(),
        bytes: 0..text.len() as u32,
        line: 1,
        field: None,
    };
    let normalized = spec.normalizer.normalize(raw);
    let mut stack = vec![&normalized];
    let mut found = 0;
    while let Some(tree) = stack.pop() {
        match tree {
            aroma_lang::kraw::KRaw::Node { children, .. } => stack.extend(children.iter().rev()),
            aroma_lang::kraw::KRaw::Token {
                kind: "identifier",
                bytes,
                line,
                ..
            } => {
                assert_eq!(*line, 2 + found * 2);
                assert_eq!(&text[bytes.start as usize..bytes.end as usize], "x");
                found += 1;
            }
            _ => {}
        }
    }
    assert_eq!(found, 20_000);
}
