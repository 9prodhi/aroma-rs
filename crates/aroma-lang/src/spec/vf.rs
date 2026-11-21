//! Visualforce pages and components, tree-sitter-html 0.23.2 (D35/D44/D46).
//!
//! A `.page` / `.component` file is a single root `apex:page` / `apex:component`
//! element. That root is one `Page` unit; a nested element whose subtree exceeds 200
//! named leaves (notably `apex:form`, `apex:pageBlock`, `apex:pageBlockTable`,
//! `apex:repeat`, `apex:dataTable`, `apex:outputPanel`) becomes a `MarkupBlock` unit,
//! with a D44 recursive split above 2,000 leaves into its child elements. Inline
//! `<script>`/`<style>` bodies are raw text and stay one literal token each.
use super::{LangSpec, MarkupBinding};
use crate::snippet::{Shape, Wrapper};
use crate::table::UnitShape;
use aroma_core::lang::Lang;
use aroma_core::unit::UnitKind;
use tree_sitter::Node;

/// Root elements that are the `Page` unit.
const PAGE_ROOTS: &[&[u8]] = &[b"apex:page", b"apex:component"];
/// Match `.page` and `.component` files, excluding their `-meta.xml` companions.
fn matches(rel: &str) -> bool {
    let file = rel.rsplit('/').next().unwrap_or(rel);
    if file.ends_with("-meta.xml") {
        return false;
    }
    let Some((stem, ext)) = file.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty() && matches!(ext.to_ascii_lowercase().as_str(), "page" | "component")
}

fn tag_name<'s>(node: Node<'_>, src: &'s [u8]) -> Option<&'s [u8]> {
    let tag = node.named_child(0)?;
    let name = tag.named_child(0)?;
    (name.kind() == "tag_name").then(|| &src[name.byte_range()])
}

/// Count named CST leaves, stopping as soon as the threshold is exceeded.
fn exceeds(node: Node<'_>, threshold: usize) -> bool {
    let mut count = 0;
    let mut cursor = node.walk();
    loop {
        let n = cursor.node();
        if n.is_named() && n.named_child_count() == 0 {
            count += 1;
            if count > threshold {
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

fn is_page_root(node: Node<'_>, src: &[u8]) -> bool {
    node.kind() == "element" && tag_name(node, src).is_some_and(|name| PAGE_ROOTS.contains(&name))
}

fn unit<'t>(_spec: &LangSpec, node: Node<'t>, src: &[u8]) -> Option<UnitShape<'t>> {
    if node.kind() != "element" {
        return None;
    }
    let name = tag_name(node, src)?;
    if PAGE_ROOTS.contains(&name) {
        return Some(UnitShape {
            kind: UnitKind::Page,
            body: node,
            leading: Vec::new(),
        });
    }
    // A MarkupBlock is any nested element over 200 leaves reached through a chain of
    // oversized (>2,000-leaf) ancestors up to the page root. Oversized blocks therefore
    // split into their child elements (D44) rather than staying one unwieldy unit.
    if !exceeds(node, 200) {
        return None;
    }
    let mut parent = node.parent()?;
    loop {
        if is_page_root(parent, src) {
            break;
        }
        if parent.kind() != "element" || !exceeds(parent, 2_000) {
            return None;
        }
        parent = parent.parent()?;
    }
    Some(UnitShape {
        kind: UnitKind::MarkupBlock,
        body: node,
        leading: Vec::new(),
    })
}

pub static SPEC: LangSpec = LangSpec {
    lang: Lang::Vf,
    language: || tree_sitter_html::LANGUAGE.into(),
    preprocess: super::identity_preprocess,
    matches,
    unit,
    feature_config: super::identity_features,
    term_rule: aroma_core::lexical::TermRule::Markup,
    omission_marker: aroma_core::render::OMITTED_MARKUP,
    repair: crate::snippet::Repair::Tags,
    wrappers: &WRAPPERS,
    leaf_kinds: &[
        "tag_name",
        "attribute_name",
        "attribute_value",
        "text",
        "raw_text",
        "identifier",
        "literal",
    ],
    atomic_kinds: &["attribute_value", "quoted_attribute_value"],
    comment_kinds: &["comment"],
    identifier_kind: "identifier",
    units: &[("element", UnitKind::Page)],
    body_field: "",
    markup_bindings: &[("var", MarkupBinding::Decl)],
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
    adapter_version: "vf-1",
    grammar_version: "tree-sitter-html 0.23.2",
    normalizer: &crate::normalize::vf::NORMALIZER,
    is_test_path: |path| path.to_string_lossy().to_ascii_lowercase().contains("test"),
};

/// Statements/member wrap a markup fragment; expression wraps a bare `{!…}` body. The
/// markup snippet path removes these synthetic elements.
const WRAPPERS: [Wrapper; 3] = [
    Wrapper {
        name: "statements",
        shape: Shape::Statements,
        prefix: "<apex:page>\n",
        suffix: "\n</apex:page>",
    },
    Wrapper {
        name: "member",
        shape: Shape::Member,
        prefix: "<apex:page>\n",
        suffix: "\n</apex:page>",
    },
    Wrapper {
        name: "expression",
        shape: Shape::Expression,
        prefix: "<apex:page>{!",
        suffix: "}</apex:page>",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_only_page_and_component_files() {
        assert!(matches("force-app/main/default/pages/Accounts.page"));
        assert!(matches("src/pages/MyComp.component"));
        assert!(matches("pages/ACCOUNTS.PAGE"));
        assert!(!matches("pages/Accounts.page-meta.xml"));
        assert!(!matches("pages/Accounts.component-meta.xml"));
        assert!(!matches("pages/Accounts.cls"));
        assert!(!matches("pages/Accounts.page.xml"));
        assert!(!matches(""));
    }
}
