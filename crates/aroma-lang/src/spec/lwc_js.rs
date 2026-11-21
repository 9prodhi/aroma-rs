//! LWC component JavaScript, pinned to tree-sitter-javascript 0.25.0.
//! Imports never affect classification; bodies and snippets share the same tables.
use super::{LangSpec, UnitShape};
use crate::snippet::{Shape, Wrapper};
use aroma_core::lang::Lang;
use aroma_core::unit::UnitKind;
use std::path::Path;
use tree_sitter::Node;

fn language() -> tree_sitter::Language {
    tree_sitter_javascript::LANGUAGE.into()
}

fn matches(rel: &str) -> bool {
    let parts: Vec<_> = rel.split('/').collect();
    parts.windows(3).enumerate().any(|(i, p)| {
        p[0] == "lwc"
            && !p[1].is_empty()
            && p[1] != "__tests__"
            && ((parts.len() == i + 3 && p[2].ends_with(".js") && p[2] != ".js")
                || (parts.len() == i + 4
                    && p[2] == "__tests__"
                    && parts[i + 3].ends_with(".js")
                    && parts[i + 3] != ".js"))
    })
}

fn is_test_path(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == "__tests__")
}

fn decorators(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|n| n.kind() == "decorator")
        .collect()
}

fn wire_call<'t>(decorator: Node<'t>, src: &[u8]) -> Option<Node<'t>> {
    let call = decorator.named_child(0)?;
    let callee = call.child_by_field_name("function")?;
    (call.kind() == "call_expression" && src.get(callee.byte_range()) == Some(b"wire".as_slice()))
        .then_some(call)
}

fn unit<'t>(_spec: &LangSpec, node: Node<'t>, src: &[u8]) -> Option<UnitShape<'t>> {
    let (kind, body, leading) = match node.kind() {
        "method_definition" => {
            let mut cursor = node.walk();
            let accessor = node
                .children(&mut cursor)
                .any(|n| !n.is_named() && matches!(n.kind(), "get" | "set"));
            (
                if accessor {
                    UnitKind::Accessor
                } else {
                    UnitKind::Method
                },
                node.child_by_field_name("body")?,
                decorators(node),
            )
        }
        "field_definition" => {
            if let Some(call) = decorators(node).into_iter().find_map(|d| wire_call(d, src)) {
                (UnitKind::WireMember, call, Vec::new())
            } else {
                let arrow = node.child_by_field_name("value")?;
                if arrow.kind() != "arrow_function" {
                    return None;
                }
                let body = arrow.child_by_field_name("body")?;
                if body.kind() != "statement_block" {
                    return None;
                }
                (UnitKind::Method, body, decorators(node))
            }
        }
        "function_declaration" => (
            UnitKind::Method,
            node.child_by_field_name("body")?,
            Vec::new(),
        ),
        "variable_declarator" => {
            let decl = node.parent()?;
            let mut cursor = decl.walk();
            if decl.kind() != "lexical_declaration"
                || !decl.children(&mut cursor).any(|n| n.kind() == "const")
            {
                return None;
            }
            let parent = decl.parent()?;
            if parent.kind() != "program"
                && !(parent.kind() == "export_statement" && parent.parent()?.kind() == "program")
            {
                return None;
            }
            let arrow = node.child_by_field_name("value")?;
            if arrow.kind() != "arrow_function" {
                return None;
            }
            let body = arrow.child_by_field_name("body")?;
            if body.kind() != "statement_block" {
                return None;
            }
            (UnitKind::Method, body, Vec::new())
        }
        _ => return None,
    };
    Some(UnitShape {
        kind,
        body,
        leading,
    })
}

