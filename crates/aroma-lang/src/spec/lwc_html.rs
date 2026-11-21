//! LWC HTML templates, tree-sitter-html 0.23.2 (D35/D44/D47).
use super::{LangSpec, MarkupBinding};
use crate::snippet::{Shape, Wrapper};
use crate::table::UnitShape;
use aroma_core::lang::Lang;
use aroma_core::unit::UnitKind;
use std::borrow::Cow;
use tree_sitter::Node;

/// tree-sitter-html rejects underscores in tag names. Rewrite only those bytes,
/// skipping comments and whole tag attributes; token text always uses the original.
pub fn preprocess(source: &str) -> Cow<'_, str> {
    let bytes = source.as_bytes();
    let mut rewritten: Option<Vec<u8>> = None;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"<!--") {
            let Some(end) = source[i + 4..].find("-->") else {
                break;
            };
            i += 4 + end + 3;
            continue;
        }
        if bytes[i..].starts_with(b"<![CDATA[") {
            let Some(end) = source[i + 9..].find("]]>") else {
                break;
            };
            i += 9 + end + 3;
            continue;
        }
        if bytes[i..].starts_with(b"<!") || bytes[i..].starts_with(b"<?") {
            // Declarations can contain quoted pseudo-tags and internal subsets.
            let mut quote = None;
            let mut brackets = 0usize;
            i += 2;
            while i < bytes.len() {
                let b = bytes[i];
                i += 1;
                if let Some(q) = quote {
                    if b == q {
                        quote = None;
                    }
                    continue;
                }
                match b {
                    b'\'' | b'"' => quote = Some(b),
                    b'[' => brackets += 1,
                    b']' => brackets = brackets.saturating_sub(1),
                    b'>' if brackets == 0 => break,
                    _ => {}
                }
            }
            continue;
        }
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let closing = bytes.get(i + 1) == Some(&b'/');
        let name_start = i + 1 + usize::from(closing);
        let mut name = name_start;
        if !bytes
            .get(name)
            .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        {
            i += 1;
            continue;
        }
        while bytes
            .get(name)
            .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.'))
        {
            if bytes[name] == b'_' {
                rewritten.get_or_insert_with(|| bytes.to_vec())[name] = b'-';
            }
            name += 1;
        }
        i = name;
        let mut quote = None;
        let mut braces = 0usize;
        while i < bytes.len() {
            let b = bytes[i];
            i += 1;
            if let Some(q) = quote {
                if b == q {
                    quote = None;
                }
                continue;
            }
            match b {
                b'\'' | b'"' => quote = Some(b),
                b'{' => braces += 1,
                b'}' => braces = braces.saturating_sub(1),
                b'>' if braces == 0 => break,
                _ => {}
            }
        }
        let raw_name = &bytes[name_start..name];
        let raw_text = [
            "script", "style", "textarea", "title", "xmp", "iframe", "noembed", "noframes",
        ]
        .iter()
        .any(|name| raw_name.eq_ignore_ascii_case(name.as_bytes()));
        if !closing && raw_text {
            // In raw-text/RCDATA content, a '<name' is text until the matching
            // end tag, even inside a JS or CSS string and even without a '>'.
            while i < bytes.len() {
                if bytes[i..].starts_with(b"</")
                    && bytes
                        .get(i + 2..i + 2 + raw_name.len())
                        .is_some_and(|name| name.eq_ignore_ascii_case(raw_name))
                    && bytes
                        .get(i + 2 + raw_name.len())
                        .is_some_and(|b| b.is_ascii_whitespace() || matches!(b, b'>' | b'/'))
                {
                    break;
                }
                i += 1;
            }
        }
    }
    rewritten.map_or(Cow::Borrowed(source), |bytes| {
        Cow::Owned(String::from_utf8(bytes).expect("ASCII replacements preserve UTF-8"))
    })
}

fn matches(rel: &str) -> bool {
    let parts: Vec<_> = rel.split('/').collect();
    parts.len() >= 3
        && !parts.contains(&"__tests__")
        && parts[parts.len() - 3] == "lwc"
        && !parts[parts.len() - 2].is_empty()
        && super::has_extension(rel, &["html"])
}

fn tag_name<'s>(node: Node<'_>, src: &'s [u8]) -> Option<&'s [u8]> {
    let tag = node.named_child(0)?;
    let name = tag.named_child(0)?;
    (name.kind() == "tag_name").then(|| &src[name.byte_range()])
}

fn template(node: Node<'_>, src: &[u8]) -> bool {
    node.kind() == "element" && tag_name(node, src) == Some(b"template")
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

fn unit<'t>(_spec: &LangSpec, node: Node<'t>, src: &[u8]) -> Option<UnitShape<'t>> {
    let kind = if template(node, src) {
        UnitKind::Template
    } else {
        if node.kind() != "element" || !exceeds(node, 200) {
            return None;
        }
        let mut parent = node.parent()?;
        loop {
            if template(parent, src) {
                break;
            }
            if parent.kind() != "element" || !exceeds(parent, 2_000) {
                return None;
            }
            parent = parent.parent()?;
        }
        UnitKind::MarkupBlock
    };
    Some(UnitShape {
        kind,
        body: node,
        leading: Vec::new(),
    })
}

pub static SPEC: LangSpec = LangSpec {
    lang: Lang::LwcHtml,
    language: || tree_sitter_html::LANGUAGE.into(),
    preprocess,
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
        "identifier",
        "literal",
    ],
    atomic_kinds: &["attribute_value", "quoted_attribute_value"],
    comment_kinds: &["comment"],
    identifier_kind: "identifier",
    units: &[("element", UnitKind::Template)],
    body_field: "",
    markup_bindings: &[
        ("for:item", MarkupBinding::Decl),
        ("for:index", MarkupBinding::Decl),
        ("iterator:", MarkupBinding::DeclPrefix),
    ],
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
    adapter_version: "lwchtml-1",
    grammar_version: "tree-sitter-html 0.23.2",
    normalizer: &crate::normalize::lwc_html::NORMALIZER,
    is_test_path: |path| path.components().any(|c| c.as_os_str() == "__tests__"),
};

/// Statements/member wrap markup fragments; expression wraps a bare LWC property
/// path in braces. The markup snippet path removes these synthetic elements.
const WRAPPERS: [Wrapper; 3] = [
    Wrapper {
        name: "statements",
        shape: Shape::Statements,
        prefix: "<template>\n",
        suffix: "\n</template>",
    },
    Wrapper {
        name: "member",
        shape: Shape::Member,
        prefix: "<template>\n",
        suffix: "\n</template>",
    },
    Wrapper {
        name: "expression",
        shape: Shape::Expression,
        prefix: "<template>{",
        suffix: "}</template>",
    },
];
