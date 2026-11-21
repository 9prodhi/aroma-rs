//! Salesforce metadata, tree-sitter-xml 0.7.0's XML grammar (never DTD).
//!
//! Unit selection uses the root tag rather than a path hook: routed field, rule and
//! CMDT files have CustomField, ValidationRule and CustomMetadata roots; MDAPI
//! CustomObject children are fields/validationRules; Flow children use FLOW_ELEMENTS.
//! Thus the existing unit hook needs no extension and remains context-free.
use super::LangSpec;
use crate::snippet::{Repair, Shape, Wrapper};
use crate::table::UnitShape;
use aroma_core::features::{FeatureConfig, LabelMode, ParentSlot, VarUsage};
use aroma_core::lang::Lang;
use aroma_core::unit::UnitKind;
use tree_sitter::Node;

pub const FLOW_ELEMENTS: &[&str] = &[
    "decisions",
    "assignments",
    "recordLookups",
    "recordCreates",
    "recordUpdates",
    "recordDeletes",
    "loops",
    "screens",
    "subflows",
    "actionCalls",
    "formulas",
];

fn matches(rel: &str) -> bool {
    let file = rel.rsplit('/').next().unwrap_or(rel);
    [
        ".flow-meta.xml",
        ".validationRule-meta.xml",
        ".field-meta.xml",
        ".md-meta.xml",
        ".object",
        ".flow",
    ]
    .iter()
    .any(|suffix| {
        file.strip_suffix(suffix)
            .is_some_and(|stem| !stem.is_empty())
    })
}

pub fn tag_name<'s>(node: Node<'_>, src: &'s [u8]) -> Option<&'s str> {
    if node.kind() != "element" {
        return None;
    }
    let tag = node.named_child(0)?;
    let name = tag.named_child(0)?;
    (name.kind() == "Name")
        .then(|| std::str::from_utf8(&src[name.byte_range()]).ok())
        .flatten()
}

fn parent_element(node: Node<'_>) -> Option<Node<'_>> {
    let parent = node.parent()?;
    if parent.kind() == "content" {
        parent.parent()
    } else {
        None
    }
}

/// Prefixes affect labels, but unit/normalizer semantics use XML local names.
pub(crate) fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn base_kind(node: Node<'_>, src: &[u8]) -> Option<UnitKind> {
    let name = local_name(tag_name(node, src)?);
    let parent = parent_element(node);
    let parent_name = parent.and_then(|n| tag_name(n, src)).map(local_name);
    match (parent_name, name) {
        (Some("Flow"), tag) if FLOW_ELEMENTS.contains(&tag) => Some(UnitKind::FlowElement),
        (Some("CustomObject"), "fields") => Some(UnitKind::Field),
        (Some("CustomObject"), "validationRules") => Some(UnitKind::ValidationRule),
        (None, "CustomField") if node.parent()?.kind() == "document" => Some(UnitKind::Field),
        (None, "ValidationRule") if node.parent()?.kind() == "document" => {
            Some(UnitKind::ValidationRule)
        }
        (None, "CustomMetadata") if node.parent()?.kind() == "document" => {
            Some(UnitKind::MetadataRecord)
        }
        _ => None,
    }
}

/// Count meaningful named CST leaves, stopping once the threshold is exceeded.
/// Formatting whitespace and comments cannot cause recursive splitting.
fn exceeds(node: Node<'_>, src: &[u8], limit: usize) -> bool {
    let mut count = 0;
    let mut cursor = node.walk();
    loop {
        let n = cursor.node();
        if n.is_named()
            && n.named_child_count() == 0
            && n.kind() != "Comment"
            && !(n.kind() == "CharData" && src[n.byte_range()].iter().all(u8::is_ascii_whitespace))
        {
            count += 1;
            if count > limit {
                return true;
            }
        }
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return false;
            }
        }
    }
}

fn unit<'t>(_spec: &LangSpec, node: Node<'t>, src: &[u8]) -> Option<UnitShape<'t>> {
    if node.kind() != "element" {
        return None;
    }
    let kind = if let Some(kind) = base_kind(node, src) {
        kind
    } else {
        // Retain the oversize parent, emitting each child (including small children)
        // and recursing only while every ancestor back to a base unit is oversize.
        let mut parent = parent_element(node)?;
        loop {
            if !exceeds(parent, src, 2_000) {
                return None;
            }
            if base_kind(parent, src).is_some() {
                break;
            }
            parent = parent_element(parent)?;
        }
        UnitKind::MarkupBlock
    };
    Some(UnitShape {
        kind,
        body: node,
        leading: Vec::new(),
    })
}

fn features(config: FeatureConfig) -> FeatureConfig {
    FeatureConfig {
        parent_slot: ParentSlot::ChildLabel,
        label_mode: LabelMode::Fixed,
        var_usage: VarUsage::Positionless,
        n_siblings: 0,
        ..config
    }
}

