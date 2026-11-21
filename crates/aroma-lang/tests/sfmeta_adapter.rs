use aroma_core::lang::Lang;
use aroma_core::unit::UnitKind;
use aroma_lang::classify::{unit_spt, VarMode};
use aroma_lang::globals::Globals;
use aroma_lang::spec::{lang_for_path, spec_for};
use aroma_lang::table::{extract_units, parse, Limits};

#[test]
fn sfmeta_routes_allowlist_and_extracts_flow_children() {
    for suffix in [
        ".flow-meta.xml",
        ".validationRule-meta.xml",
        ".field-meta.xml",
        ".md-meta.xml",
        ".object",
        ".flow",
    ] {
        assert_eq!(
            lang_for_path(&format!("metadata/Test{suffix}"), Lang::SPECCED),
            Some(Lang::SfMeta)
        );
    }
    for path in [
        "x.xml",
        "x.object-meta.xml",
        "x.cls-meta.xml",
        "x.field-meta.xml.bak",
    ] {
        assert_eq!(lang_for_path(path, Lang::SPECCED), None);
    }
    let spec = spec_for(Lang::SfMeta).expect("sfmeta adapter");
    let src = "<Flow><variables><name>external</name></variables><decisions><name>Check</name><leftValueReference>external</leftValueReference></decisions></Flow>";
    let tree = parse(spec, src, &Limits::DEFAULT).unwrap();
    assert!(!tree.root_node().has_error());
    let units = extract_units(&tree, src, spec, &Limits::DEFAULT);
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].kind, UnitKind::FlowElement);
    let spt = unit_spt(&units[0], spec, &Globals::default(), VarMode::ContextFree).unwrap();
    assert!(spt
        .leaves
        .iter()
        .any(|l| l.text.as_ref() == "external" && !l.is_var));
}

fn spt(src: &str) -> aroma_core::spt::Spt {
    let spec = spec_for(Lang::SfMeta).unwrap();
    let tree = parse(spec, src, &Limits::DEFAULT).unwrap();
    assert!(
        !tree.root_node().has_error(),
        "{}",
        tree.root_node().to_sexp()
    );
    let units = extract_units(&tree, src, spec, &Limits::DEFAULT);
    assert!(!units.is_empty(), "{src}");
    unit_spt(&units[0], spec, &Globals::default(), VarMode::ContextFree).unwrap()
}

fn assert_structural_construct(root: &str, tag: &str, value: &str, leaf: &str) {
    use aroma_core::features::{featurize, FeatureConfig};
    let src = format!("<{root}><{tag}>{value}</{tag}></{root}>");
    let spec = spec_for(Lang::SfMeta).unwrap();
    let cst = parse(spec, &src, &Limits::DEFAULT).unwrap();
    let units = extract_units(&cst, &src, spec, &Limits::DEFAULT);
    assert_eq!(units.len(), 1, "{src}");
    let kind = match root {
        "Flow" => UnitKind::FlowElement,
        "ValidationRule" => UnitKind::ValidationRule,
        "CustomMetadata" => UnitKind::MetadataRecord,
        "CustomObject" if tag == "validationRules" => UnitKind::ValidationRule,
        _ => UnitKind::Field,
    };
    assert_eq!(units[0].kind, kind, "{src}");
    let tree = spt(&src);
    assert!(tree.nodes.iter().any(|n| n.label.contains(tag)), "{src}");
    assert!(
        tree.leaves.iter().any(|l| l.text.as_ref() == leaf),
        "{src}: {:?}",
        tree.tokens(None)
    );
    assert!(!tree.tokens(None).iter().any(|t| t.contains("drop me")));
    assert!(!featurize(&tree, &(spec.feature_config)(FeatureConfig::DEFAULT), None).is_empty());
    for leaf in &tree.leaves {
        assert_eq!(
            &src[leaf.bytes.start as usize..leaf.bytes.end as usize],
            leaf.text.as_ref()
        );
    }
}

