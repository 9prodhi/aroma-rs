//! Aura controller, helper and renderer JavaScript (tree-sitter-javascript 0.25.0).
use super::{JsMemberContext, JsSnippetPolicy, LangSpec, UnitShape};
use crate::snippet::{Shape, Wrapper};
use aroma_core::lang::Lang;
use aroma_core::unit::UnitKind;
use tree_sitter::Node;

fn matches(rel: &str) -> bool {
    let parts: Vec<_> = rel.split('/').collect();
    parts.windows(3).enumerate().any(|(i, p)| {
        p[0] == "aura"
            && !p[1].is_empty()
            && parts.len() == i + 3
            && ["Controller.js", "Helper.js", "Renderer.js"]
                .iter()
                .any(|suffix| p[2].ends_with(suffix))
    })
}

/// Only the controller's outer object contributes units. Functions in callbacks,
/// nested objects and arbitrary variable initializers remain part of their parent.
fn unit<'t>(_spec: &LangSpec, node: Node<'t>, _src: &[u8]) -> Option<UnitShape<'t>> {
    if node.kind() != "pair" {
        return None;
    }
    let object = node.parent()?;
    if object.kind() != "object" {
        return None;
    }
    let mut parent = object.parent()?;
    while parent.kind() == "parenthesized_expression" {
        parent = parent.parent()?;
    }
    if parent.kind() != "expression_statement" || parent.parent()?.kind() != "program" {
        return None;
    }
    let value = node.child_by_field_name("value")?;
    if value.kind() != "function_expression" {
        return None;
    }
    Some(UnitShape {
        kind: UnitKind::Method,
        body: value.child_by_field_name("body")?,
        leading: Vec::new(),
    })
}

pub static SPEC: LangSpec = LangSpec {
    lang: Lang::AuraJs,
    matches,
    unit,
    units: &[("pair", UnitKind::Method)],
    wrappers: &WRAPPERS,
    js_snippets: Some(&JsSnippetPolicy {
        member_context: JsMemberContext::Object,
        method_kind: "method_definition",
        statement_keywords: super::JS_STATEMENT_KEYWORDS,
    }),
    builtin_globals: &[
        "$A",
        "sforce",
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
        "setInterval",
        "clearInterval",
        "fetch",
        "CustomEvent",
        "navigator",
        "location",
        "localStorage",
        "sessionStorage",
        "alert",
        "confirm",
        "Intl",
        "encodeURIComponent",
        "decodeURIComponent",
        "parseInt",
        "parseFloat",
        "isNaN",
        "undefined",
        "NaN",
        "Infinity",
    ],
    adapter_version: "aurajs-1",
    normalizer: &crate::normalize::aura_js::NORMALIZER,
    is_test_path: |_| false,
    // Token, declaration, scope and repair rules are shared JavaScript syntax.
    ..super::lwc_js::SPEC
};

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
        prefix: "({\n",
        suffix: "\n})",
    },
    Wrapper {
        name: "expression",
        shape: Shape::Expression,
        prefix: "const __aromaQ = (\n",
        suffix: "\n);",
    },
];
