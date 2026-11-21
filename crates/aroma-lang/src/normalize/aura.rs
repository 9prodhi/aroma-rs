//! Aura component markup normalization: keyword tag/attribute names, atomic attribute
//! values and text with `{!…}` / `{#…}` expression tokenization, and loop declarations
//! carried by `aura:iteration var` / `indexVar`. XML `content` containers are spliced,
//! whitespace-only CharData is dropped, and entities stay opaque. Names/values retain
//! their source order exactly (positional features, like VF).
use super::expr::{find_expressions, parse_expression, Dialect};
use super::Normalizer;
use crate::kraw::KRaw;
use crate::spec::MarkupBinding;

/// The binding table declared by the Aura spec (`var` / `indexVar` on `aura:iteration`).
const BINDINGS: &[(&str, MarkupBinding)] = crate::spec::aura::SPEC.markup_bindings;

pub struct AuraNormalizer;
pub static NORMALIZER: AuraNormalizer = AuraNormalizer;

struct Frame {
    kind: &'static str,
    field: Option<&'static str>,
    pending: std::vec::IntoIter<KRaw>,
    out: Vec<KRaw>,
}

fn node(kind: &'static str, children: Vec<KRaw>) -> KRaw {
    KRaw::Node {
        kind,
        field: None,
        children,
    }
}

fn token_text(tree: &KRaw) -> Option<&str> {
    if let KRaw::Token { text, .. } = tree {
        Some(text)
    } else {
        None
    }
}

/// Split only at `{!…}` / `{#…}` expression boundaries; preserve literal spelling and
/// spans. AttValue's whole text is atomic, so its hidden value text arrives in one token.
fn value(token: KRaw) -> KRaw {
    let KRaw::Token {
        kind,
        text,
        bytes,
        line,
        ..
    } = token
    else {
        return token;
    };
    let quoted = kind == "AttValue";
    let expressions = find_expressions(&text, Dialect::Aura);
    if quoted && expressions.is_empty() {
        return KRaw::Token {
            kind: "literal",
            named: true,
            text,
            bytes,
            line,
            field: None,
        };
    }
    let mut children = Vec::new();
    let mut previous = 0;
    let mut current_line = line;
    for (start, end, inner) in expressions {
        literal(
            &text,
            previous,
            start,
            bytes.start,
            current_line,
            &mut children,
        );
        let offset = start + 2;
        let expression_line = current_line
            + text[previous..offset]
                .bytes()
                .filter(|&b| b == b'\n')
                .count() as u32;
        let mut expression = parse_expression(
            inner,
            Dialect::Aura,
            bytes.start + offset as u32,
            expression_line,
        );
        if let KRaw::Node {
            kind: "path",
            children,
            ..
        } = &mut expression
        {
            for child in children.iter_mut().skip(1) {
                if let KRaw::Token {
                    kind: "identifier",
                    field,
                    ..
                } = child
                {
                    *field = Some("markup_property");
                }
            }
        }
        children.push(expression);
        current_line =
            expression_line + text[offset..end].bytes().filter(|&b| b == b'\n').count() as u32;
        previous = end;
    }
    literal(
        &text,
        previous,
        text.len(),
        bytes.start,
        current_line,
        &mut children,
    );
    // Quotes enclosing a whole expression are delimiters rather than literal text.
    if quoted && children.len() >= 3 {
        if matches!(children.first().and_then(token_text), Some("\"" | "'")) {
            children.remove(0);
        }
        if matches!(children.last().and_then(token_text), Some("\"" | "'")) {
            children.pop();
        }
    }
    node("value", children)
}

fn literal(text: &str, start: usize, end: usize, base: u32, line: u32, out: &mut Vec<KRaw>) {
    let part = &text[start..end];
    let trimmed = part.trim();
    if trimmed.is_empty() {
        return;
    }
    let trimmed_prefix = part.len() - part.trim_start().len();
    let start = start + trimmed_prefix;
    out.push(KRaw::Token {
        kind: "literal",
        named: true,
        text: trimmed.to_owned(),
        bytes: base + start as u32..base + (start + trimmed.len()) as u32,
        line: line
            + part[..trimmed_prefix]
                .bytes()
                .filter(|&b| b == b'\n')
                .count() as u32,
        field: None,
    });
}

/// `Name` (tag or attribute name) is a keyword; `AttValue`, `CharData` and `CData` are
/// literal/expression values. `EntityRef` / `CharRef` are atomic and never expanded.
fn rewrite_token(mut token: KRaw) -> KRaw {
    match token.kind() {
        "Name" => {
            if let KRaw::Token { named, .. } = &mut token {
                *named = false;
            }
            token
        }
        "AttValue" | "CharData" | "CData" => value(token),
        _ => token,
    }
}

/// A loop declaration's spelling and source position come from its own attribute.
/// The classifier opens a scope per element, so declarations cannot escape siblings.
fn attribute(mut children: Vec<KRaw>) -> Vec<KRaw> {
    let name = children
        .first()
        .and_then(token_text)
        .map(str::to_owned)
        .unwrap_or_default();
    for &(attr, binding) in BINDINGS {
        match binding {
            MarkupBinding::Decl if name == attr => {
                if let Some(KRaw::Token {
                    kind,
                    named,
                    text,
                    bytes,
                    field,
                    ..
                }) = children.get_mut(2)
                {
                    let trimmed = text.trim_matches(['\'', '"']);
                    let offset = text.len() - text.trim_start_matches(['\'', '"']).len();
                    *bytes =
                        bytes.start + offset as u32..bytes.start + (offset + trimmed.len()) as u32;
                    *text = trimmed.to_owned();
                    *kind = "identifier";
                    *named = true;
                    *field = Some("markup_decl");
                }
            }
            _ => {}
        }
    }
    children
}

impl Normalizer for AuraNormalizer {
    fn normalize(&self, root: KRaw) -> KRaw {
        let KRaw::Node {
            kind,
            field,
            children,
        } = root
        else {
            return rewrite_token(root);
        };
        let mut stack = vec![Frame {
            kind,
            field,
            pending: children.into_iter(),
            out: Vec::new(),
        }];
        loop {
            match stack.last_mut().expect("root remains").pending.next() {
                Some(KRaw::Node {
                    kind,
                    field,
                    children,
                }) => stack.push(Frame {
                    kind,
                    field,
                    pending: children.into_iter(),
                    out: Vec::new(),
                }),
                Some(token) => stack
                    .last_mut()
                    .expect("non-empty")
                    .out
                    .push(rewrite_token(token)),
                None => {
                    let frame = stack.pop().expect("non-empty");
                    let children = if frame.kind == "Attribute" {
                        attribute(frame.out)
                    } else {
                        frame.out
                    };
                    let done = KRaw::Node {
                        kind: frame.kind,
                        field: frame.field,
                        children,
                    };
                    match stack.last_mut() {
                        Some(parent) if frame.kind == "content" => {
                            // XML wraps element children in `content`; splice it so the
                            // element's own children sit directly under the element.
                            if let KRaw::Node { children, .. } = done {
                                parent.out.extend(children);
                            }
                        }
                        Some(parent) => parent.out.push(done),
                        None => return done,
                    }
                }
            }
        }
    }
}