macro_rules! structural_constructs {
    ($($name:ident: ($root:expr, $tag:expr, $value:expr, $leaf:expr)),* $(,)?) => {
        $(#[test]
        fn $name() { assert_structural_construct($root, $tag, $value, $leaf); })*
    };
}

structural_constructs! {
    structural_decision: ("Flow", "decisions", "<name>Choose</name><rules><conditions><leftValueReference>Status__c</leftValueReference></conditions></rules>", "Status__c"),
    structural_assignment: ("Flow", "assignments", "<name>Assign</name><assignmentItems><assignToReference>account</assignToReference><operator>Assign</operator></assignmentItems>", "account"),
    structural_record_lookup: ("Flow", "recordLookups", "<name>Lookup</name><object>Account</object>", "Account"),
    structural_record_create: ("Flow", "recordCreates", "<name>Create</name><object>Contact</object>", "Contact"),
    structural_record_update: ("Flow", "recordUpdates", "<name>Update</name><inputReference>record</inputReference>", "record"),
    structural_record_delete: ("Flow", "recordDeletes", "<name>Delete</name><inputReference>record</inputReference>", "record"),
    structural_loop: ("Flow", "loops", "<name>Loop</name><collectionReference>records</collectionReference>", "records"),
    structural_screen: ("Flow", "screens", "<name>Screen</name><fields><name>Display</name><fieldText>Hello {!visitor}</fieldText></fields>", "visitor"),
    structural_subflow: ("Flow", "subflows", "<name>Child</name><flowName>ChildFlow</flowName>", "ChildFlow"),
    structural_action_call: ("Flow", "actionCalls", "<name>Action</name><actionName>emailAlert</actionName>", "emailAlert"),
    structural_flow_formula: ("Flow", "formulas", "<name>Calculate</name><expression>Amount__c * 2</expression>", "Amount__c"),
    structural_text_field: ("CustomField", "type", "Text", "Text"),
    structural_number_field: ("CustomField", "type", "Number", "Number"),
    structural_currency_field: ("CustomField", "type", "Currency", "Currency"),
    structural_percent_field: ("CustomField", "type", "Percent", "Percent"),
    structural_date_field: ("CustomField", "type", "Date", "Date"),
    structural_datetime_field: ("CustomField", "type", "DateTime", "DateTime"),
    structural_checkbox_field: ("CustomField", "type", "Checkbox", "Checkbox"),
    structural_email_field: ("CustomField", "type", "Email", "Email"),
    structural_phone_field: ("CustomField", "type", "Phone", "Phone"),
    structural_url_field: ("CustomField", "type", "Url", "Url"),
    structural_lookup_field: ("CustomField", "type", "Lookup", "Lookup"),
    structural_master_detail_field: ("CustomField", "type", "MasterDetail", "MasterDetail"),
    structural_long_text_field: ("CustomField", "type", "LongTextArea", "LongTextArea"),
    structural_picklist_field: ("CustomField", "type", "Picklist", "Picklist"),
    structural_multiselect_field: ("CustomField", "type", "MultiselectPicklist", "MultiselectPicklist"),
    structural_picklist_values: ("CustomField", "valueSet", "<valueSetDefinition><value><fullName>Open</fullName><default>true</default></value></valueSetDefinition>", "Open"),
    structural_formula_field: ("CustomField", "formula", "IF(Amount__c &gt; 0, Account.Name, 'empty')", "Amount__c"),
    structural_validation_rule: ("ValidationRule", "errorConditionFormula", "AND(ISNEW(), ISBLANK(Name))", "Name"),
    structural_cmdt_values: ("CustomMetadata", "values", "<field>Enabled__c</field><value>true</value>", "Enabled__c"),
    structural_mdapi_field: ("CustomObject", "fields", "<fullName>Status__c</fullName><type>Picklist</type>", "Status__c"),
    structural_mdapi_validation_rule: ("CustomObject", "validationRules", "<fullName>Required</fullName><errorMessage>Enter a name</errorMessage>", "Required"),
    structural_cdata_formula: ("CustomField", "formula", "<![CDATA[Amount__c > 5 && Count__c < 10]]>", "Count__c"),
    structural_entity_text: ("CustomField", "label", "Hello &amp; goodbye &#65;", "&amp;"),
    structural_namespace: ("CustomField", "label", "<m:text xmlns:m=\"urn:metadata\">Café</m:text>", "Café"),
    structural_attributes: ("CustomMetadata", "values", "<value xsi:type=\"xsd:string\" xmlns:xsi=\"urn:xsi\">Hello {!person}</value>", "person"),
    structural_comments: ("CustomField", "label", "<!-- drop me -->Visible", "Visible"),
}

