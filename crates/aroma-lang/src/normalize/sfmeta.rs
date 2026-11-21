//! XML metadata normalization. `content` and CDATA containers are spliced, blank
//! CharData is dropped, names are keywords and values retain their source order.
//! Structural mode sorts child elements, leaving text and expression order intact.
//! Formula text is joined across XML references/CDATA before expression parsing.
//! Only XML's predefined/numeric references are decoded for formulas; user-defined
//! entities stay opaque. A byte map preserves original source spans and lines.
use super::expr::{find_expressions, parse_expression, Dialect};
use super::Normalizer;
use crate::kraw::KRaw;

pub struct SfmetaNormalizer {
    structural: bool,
}
pub static NORMALIZER: SfmetaNormalizer = SfmetaNormalizer { structural: true };
pub static LEXICAL_NORMALIZER: SfmetaNormalizer = SfmetaNormalizer { structural: false };

fn token_text(tree: &KRaw) -> Vec<&str> {
    let mut out = Vec::new();
    let mut stack = vec![tree];
    while let Some(tree) = stack.pop() {
        match tree {
            KRaw::Node { children, .. } => stack.extend(children.iter().rev()),
            KRaw::Token { text, .. } => out.push(text.as_str()),
        }
    }
    out
}

fn normalized_tag(tree: &KRaw) -> Option<&str> {
    let KRaw::Node {
        kind: "element",
        children,
        ..
    } = tree
    else {
        return None;
    };
    // Structural element tags have already been spliced into the element.
    children.iter().find_map(|child| match child {
        KRaw::Token {
            kind: "Name", text, ..
        } => Some(text.as_str()),
        _ => None,
    })
}