pub static SPEC: LangSpec = LangSpec {
    lang: Lang::LwcJs,
    language,
    matches,
    preprocess: super::identity_preprocess,
    unit,
    feature_config: super::identity_features,
    term_rule: aroma_core::lexical::TermRule::Identifier,
    omission_marker: aroma_core::render::OMITTED_SLASH,
    repair: crate::snippet::Repair::Brackets,
    wrappers: &WRAPPERS,
    // `this` and optional_chain intentionally stay keywords. Template fragments are
    // leaves, but template_string is not atomic: substitutions remain expressions.
    leaf_kinds: &[
        "identifier",
        "property_identifier",
        "private_property_identifier",
        "shorthand_property_identifier",
        "shorthand_property_identifier_pattern",
        "statement_identifier",
        "number",
        "regex",
        "true",
        "false",
        "null",
        "undefined",
        "super",
        "string_fragment",
        "escape_sequence",
    ],
    atomic_kinds: &["string", "regex"],
    comment_kinds: &["comment"],
    identifier_kind: "identifier",
    units: &[
        ("method_definition", UnitKind::Method),
        ("field_definition", UnitKind::WireMember),
        ("function_declaration", UnitKind::Method),
        ("variable_declarator", UnitKind::Method),
    ],
    body_field: "body",
    markup_bindings: &[],
    js_scope_kinds: &[
        "statement_block",
        "arrow_function",
        "function_expression",
        "function_declaration",
        "method_definition",
        "for_in_statement",
        "switch_body",
    ],
    function_scope_kinds: &[
        "arrow_function",
        "function_expression",
        "function_declaration",
        "method_definition",
    ],
    js_snippets: Some(&super::JsSnippetPolicy {
        member_context: super::JsMemberContext::Class,
        method_kind: "method_definition",
        statement_keywords: super::JS_STATEMENT_KEYWORDS,
    }),
    decl_name_parents: &[
        "variable_declarator",
        "formal_parameters",
        "function_declaration",
        "function_expression",
        "class_declaration",
        "catch_clause",
        "arrow_function",
        "array_pattern",
        "object_pattern",
        "pair_pattern",
        "assignment_pattern",
        "object_assignment_pattern",
        "rest_pattern",
        "for_in_statement",
    ],
    non_var_parents: &[
        "class_declaration",
        "function_declaration",
        "function_expression",
        "import_specifier",
        "import_clause",
        "namespace_import",
    ],
    non_var_fields: &[
        ("call_expression", "function"),
        ("new_expression", "constructor"),
        ("pair", "key"),
        ("pair_pattern", "key"),
        ("member_expression", "property"),
        ("method_definition", "name"),
    ],
    type_decl_kinds: &["class_declaration"],
    builtin_globals: &[
        "window",
        "document",
        "console",
        "JSON",
        "Math",
        "Promise",
        "Object",
        "Array",
        "Date",
        "Number",
        "String",
        "Boolean",
        "Error",
        "Map",
        "Set",
        "setTimeout",
        "clearTimeout",
        "fetch",
        "LightningElement",
        "NavigationMixin",
        "ShowToastEvent",
        "CustomEvent",
        "FlowAttributeChangeEvent",
        "FlowNavigationNextEvent",
        "refreshApex",
        "getRecord",
        "getFieldValue",
        "getFieldDisplayValue",
        "createRecord",
        "updateRecord",
        "deleteRecord",
        "notifyRecordUpdateAvailable",
        "getRecordNotifyChange",
        "publish",
        "subscribe",
        "unsubscribe",
        "MessageContext",
        "APPLICATION_SCOPE",
        "loadScript",
        "loadStyle",
        "reduceErrors",
    ],
    global_suffixes: &[],
    fold_case: false,
    adapter_version: "lwcjs-1",
    grammar_version: "tree-sitter-javascript 0.25.0",
    normalizer: &crate::normalize::lwc_js::NORMALIZER,
    is_test_path,
};

// Each wrapper is valid JavaScript; the expression wrapper uses a module-level
// lexical declaration (Java's typed class-field initializer is invalid JavaScript).
const WRAPPERS: [Wrapper; 3] = [
    Wrapper {
        name: "statements",
        shape: Shape::Statements,
        prefix: "class __AromaQ { __aromaQ() {\n",
        suffix: "\n} }",
    },
    Wrapper {
        name: "member",
        shape: Shape::Member,
        prefix: "class __AromaQ {\n",
        suffix: "\n}",
    },
    Wrapper {
        name: "expression",
        shape: Shape::Expression,
        prefix: "const __aromaQ = (\n",
        suffix: "\n);",
    },
];