#[test]
fn metadata_references_bind_only_in_unit_declarations_and_never_field_paths() {
    let tree = spt(
        r#"<Flow><variables><name>outer</name></variables><screens><name>Screen</name><variables><name>local</name></variables><leftValueReference>local</leftValueReference><elementReference>outer</elementReference><fieldText>{!local} {!outer} {!Status__c} {!Account.Name}</fieldText><leftValueReference>Status__c</leftValueReference><elementReference>Account.Name</elementReference></screens></Flow>"#,
    );
    let locals: Vec<_> = tree
        .leaves
        .iter()
        .filter(|l| l.text.as_ref() == "local")
        .collect();
    assert_eq!(locals.len(), 3);
    assert!(locals
        .iter()
        .all(|l| l.is_var && l.binding == locals[0].binding && l.binding.is_some()));
    assert!(tree
        .leaves
        .iter()
        .filter(|l| l.text.as_ref() != "local")
        .all(|l| !l.is_var));
}

#[test]
fn metadata_wrappers_repair_fragments_and_strip_synthetic_roots() {
    let spec = spec_for(Lang::SfMeta).unwrap();
    for text in [
        "<decisions><name>Check</name><rules>",
        "<fields><fullName>Status__c</fullName>",
        "<formula>Count__c + 1</formula>",
        "<errorConditionFormula>ISNEW()</errorConditionFormula>",
        "<values><field>Enabled__c</field>",
        "<label>Metadata</label>",
    ] {
        let q = aroma_lang::snippet::parse_snippet(
            spec,
            text,
            &Globals::default(),
            VarMode::ContextFree,
            &Limits::DEFAULT,
        );
        assert_eq!(q.damage, 0.0, "{text}");
        assert!(q.spt.is_some(), "{text}");
        for leaf in q.spt.unwrap().leaves {
            assert_eq!(
                &text[leaf.bytes.start as usize..leaf.bytes.end as usize],
                leaf.text.as_ref(),
                "{text}"
            );
        }
    }
}

#[test]
fn metadata_does_not_expand_entities() {
    let src = "<!DOCTYPE CustomField [<!ENTITY custom 'never expanded'> ]><CustomField><label>&custom;</label><type>Text</type></CustomField>";
    let tree = spt(src);
    assert!(tree.tokens(None).contains(&"&custom;"));
    assert!(!tree.tokens(None).contains(&"never expanded"));
    assert_eq!(
        tree.leaves
            .iter()
            .map(|l| l.text.as_ref())
            .collect::<Vec<_>>(),
        ["&custom;", "Text"]
    );
}

fn fixture_options(root: &std::path::Path) -> aroma_lang::build::BuildOptions {
    aroma_lang::build::BuildOptions {
        root: root.to_owned(),
        langs: vec![Lang::SfMeta],
        profile: aroma_core::profile::ProfileName::Reference,
        compat: false,
        lock_timeout: std::time::Duration::ZERO,
        jobs: Some(1),
    }
}

