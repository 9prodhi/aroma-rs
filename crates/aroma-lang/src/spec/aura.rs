//! Aura component markup, tree-sitter-xml 0.7.0 (D35/D44/D46).
//!
//! A `.cmp` / `.app` / `.evt` / `.intf` file under `…/aura/<bundle>/` has a single root
//! `aura:component` / `aura:application` / `aura:event` / `aura:interface`; a `.design`
//! file has a `design:component` root. That root is one `Component` unit; a nested
//! element whose subtree exceeds 200 named leaves (notably `aura:if`, `aura:iteration`,
//! `aura:set`) becomes a `MarkupBlock` unit, with a D44 recursive split above 2,000
//! leaves into its child elements. Entities are opaque (never expanded) and attribute
//! values are atomic, so `{!…}` / `{#…}` expressions are tokenized by the shared lexer.
use super::{LangSpec, MarkupBinding};
use crate::snippet::{Repair, Shape, Wrapper};
use crate::table::UnitShape;
use aroma_core::lang::Lang;
use aroma_core::unit::UnitKind;
use tree_sitter::Node;

/// Root elements that are the `Component` unit. `design:component` only appears in
/// `.design` files; the others cover `.cmp`, `.app`, `.evt` and `.intf`.
const AURA_ROOTS: &[&[u8]] = &[
    b"aura:component",
    b"aura:application",
    b"aura:event",
    b"aura:interface",
    b"design:component",
];

/// Match files under `…/aura/<bundle>/` with an Aura markup extension, never their
/// `-meta.xml` companions and never the non-markup members of a bundle.
fn matches(rel: &str) -> bool {
    let file = rel.rsplit('/').next().unwrap_or(rel);
    if file.ends_with("-meta.xml") {
        return false;
    }
    let Some((stem, ext)) = file.rsplit_once('.') else {
        return false;
    };
    if stem.is_empty()
        || !matches!(
            ext.to_ascii_lowercase().as_str(),
            "cmp" | "app" | "evt" | "intf" | "design" | "tokens"
        )
    {
        return false;
    }
    // The path must be `…/aura/<bundle>/<file>`: the file's parent is the bundle and
    // the directory above it is exactly `aura` (ASCII case-insensitive).
    let mut parts = rel.split('/');
    let _file = parts.next_back();
    let bundle = parts.next_back();
    let aura = parts.next_back();
    bundle.is_some() && aura.is_some_and(|d| d.eq_ignore_ascii_case("aura"))
}

fn tag_name<'s>(node: Node<'_>, src: &'s [u8]) -> Option<&'s [u8]> {
    if node.kind() != "element" {
        return None;
    }
    let tag = node.named_child(0)?;
    let name = tag.named_child(0)?;
    (name.kind() == "Name").then(|| &src[name.byte_range()])
}

/// The element immediately containing `node` (XML wraps element children in `content`).
fn parent_element(node: Node<'_>) -> Option<Node<'_>> {
    let parent = node.parent()?;
    if parent.kind() == "content" {
        parent.parent()
    } else {
        None
    }
}

/// Count named CST leaves (excluding comments and whitespace CharData), stopping as soon
/// as the threshold is exceeded. Formatting whitespace cannot cause recursive splitting.
fn exceeds(node: Node<'_>, src: &[u8], threshold: usize) -> bool {
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

fn is_aura_root(node: Node<'_>, src: &[u8]) -> bool {
    node.kind() == "element" && tag_name(node, src).is_some_and(|name| AURA_ROOTS.contains(&name))
}

fn unit<'t>(_spec: &LangSpec, node: Node<'t>, src: &[u8]) -> Option<UnitShape<'t>> {
    if node.kind() != "element" {
        return None;
    }
    let name = tag_name(node, src)?;
    if AURA_ROOTS.contains(&name) {
        return Some(UnitShape {
            kind: UnitKind::Component,
            body: node,
            leading: Vec::new(),
        });
    }
    // A MarkupBlock is any nested element over 200 leaves reached through a chain of
    // oversized (>2,000-leaf) ancestors up to the root. Oversized blocks therefore
    // split into their child elements (D44) rather than staying one unwieldy unit.
    if !exceeds(node, src, 200) {
        return None;
    }
    let mut parent = parent_element(node)?;
    loop {
        if is_aura_root(parent, src) {
            break;
        }
        if parent.kind() != "element" || !exceeds(parent, src, 2_000) {
            return None;
        }
        parent = parent_element(parent)?;
    }
    Some(UnitShape {
        kind: UnitKind::MarkupBlock,
        body: node,
        leading: Vec::new(),
    })
}

pub static SPEC: LangSpec = LangSpec {
    lang: Lang::Aura,
    language: || tree_sitter_xml::LANGUAGE_XML.into(),
    preprocess: super::identity_preprocess,
    matches,
    unit,
    feature_config: super::identity_features,
    term_rule: aroma_core::lexical::TermRule::Markup,
    omission_marker: aroma_core::render::OMITTED_MARKUP,
    repair: Repair::Tags,
    wrappers: &WRAPPERS,
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
    units: &[("element", UnitKind::Component)],
    body_field: "",
    markup_bindings: &[
        ("var", MarkupBinding::Decl),
        ("indexVar", MarkupBinding::Decl),
    ],
    js_scope_kinds: &[],
    function_scope_kinds: &[],
    js_snippets: None,
    decl_name_parents: &[],
    non_var_parents: &[],
    non_var_fields: &[],
    type_decl_kinds: &[],
    builtin_globals: &["$A"],
    global_suffixes: &[],
    fold_case: false,
    adapter_version: "aura-1",
    grammar_version: "tree-sitter-xml 0.7.0",
    normalizer: &crate::normalize::aura::NORMALIZER,
    is_test_path: |path| path.to_string_lossy().to_ascii_lowercase().contains("test"),
};

/// Statements/member wrap a markup fragment; expression wraps a bare `{!…}` body. The
/// markup snippet path removes these synthetic elements.
const WRAPPERS: [Wrapper; 3] = [
    Wrapper {
        name: "statements",
        shape: Shape::Statements,
        prefix: "<aura:component>\n",
        suffix: "\n</aura:component>",
    },
    Wrapper {
        name: "member",
        shape: Shape::Member,
        prefix: "<aura:component>\n",
        suffix: "\n</aura:component>",
    },
    Wrapper {
        name: "expression",
        shape: Shape::Expression,
        prefix: "<aura:component>{!",
        suffix: "}</aura:component>",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_only_aura_markup_files_under_a_bundle() {
        assert!(matches("force-app/main/default/aura/myBundle/myBundle.cmp"));
        assert!(matches("src/main/aura/app/app.app"));
        assert!(matches("aura/evt/evt.evt"));
        assert!(matches("aura/intf/intf.intf"));
        assert!(matches("aura/design/design.design"));
        assert!(matches("aura/tokens/tokens.tokens"));
        assert!(matches("aura/X/X.CMP"));
        assert!(!matches("aura/x/x.cmp-meta.xml"));
        assert!(!matches("aura/x/x.app-meta.xml"));
        assert!(!matches("aura/x/x.js"));
        assert!(!matches("aura/x/x.css"));
        assert!(!matches("aura/x/x.svg"));
        assert!(!matches("aura/x/x.auradoc"));
        assert!(!matches("pages/Account.page"));
        assert!(!matches("aura/myBundle/../other.cmp"));
        assert!(!matches(""));
    }
}