/// Lexical mode retains the pre-spike normalizer, SPT and token stream.
pub static LEXICAL_SPEC: LangSpec = LangSpec {
    feature_config: super::identity_features,
    normalizer: &crate::normalize::sfmeta::LEXICAL_NORMALIZER,
    adapter_version: "sfmeta-1",
    ..SPEC
};

pub(crate) fn for_mode(
    spec: &LangSpec,
    mode: aroma_core::store::manifest::SfmetaMode,
) -> &LangSpec {
    if spec.lang != Lang::SfMeta {
        return spec;
    }
    match mode {
        aroma_core::store::manifest::SfmetaMode::Lexical => &LEXICAL_SPEC,
        aroma_core::store::manifest::SfmetaMode::Structural => &SPEC,
    }
}

/// D34 structural model, the default for new builds and unqualified queries.
pub static SPEC: LangSpec = LangSpec {
    lang: Lang::SfMeta,
    language: || tree_sitter_xml::LANGUAGE_XML.into(),
    preprocess: super::identity_preprocess,
    matches,
    unit,
    feature_config: features,
    term_rule: aroma_core::lexical::TermRule::Markup,
    omission_marker: aroma_core::render::OMITTED_MARKUP,
    repair: Repair::Tags,
    wrappers: &METADATA,
    leaf_kinds: &[
        "Name",
        "CharData",
        "AttValue",
        "CData",
        "EntityRef",
        "CharRef",
        "identifier",
        "literal",
    ],
    // AttValue hides its value text; references are opaque and never expanded by XML.
    atomic_kinds: &["AttValue", "EntityRef", "CharRef"],
    comment_kinds: &["Comment"],
    identifier_kind: "identifier",
    units: &[("element", UnitKind::FlowElement)],
    body_field: "",
    markup_bindings: &[],
    js_scope_kinds: &[],
    function_scope_kinds: &[],
    js_snippets: None,
    decl_name_parents: &[],
    non_var_parents: &[],
    non_var_fields: &[],
    type_decl_kinds: &[],
    builtin_globals: &[],
    global_suffixes: &[],
    fold_case: false,
    adapter_version: "sfmeta-2-orderless",
    grammar_version: "tree-sitter-xml 0.7.0",
    normalizer: &crate::normalize::sfmeta::NORMALIZER,
    is_test_path: |_| false,
};

const fn wrappers(prefix: &'static str, suffix: &'static str) -> [Wrapper; 3] {
    [
        Wrapper {
            name: "statements",
            shape: Shape::Statements,
            prefix,
            suffix,
        },
        Wrapper {
            name: "member",
            shape: Shape::Member,
            prefix,
            suffix,
        },
        Wrapper {
            name: "expression",
            shape: Shape::Expression,
            prefix,
            suffix,
        },
    ]
}
static FLOW: [Wrapper; 3] = wrappers("<Flow>\n", "\n</Flow>");
static OBJECT: [Wrapper; 3] = wrappers("<CustomObject>\n", "\n</CustomObject>");
static FIELD: [Wrapper; 3] = wrappers("<CustomField>\n", "\n</CustomField>");
static RULE: [Wrapper; 3] = wrappers("<ValidationRule>\n", "\n</ValidationRule>");
static RECORD: [Wrapper; 3] = wrappers("<CustomMetadata>\n", "\n</CustomMetadata>");
static METADATA: [Wrapper; 3] = wrappers("<Metadata>\n", "\n</Metadata>");

/// Snippet root table (first actual start tag, skipping comments/declarations):
/// FLOW_ELEMENTS and variables -> Flow; fields/validationRules -> CustomObject;
/// formula/type/label -> CustomField; errorConditionFormula -> ValidationRule;
/// values -> CustomMetadata; everything else -> Metadata. Synthetic roots are
/// stripped before normalization, so the guess contributes no evidence (D18).
pub fn snippet_wrappers(text: &str) -> &'static [Wrapper; 3] {
    let mut rest = text;
    let name = loop {
        let Some(start) = rest.find('<') else {
            break "";
        };
        rest = &rest[start + 1..];
        let close = if rest.starts_with("!--") {
            "-->"
        } else if rest.starts_with('?') {
            "?>"
        } else if rest.starts_with('!') {
            ">"
        } else {
            ""
        };
        if !close.is_empty() {
            let Some(end) = rest.find(close) else {
                break "";
            };
            rest = &rest[end + close.len()..];
            continue;
        }
        break rest
            .split(|c: char| c.is_whitespace() || matches!(c, '/' | '>'))
            .next()
            .unwrap_or("");
    };
    match local_name(name) {
        name if FLOW_ELEMENTS.contains(&name) || name == "variables" => &FLOW,
        "fields" | "validationRules" => &OBJECT,
        "formula" | "type" | "label" => &FIELD,
        "errorConditionFormula" => &RULE,
        "values" => &RECORD,
        _ => &METADATA,
    }
}
