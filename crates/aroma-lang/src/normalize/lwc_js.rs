//! Context-free, bottom-up JavaScript rewrites. Decorator names are keywords;
//! shorthand properties are expression identifiers and binding patterns get `name`
//! anchors for the shared classifier. `this`/`?.` and template fragments are handled
//! by the spec's token tables, preserving all source text and byte spans.
use super::Normalizer;
use crate::kraw::KRaw;

pub struct LwcJsNormalizer;
pub static NORMALIZER: LwcJsNormalizer = LwcJsNormalizer;

struct Frame {
    kind: &'static str,
    field: Option<&'static str>,
    pending: std::vec::IntoIter<KRaw>,
    children: Vec<KRaw>,
}

impl Normalizer for LwcJsNormalizer {
    fn normalize(&self, root: KRaw) -> KRaw {
        let mut stack = Vec::<Frame>::new();
        let mut next = Some(root);
        loop {
            if let Some(node) = next.take() {
                match node {
                    KRaw::Node {
                        kind,
                        field,
                        children,
                    } => stack.push(Frame {
                        kind,
                        field,
                        pending: children.into_iter(),
                        children: Vec::new(),
                    }),
                    token => {
                        let token = rewrite_token(token);
                        if let Some(parent) = stack.last_mut() {
                            parent.children.push(token);
                        } else {
                            return token;
                        }
                    }
                }
            }
            let frame = stack.last_mut().expect("open root");
            if let Some(child) = frame.pending.next() {
                next = Some(child);
                continue;
            }
            let frame = stack.pop().expect("open root");
            let node = rewrite_node(frame.kind, frame.field, frame.children);
            if let Some(parent) = stack.last_mut() {
                parent.children.push(node);
            } else {
                return node;
            }
        }
    }
}

fn rewrite_token(token: KRaw) -> KRaw {
    match token {
        KRaw::Token {
            kind: "shorthand_property_identifier" | "shorthand_property_identifier_pattern",
            named,
            text,
            bytes,
            line,
            field,
        } => KRaw::Token {
            kind: "identifier",
            named,
            text,
            bytes,
            line,
            field,
        },
        other => other,
    }
}

fn keyword(token: KRaw) -> KRaw {
    match token {
        KRaw::Token {
            kind,
            text,
            bytes,
            line,
            field,
            ..
        } => KRaw::Token {
            kind,
            named: false,
            text,
            bytes,
            line,
            field,
        },
        other => other,
    }
}

fn rewrite_node(kind: &'static str, field: Option<&'static str>, children: Vec<KRaw>) -> KRaw {
    let mut out = Vec::with_capacity(children.len());
    for child in children {
        if kind == "decorator" {
            // Lift the decorator callee alongside `@` so labels contain `@wire#`.
            match child {
                KRaw::Node {
                    kind: "call_expression",
                    children,
                    ..
                } => {
                    out.extend(children.into_iter().map(|c| {
                        if c.field() == Some("function") {
                            keyword(c)
                        } else {
                            c
                        }
                    }));
                }
                t @ KRaw::Token {
                    kind: "identifier", ..
                } => out.push(keyword(t)),
                other => out.push(other),
            }
            continue;
        }
        let declaration = child.kind() == "identifier"
            && match kind {
                "formal_parameters" | "array_pattern" | "object_pattern" => true,
                "catch_clause" | "arrow_function" => child.field() == Some("parameter"),
                "pair_pattern" => child.field() == Some("value"),
                "assignment_pattern" | "object_assignment_pattern" => child.field() == Some("left"),
                "rest_pattern" => true,
                "for_in_statement" => {
                    child.field() == Some("left")
                        && out
                            .iter()
                            .any(|c: &KRaw| matches!(c.kind(), "const" | "let" | "var"))
                }
                _ => false,
            };
        out.push(if declaration {
            child.with_field(Some("name"))
        } else {
            child
        });
    }
    KRaw::Node {
        kind,
        field,
        children: out,
    }
}