#[test]
fn d18_sfmeta_structural_query_features_equal_index_for_200_units() {
    use aroma_core::features::{featurize, unit_bag, FeatureConfig};
    use aroma_lang::build::{build_index, read_inventory, SfmetaMode, INVENTORY_FILE};
    let repo = tempfile::tempdir().unwrap();
    for i in 0..40 {
        let flow = format!("<Flow xmlns=\"urn:metadata\">\n<variables><name>outer</name></variables>\n<decisions><name>Check{i}</name><rules><conditions><leftValueReference>Status__c</leftValueReference><operator>EqualTo</operator><rightValue><stringValue>Open</stringValue></rightValue></conditions></rules></decisions>\n<assignments><name>Set{i}</name><assignmentItems><assignToReference>outer</assignToReference><value><stringValue>Hello {{!outer}}</stringValue></value></assignmentItems></assignments>\n<formulas><name>Total{i}</name><expression>Amount__c * 2</expression></formulas>\n<screens><name>Screen{i}</name><fields><name>Input</name><fieldText>{{!outer}}</fieldText></fields></screens>\n</Flow>");
        std::fs::write(repo.path().join(format!("Flow{i}.flow-meta.xml")), flow).unwrap();
        std::fs::write(repo.path().join(format!("Amount{i}.field-meta.xml")), format!("<?xml version=\"1.0\"?><CustomField xmlns=\"urn:metadata\"><fullName>Total{i}__c</fullName><formula>IF(Amount__c &gt; 0, Amount__c * 2, 0)</formula><type>Number</type></CustomField>")).unwrap();
    }
    let opts = fixture_options(repo.path()).with_sfmeta_mode(SfmetaMode::Structural);
    assert_eq!(build_index(&opts).unwrap().units, 200);
    let index = aroma_lang::index::Index::open(repo.path()).unwrap();
    let generation = repo
        .path()
        .join(format!(".aroma/gen-{}", index.manifest().generation));
    let inventory = read_inventory(&generation.join(INVENTORY_FILE)).unwrap();
    let spec = &aroma_lang::spec::sfmeta::SPEC;
    let mut checked = 0;
    for file in inventory {
        let src = std::fs::read_to_string(repo.path().join(file.path)).unwrap();
        let tree = parse(spec, &src, &Limits::DEFAULT).unwrap();
        for (unit, stored) in extract_units(&tree, &src, spec, &Limits::DEFAULT)
            .iter()
            .zip(file.units)
        {
            let indexed = unit_spt(unit, spec, &Globals::default(), VarMode::ContextFree).unwrap();
            let body = &src[unit.bytes.start as usize..unit.bytes.end as usize];
            let q = aroma_lang::snippet::parse_snippet(
                spec,
                body,
                &Globals::default(),
                VarMode::ContextFree,
                &Limits::DEFAULT,
            );
            assert_eq!(q.damage, 0.0, "{body}");
            let queried = q.spt.unwrap();
            assert_eq!(queried.tokens(None), indexed.tokens(None), "{body}");
            let config = (spec.feature_config)(FeatureConfig::DEFAULT);
            let q_features = featurize(&queried, &config, None);
            assert_eq!(q_features, featurize(&indexed, &config, None), "{body}");
            assert_eq!(
                unit_bag(Lang::SfMeta, &q_features)
                    .distinct()
                    .collect::<Vec<_>>(),
                stored.features
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 200);
}

#[test]
fn metadata_modes_control_postings_and_manifest_compatibility() {
    use aroma_lang::build::{build_index, read_inventory, SfmetaMode, INVENTORY_FILE};
    let repo = tempfile::tempdir().unwrap();
    std::fs::write(repo.path().join("Amount.field-meta.xml"), "<CustomField>\n<fullName>Double__c</fullName>\n<formula>Amount__c * 2</formula>\n<type>Number</type>\n</CustomField>\n").unwrap();
    for mode in [SfmetaMode::Lexical, SfmetaMode::Structural] {
        let opts = fixture_options(repo.path()).with_sfmeta_mode(mode);
        assert_eq!(build_index(&opts).unwrap().files_reparsed, 1);
        let index = aroma_lang::index::Index::open(repo.path()).unwrap();
        assert_eq!(index.manifest().sfmeta_mode, mode);
        let generation = repo
            .path()
            .join(format!(".aroma/gen-{}", index.manifest().generation));
        let inventory = read_inventory(&generation.join(INVENTORY_FILE)).unwrap();
        assert_eq!(
            inventory[0].units[0].features.is_empty(),
            mode == SfmetaMode::Lexical
        );
        assert_eq!(index.stats().n_postings == 0, mode == SfmetaMode::Lexical);
        let out = index.query(
            "<formula>Amount__c * 2</formula>",
            &aroma_lang::index::QueryOptions::new(Lang::SfMeta),
        );
        assert_eq!(out.similar[0].path, "Amount.field-meta.xml");
        assert_eq!(
            out.diagnostics.experimental_lexical_only,
            mode == SfmetaMode::Lexical
        );
        if mode == SfmetaMode::Lexical {
            assert_eq!(out.similar[0].score, out.similar[0].bm25.unwrap());
            assert!(out.similar[0].text.contains("<type>Number</type>"));
        }
        assert!(aroma_lang::index::Index::open_with_sfmeta_mode(repo.path(), mode).is_ok());
        let other = if mode == SfmetaMode::Lexical {
            SfmetaMode::Structural
        } else {
            SfmetaMode::Lexical
        };
        let mismatch = aroma_lang::index::Index::open_with_sfmeta_mode(repo.path(), other)
            .err()
            .unwrap()
            .to_string();
        assert!(mismatch.contains("re-index required") && mismatch.contains("sfmeta_mode"));
        let mut expected = index.manifest().clone();
        expected.sfmeta_mode = if mode == SfmetaMode::Lexical {
            SfmetaMode::Structural
        } else {
            SfmetaMode::Lexical
        };
        let error = index
            .manifest()
            .check_compatible(&expected)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("re-index required") && error.contains("sfmeta_mode"),
            "{error}"
        );
        assert_eq!(build_index(&opts).unwrap().files_unchanged, 1);
    }
}

#[test]
fn oversized_flow_screen_splits_recursively_and_lexical_admits_whole_unit() {
    use aroma_lang::build::{build_index, SfmetaMode};
    let repo = tempfile::tempdir().unwrap();
    let fields = (0..1500).map(|i| format!("<fields>\n<name>Field{i}</name>\n<fieldText>ScreenValue{i}</fieldText>\n</fields>\n")).collect::<String>();
    let src = format!("<Flow>\n<screens>\n<name>LargeScreen</name>\n{fields}</screens>\n</Flow>");
    std::fs::write(repo.path().join("Screen.flow-meta.xml"), &src).unwrap();
    let spec = spec_for(Lang::SfMeta).unwrap();
    let tree = parse(spec, &src, &Limits::DEFAULT).unwrap();
    let units = extract_units(&tree, &src, spec, &Limits::DEFAULT);
    assert_eq!(units[0].kind, UnitKind::FlowElement);
    assert!(
        units
            .iter()
            .filter(|u| u.kind == UnitKind::MarkupBlock)
            .count()
            >= 1500
    );
    let opts = fixture_options(repo.path()).with_sfmeta_mode(SfmetaMode::Lexical);
    build_index(&opts).unwrap();
    let index = aroma_lang::index::Index::open(repo.path()).unwrap();
    assert_eq!(
        index.stats().oversize_units_by_kind.get("flowelement"),
        Some(&1)
    );
    assert!(index.stats().split_units_by_kind["markupblock"] >= 1500);
    let mut query = aroma_lang::index::QueryOptions::new(Lang::SfMeta);
    query.kind = Some(UnitKind::FlowElement);
    let out = index.query("<fieldText>ScreenValue300</fieldText>", &query);
    assert_eq!(out.similar.len(), 1);
    assert!(out.similar[0].text.contains("ScreenValue300"));
    assert!(!out.similar[0].text.contains("omitted"));
    let opts = fixture_options(repo.path()).with_sfmeta_mode(SfmetaMode::Structural);
    build_index(&opts).unwrap();
    let index = aroma_lang::index::Index::open(repo.path()).unwrap();
    query.kind = Some(UnitKind::MarkupBlock);
    let out = index.query(
        "<fields><name>Field300</name><fieldText>ScreenValue300</fieldText></fields>",
        &query,
    );
    assert!(!out.similar.is_empty());
    assert!(out.similar[0].text.contains("ScreenValue300"));
}

#[test]
fn prefixed_metadata_names_still_select_units_and_parse_formulas() {
    let src = "<md:CustomField xmlns:md=\"urn:metadata\"><md:formula>Amount__c + 2</md:formula></md:CustomField>";
    let tree = spt(src);
    assert!(tree.leaves.iter().any(|l| l.text.as_ref() == "Amount__c"));
    assert!(tree.nodes.iter().any(|n| n.label.contains('+')));
}

#[test]
fn metadata_formula_entities_cdata_merge_fields_keep_exact_identifier_spans() {
    let src = "<CustomField>\n<label>Café</label>\n<formula>IF(Amount__c &gt; 1 &amp;&amp; Count__c &lt; 9,\n<![CDATA[Total__c + 2]]>, {!Fallback__c})</formula>\n</CustomField>";
    let tree = spt(src);
    for text in ["Amount__c", "Count__c", "Total__c", "Fallback__c"] {
        let leaf = tree
            .leaves
            .iter()
            .find(|l| l.text.as_ref() == text)
            .unwrap();
        assert_eq!(
            &src[leaf.bytes.start as usize..leaf.bytes.end as usize],
            text
        );
        assert_eq!(
            leaf.line,
            if ["Total__c", "Fallback__c"].contains(&text) {
                4
            } else {
                3
            }
        );
        assert!(!leaf.is_var);
    }
    assert!(!tree.tokens(None).contains(&"#ERR"));
}

#[test]
fn forward_unit_declarations_bind_in_both_var_modes_but_field_names_never_do() {
    let src = "<Flow><screens><fieldText>{!local} {!Status__c} {!Account.Name}</fieldText><variables><name>local</name></variables><variables><name>Status__c</name></variables><variables><name>Account</name></variables></screens></Flow>";
    let spec = spec_for(Lang::SfMeta).unwrap();
    let tree = parse(spec, src, &Limits::DEFAULT).unwrap();
    let units = extract_units(&tree, src, spec, &Limits::DEFAULT);
    for mode in [VarMode::ContextFree, VarMode::Compat] {
        let tree = unit_spt(&units[0], spec, &Globals::default(), mode).unwrap();
        assert!(tree
            .leaves
            .iter()
            .filter(|l| l.text.as_ref() == "local")
            .all(|l| l.is_var));
        assert!(tree
            .leaves
            .iter()
            .filter(|l| ["Status__c", "Name"].contains(&l.text.as_ref()))
            .all(|l| !l.is_var));
        let account_path = tree
            .leaves
            .iter()
            .find(|l| l.text.as_ref() == "Account")
            .unwrap();
        assert!(!account_path.is_var);
    }
}

#[test]
fn lexical_mode_keeps_clones_filters_language_and_kind_and_handles_stale_sources() {
    use aroma_lang::build::build_index;
    use aroma_lang::index::{Index, QueryOptions};
    let repo = tempfile::tempdir().unwrap();
    let source =
        "<CustomField>\n<fullName>UrgentCustomer__c</fullName>\n<type>Text</type>\n</CustomField>";
    for file in ["A.field-meta.xml", "B.field-meta.xml"] {
        std::fs::write(repo.path().join(file), source).unwrap();
    }
    std::fs::write(
        repo.path().join("C.validationRule-meta.xml"),
        "<ValidationRule><fullName>UrgentCustomer__c</fullName></ValidationRule>",
    )
    .unwrap();
    std::fs::write(
        repo.path().join("D.java"),
        "class D { void m() { int UrgentCustomer__c = 1; } }",
    )
    .unwrap();
    let mut build = fixture_options(repo.path());
    build.langs.clear();
    build.compat = true;
    build_index(&build.with_sfmeta_mode(aroma_lang::build::SfmetaMode::Lexical)).unwrap();
    let index = Index::open(repo.path()).unwrap();
    let mut opts = QueryOptions::new(Lang::SfMeta);
    opts.kind = Some(UnitKind::Field);
    opts.lexical = false;
    opts.experimental_recommend = true;
    let out = index.query("UrgentCustomer__c", &opts);
    assert_eq!(out.similar.len(), 2, "lexical mode keeps duplicate units");
    assert!(out.similar.iter().all(|hit| hit.kind == UnitKind::Field
        && hit.score == hit.bm25.unwrap()
        && hit.structural_rank == 0));
    assert!(out.recommendations.is_empty());
    assert!(out.diagnostics.experimental_lexical_only);
    assert_eq!(out.diagnostics.engine.query_features, 0);
    opts.budget = Some(std::time::Duration::ZERO);
    let out = index.query("UrgentCustomer__c", &opts);
    assert!(out.similar.is_empty());
    assert_eq!(out.diagnostics.engine.budget_exhausted, Some("lexical"));
    opts.budget = None;
    std::fs::remove_file(repo.path().join("A.field-meta.xml")).unwrap();
    let out = index.query("UrgentCustomer__c", &opts);
    let stale = out
        .similar
        .iter()
        .find(|hit| hit.path == "A.field-meta.xml")
        .unwrap();
    assert!(stale.stale_source && stale.text.contains("UrgentCustomer__c"));
}

#[test]
fn new_metadata_builds_and_unqualified_specs_are_structural() {
    use aroma_lang::build::SfmetaMode;
    use aroma_lang::build::{build_index, expected_manifest};
    use aroma_lang::index::{Index, QueryOptions};
    let repo = tempfile::tempdir().unwrap();
    std::fs::write(repo.path().join("A.field-meta.xml"), "<CustomField><fullName>Total__c</fullName><formula>Amount__c * 2</formula><type>Number</type></CustomField>").unwrap();
    let opts = fixture_options(repo.path());
    build_index(&opts).unwrap();
    let index = Index::open(repo.path()).unwrap();
    assert_eq!(index.manifest().sfmeta_mode, SfmetaMode::Structural);
    assert_eq!(
        expected_manifest(&opts.langs, opts.profile, opts.compat).sfmeta_mode,
        SfmetaMode::Structural
    );
    assert!(std::ptr::eq(
        spec_for(Lang::SfMeta).unwrap(),
        &aroma_lang::spec::sfmeta::SPEC
    ));
    assert!(index.stats().n_postings > 0);
    let out = index.query(
        "<formula>Amount__c * 2</formula>",
        &QueryOptions::new(Lang::SfMeta),
    );
    assert!(!out.diagnostics.experimental_lexical_only);
    assert!(out.diagnostics.engine.query_features > 0);
    assert_eq!(out.similar[0].path, "A.field-meta.xml");
}

#[test]
fn manifest_without_metadata_mode_opens_and_queries_as_lexical() {
    use aroma_lang::build::{build_index, SfmetaMode};
    use aroma_lang::index::{Index, QueryOptions};
    let repo = tempfile::tempdir().unwrap();
    std::fs::write(
        repo.path().join("A.field-meta.xml"),
        "<CustomField><fullName>UrgentCustomer__c</fullName><type>Text</type></CustomField>",
    )
    .unwrap();
    build_index(&fixture_options(repo.path()).with_sfmeta_mode(SfmetaMode::Lexical)).unwrap();
    let index = Index::open(repo.path()).unwrap();
    let manifest = repo.path().join(format!(
        ".aroma/gen-{}/manifest.json",
        index.manifest().generation
    ));
    let mut json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
    json.as_object_mut().unwrap().remove("sfmeta_mode");
    std::fs::write(manifest, serde_json::to_vec(&json).unwrap()).unwrap();
    let legacy = Index::open(repo.path()).unwrap();
    assert_eq!(legacy.manifest().sfmeta_mode, SfmetaMode::Lexical);
    let out = legacy.query("UrgentCustomer__c", &QueryOptions::new(Lang::SfMeta));
    assert!(out.diagnostics.experimental_lexical_only);
    assert_eq!(out.similar[0].score, out.similar[0].bm25.unwrap());
    // A subsequent unqualified build changes the mode and fully rebuilds.
    assert_eq!(
        build_index(&fixture_options(repo.path()))
            .unwrap()
            .files_reparsed,
        1
    );
    assert_eq!(
        Index::open(repo.path()).unwrap().manifest().sfmeta_mode,
        SfmetaMode::Structural
    );
}

#[test]
fn malformed_metadata_accounts_only_error_bytes_inside_the_unit() {
    use aroma_lang::table::{to_kraw, AdapterDiag};
    let spec = spec_for(Lang::SfMeta).unwrap();
    let src = "<CustomField><fullName>Total__c</fullName><label bad=>Good</label><type>Text</type></CustomField>";
    let cst = parse(spec, src, &Limits::DEFAULT).unwrap();
    assert!(cst.root_node().has_error());
    let units = extract_units(&cst, src, spec, &Limits::DEFAULT);
    assert_eq!(units.len(), 1);
    let mut diag = AdapterDiag::default();
    let body = cst.root_node().named_child(0).unwrap();
    to_kraw(body, src.as_bytes(), spec, &Limits::DEFAULT, &mut diag).unwrap();
    assert_eq!(diag.error_nodes, 1);
    assert_eq!(diag.damaged_bytes, 4); // The malformed `bad=`, not the whole field.
    assert_eq!(units[0].damage, 4.0 / src.len() as f64);
    let tree = unit_spt(&units[0], spec, &Globals::default(), VarMode::ContextFree).unwrap();
    assert!(tree.tokens(None).contains(&"#ERR"));
    assert!(tree.tokens(None).contains(&"Total__c"));
    let query = aroma_lang::snippet::parse_snippet(
        spec,
        src,
        &Globals::default(),
        VarMode::ContextFree,
        &Limits::DEFAULT,
    );
    assert_eq!(query.damage, units[0].damage);
    assert!(query.spt.unwrap().tokens(None).contains(&"#ERR"));
}

proptest::proptest! {
    #[test]
    fn d34_prior_evidence_survives_child_reordering_and_insertion(
        tag in proptest::sample::select(aroma_lang::spec::sfmeta::FLOW_ELEMENTS),
        seed in proptest::prelude::any::<usize>(),
        extra in "[A-Za-z]{1,20}",
    ) {
        use aroma_core::features::{featurize, unit_bag, FeatureConfig};
        let bag = |src: &str| {
            let spec = spec_for(Lang::SfMeta).unwrap();
            unit_bag(Lang::SfMeta, &featurize(&spt(src), &(spec.feature_config)(FeatureConfig::DEFAULT), None))
        };
        let children = [
            "<name>Step</name>",
            "<variables><name>local</name></variables>",
            "<rules><name>Named</name><conditions><leftValueReference>local</leftValueReference><operator>EqualTo</operator></conditions></rules>",
            "<rules><conditions><leftValueReference>Status__c</leftValueReference></conditions></rules>",
            "<rules><conditions><leftValueReference>Count__c</leftValueReference></conditions></rules>",
            "<expression>IF(local &gt; 0, Amount__c, local)</expression>",
        ];
        let before = format!("<Flow><{tag}>{}</{tag}></Flow>", children.join(""));
        let mut reordered = children.to_vec();
        let len = reordered.len();
        reordered.rotate_left(seed % len);
        if seed % 2 == 0 { reordered.reverse(); }
        let reordered = reordered.join("").replace(
            "<leftValueReference>local</leftValueReference><operator>EqualTo</operator>",
            "<operator>EqualTo</operator><leftValueReference>local</leftValueReference>"
        );
        let reordered = format!("<Flow><{tag}>{reordered}</{tag}></Flow>");
        let original = bag(&before);
        let shuffled = bag(&reordered);
        proptest::prop_assert_eq!(&original.intersect(&shuffled), &original);
        let inserted = reordered.replace(&format!("</{tag}>"), &format!("<description>{extra}</description></{tag}>"));
        proptest::prop_assert_eq!(&original.intersect(&bag(&inserted)), &original);
    }
}