fn child_key(tree: &KRaw) -> (String, Vec<String>, Vec<String>) {
    let KRaw::Node { children, .. } = tree else {
        unreachable!()
    };
    // Prefer fullName, then name, then apiName even if the input is reordered.
    let identity = ["fullName", "name", "apiName"]
        .iter()
        .find_map(|name| {
            children
                .iter()
                .filter(|child| {
                    normalized_tag(child)
                        .is_some_and(|tag| crate::spec::sfmeta::local_name(tag) == *name)
                })
                .map(|child| {
                    let KRaw::Node { children, .. } = child else {
                        unreachable!()
                    };
                    children
                        .iter()
                        .filter(|part| !matches!(part, KRaw::Token { named: false, .. }))
                        .flat_map(token_text)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .min()
        })
        .unwrap_or_default();
    (
        normalized_tag(tree).unwrap_or_default().to_owned(),
        identity,
        token_text(tree).into_iter().map(str::to_owned).collect(),
    )
}

fn sort_elements(children: &mut Vec<KRaw>) {
    // Sort only element slots: mixed text, attributes and formula operands retain
    // their positions. Token vectors frame boundaries without separator collisions.
    let mut elements = Vec::new();
    let slots: Vec<_> = std::mem::take(children)
        .into_iter()
        .map(|child| {
            if child.kind() == "element" {
                elements.push((child_key(&child), child));
                None
            } else {
                Some(child)
            }
        })
        .collect();
    elements.sort_by(|a, b| a.0.cmp(&b.0));
    let mut sorted = elements.into_iter();
    children.extend(
        slots
            .into_iter()
            .map(|child| child.unwrap_or_else(|| sorted.next().expect("same element count").1)),
    );
}

#[derive(Clone, Copy)]
enum Role {
    Literal,
    Reference,
    Declaration,
}

fn node(kind: &'static str, children: Vec<KRaw>) -> KRaw {
    KRaw::Node {
        kind,
        field: None,
        children,
    }
}

fn raw_tag(children: &[KRaw]) -> Option<&str> {
    let KRaw::Node {
        children,
        kind: "STag" | "EmptyElemTag",
        ..
    } = children.first()?
    else {
        return None;
    };
    children.iter().find_map(|k| match k {
        KRaw::Token {
            kind: "Name", text, ..
        } => Some(text.as_str()),
        _ => None,
    })
}

pub(crate) fn field_api(text: &str) -> bool {
    text.contains('.') || text.contains("__") || text.starts_with('$')
}

fn symbol(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
        && !field_api(text)
}

/// Dotted formula paths name Salesforce fields, never local variables.
fn mark_expression(tree: &mut KRaw) {
    let mut stack = vec![(tree, false)];
    while let Some((tree, path)) = stack.pop() {
        match tree {
            KRaw::Node { kind, children, .. } => {
                let path = path || *kind == "path";
                stack.extend(children.iter_mut().map(|c| (c, path)));
            }
            KRaw::Token {
                kind: "identifier",
                text,
                field,
                ..
            } if path || field_api(text) => *field = Some("sfmeta_field"),
            _ => {}
        }
    }
}

fn value_part(
    text: &str,
    start: usize,
    end: usize,
    base: u32,
    line: u32,
    role: Role,
    out: &mut Vec<KRaw>,
) {
    let part = &text[start..end];
    let trimmed = part.trim();
    if trimmed.is_empty() {
        return;
    }
    let offset = start + part.len() - part.trim_start().len();
    let identifier = !matches!(role, Role::Literal) && symbol(trimmed);
    out.push(KRaw::Token {
        kind: if identifier { "identifier" } else { "literal" },
        named: true,
        text: trimmed.to_owned(),
        bytes: base + offset as u32..base + (offset + trimmed.len()) as u32,
        line: line + text[..offset].bytes().filter(|&b| b == b'\n').count() as u32,
        field: if identifier && matches!(role, Role::Declaration) {
            Some("sfmeta_decl")
        } else {
            None
        },
    });
}

fn value(token: KRaw, role: Role) -> KRaw {
    let KRaw::Token {
        text, bytes, line, ..
    } = token
    else {
        return token;
    };
    let mut out = Vec::new();
    let mut previous = 0;
    for (start, end, inner) in find_expressions(&text, Dialect::Formula) {
        value_part(&text, previous, start, bytes.start, line, role, &mut out);
        let offset = start + 2;
        let mut expr = parse_expression(
            inner,
            Dialect::Formula,
            bytes.start + offset as u32,
            line + text[..offset].bytes().filter(|&b| b == b'\n').count() as u32,
        );
        mark_expression(&mut expr);
        out.push(expr);
        previous = end;
    }
    value_part(
        &text,
        previous,
        text.len(),
        bytes.start,
        line,
        role,
        &mut out,
    );
    node("value", out)
}

fn predefined(text: &str) -> Option<char> {
    match text {
        "&lt;" => Some('<'),
        "&gt;" => Some('>'),
        "&amp;" => Some('&'),
        "&apos;" => Some('\''),
        "&quot;" => Some('"'),
        _ => {
            let number = text.strip_prefix("&#")?.strip_suffix(';')?;
            let value = if let Some(hex) = number.strip_prefix('x') {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                number.parse().ok()?
            };
            char::from_u32(value)
        }
    }
}

fn formula(content: &KRaw) -> KRaw {
    let mut text = String::new();
    // One original span and source line for every byte of the decoded expression.
    let mut positions: Vec<(u32, u32, u32)> = Vec::new();
    let mut stack = vec![content];
    while let Some(tree) = stack.pop() {
        match tree {
            KRaw::Node { children, .. } => stack.extend(children.iter().rev()),
            KRaw::Token {
                kind,
                text: part,
                bytes,
                line,
                ..
            } if matches!(*kind, "CharData" | "CData" | "EntityRef" | "CharRef") => {
                if matches!(*kind, "EntityRef" | "CharRef") {
                    if let Some(ch) = predefined(part) {
                        text.push(ch);
                        positions.extend(std::iter::repeat_n(
                            (bytes.start, bytes.end, *line),
                            ch.len_utf8(),
                        ));
                        continue;
                    }
                }
                text.push_str(part);
                let mut current_line = *line;
                for (offset, b) in part.bytes().enumerate() {
                    positions.push((
                        bytes.start + offset as u32,
                        bytes.start + offset as u32 + 1,
                        current_line,
                    ));
                    current_line += u32::from(b == b'\n');
                }
            }
            _ => {}
        }
    }
    // Merge fields within formulas have expression semantics too; remove only the
    // delimiters, retaining the byte mapping for the inner text.
    let spans = find_expressions(&text, Dialect::Formula);
    let mut decoded = String::new();
    let mut mapped = Vec::new();
    let mut previous = 0;
    for (start, end, _) in spans {
        decoded.push_str(&text[previous..start]);
        mapped.extend_from_slice(&positions[previous..start]);
        decoded.push_str(&text[start + 2..end - 1]);
        mapped.extend_from_slice(&positions[start + 2..end - 1]);
        previous = end;
    }
    decoded.push_str(&text[previous..]);
    mapped.extend_from_slice(&positions[previous..]);
    let mut expression = parse_expression(&decoded, Dialect::Formula, 0, 1);
    let mut stack = vec![&mut expression];
    while let Some(tree) = stack.pop() {
        match tree {
            KRaw::Node { children, .. } => stack.extend(children.iter_mut()),
            KRaw::Token { bytes, line, .. } => {
                if let Some(&(start, _, first_line)) = mapped.get(bytes.start as usize) {
                    let end = mapped
                        .get(bytes.end.saturating_sub(1) as usize)
                        .map_or(start, |p| p.1);
                    *bytes = start..end;
                    *line = first_line;
                } else if let Some(&(_, end, last_line)) = mapped.last() {
                    *bytes = end..end;
                    *line = last_line;
                }
            }
        }
    }
    mark_expression(&mut expression);
    expression
}

struct Frame {
    kind: &'static str,
    field: Option<&'static str>,
    tag: Option<String>,
    role: Role,
    pending: std::vec::IntoIter<KRaw>,
    out: Vec<KRaw>,
}

impl Frame {
    fn new(
        kind: &'static str,
        field: Option<&'static str>,
        mut children: Vec<KRaw>,
        parent_tag: Option<&str>,
        inherited: Role,
    ) -> Self {
        let tag = (kind == "element")
            .then(|| {
                raw_tag(&children)
                    .map(crate::spec::sfmeta::local_name)
                    .map(str::to_owned)
            })
            .flatten();
        let role = match tag.as_deref() {
            Some("name") if matches!(parent_tag, Some("variables" | "formulas")) => {
                Role::Declaration
            }
            Some(
                "leftValueReference"
                | "elementReference"
                | "assignToReference"
                | "inputReference"
                | "collectionReference",
            ) => Role::Reference,
            Some(_) => Role::Literal,
            None => inherited,
        };
        if kind == "element"
            && matches!(
                tag.as_deref(),
                Some("formula" | "errorConditionFormula" | "expression")
            )
        {
            for child in &mut children {
                if child.kind() == "content" {
                    *child = formula(child);
                }
            }
        }
        Self {
            kind,
            field,
            tag: tag.or_else(|| parent_tag.map(str::to_owned)),
            role,
            pending: children.into_iter(),
            out: Vec::new(),
        }
    }
}

impl Normalizer for SfmetaNormalizer {
    fn normalize(&self, root: KRaw) -> KRaw {
        let KRaw::Node {
            kind,
            field,
            children,
        } = root
        else {
            return value(root, Role::Literal);
        };
        let mut stack = vec![Frame::new(kind, field, children, None, Role::Literal)];
        loop {
            let top = stack.last_mut().expect("root remains");
            match top.pending.next() {
                Some(KRaw::Node {
                    kind,
                    field,
                    children,
                }) => {
                    let frame = Frame::new(kind, field, children, top.tag.as_deref(), top.role);
                    stack.push(frame);
                }
                Some(mut token) => match token.kind() {
                    "Name" => {
                        if let KRaw::Token { named, .. } = &mut token {
                            *named = false;
                        }
                        top.out.push(token);
                    }
                    "CharData" | "CData" | "AttValue" | "EntityRef" | "CharRef" => {
                        let rewritten = value(token, top.role);
                        if let KRaw::Node { children, .. } = rewritten {
                            top.out.extend(children);
                        }
                    }
                    "CDStart" | "]]>" => {}
                    _ => top.out.push(token),
                },
                None => {
                    let mut done = stack.pop().expect("nonempty");
                    if self.structural && done.kind == "element" {
                        sort_elements(&mut done.out);
                    }
                    let splice = matches!(done.kind, "content" | "CDSect")
                        || (self.structural
                            && matches!(done.kind, "STag" | "ETag" | "EmptyElemTag"));
                    let tree = KRaw::Node {
                        kind: done.kind,
                        field: done.field,
                        children: done.out,
                    };
                    match stack.last_mut() {
                        Some(parent) if splice => {
                            if let KRaw::Node { children, .. } = tree {
                                parent.out.extend(children);
                            }
                        }
                        Some(parent) => parent.out.push(tree),
                        None => return tree,
                    }
                }
            }
        }
    }
}
