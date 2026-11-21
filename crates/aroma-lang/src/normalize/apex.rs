//! Apex normalizer (stage 2, D21) for tree-sitter-sfapex 3.0.2 `apex` trees. Apex has no
//! reference implementation to match, so the rewrites only make equivalent code look
//! the same (all bottom-up, explicit stack, deterministic and context-free):
//!
//! * **Order-insensitive SELECT.** The items of every `select_clause` (fields, dotted
//!   paths, aliases, functions, sub-queries, `TYPEOF`, `FIELDS(..)`) are sorted by their
//!   ASCII-folded token text (stable), and the original `,` tokens are re-interleaved, so
//!   `SELECT Name, Id` and `SELECT id, NAME` give the same labels, child slots and
//!   features. A clause containing an `#ERR` token is left alone.
//! * **Stable DML label.** `dml_expression` children are flattened to the DML verb token,
//!   the optional `as user|system` tokens, then target / upsert key / merge target, so the
//!   SPT label is always `insert#`, `upsert##`, `deleteasuser#`, … (case folding happens in
//!   stage 3).
//! * The `query_expression` subtree and every `bound_apex_expression` (`:expr`) are kept
//!   unchanged: SOQL names stay under `field_identifier` / `storage_identifier` (never
//!   `#VAR`) and bound identifiers keep an expression parent, so binds join the usage
//!   chains of their locals.
//! * **sObject field initializers.** In `new Account(Name = 'x')` the left identifier of
//!   each `assignment_expression` argument is a field name: it is wrapped in a
//!   single-child `field_identifier` anchor (never `#VAR`; collapse rule A removes the
//!   wrapper from the SPT).
//! * **Typed `when` bindings.** In `switch on rec { when Account a { … } }` the identifier
//!   of `when_sobject_type` gets field `name`, so stage 3 declares it (spec
//!   `decl_name_parents`) in the enclosing `switch_rule` scope.
//! * **Trigger bodies.** A `trigger_body` root holding one `block` is replaced by that
//!   block, so trigger units have the same `{…}` root as methods and as pasted snippets.
//!
//! Apart from those two anchors/fields, identifiers keep their enclosing node kind and
//! field, so stage-3 classification is otherwise exactly what it is without this pass.
use super::Normalizer;
use crate::kraw::KRaw;

pub struct ApexNormalizer;

pub static NORMALIZER: ApexNormalizer = ApexNormalizer;

impl Normalizer for ApexNormalizer {
    fn normalize(&self, node: KRaw) -> KRaw {
        match bottom_up(node) {
            KRaw::Node {
                kind: "trigger_body",
                mut children,
                ..
            } if children.len() == 1 && matches!(children[0], KRaw::Node { .. }) => {
                children.pop().expect("one child").with_field(None)
            }
            other => other,
        }
    }
}

/// One open node of the bottom-up walk.
struct Frame {
    kind: &'static str,
    field: Option<&'static str>,
    pending: std::vec::IntoIter<KRaw>,
    out: Vec<KRaw>,
}

/// Rewrites every node after its children (explicit stack). The root keeps its field.
fn bottom_up(root: KRaw) -> KRaw {
    let (kind, field, children) = match root {
        KRaw::Node {
            kind,
            field,
            children,
        } => (kind, field, children),
        token @ KRaw::Token { .. } => return token,
    };
    let mut stack = vec![Frame {
        kind,
        field,
        pending: children.into_iter(),
        out: Vec::new(),
    }];
    loop {
        let next = stack
            .last_mut()
            .expect("the root frame stays until it returns")
            .pending
            .next();
        match next {
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
            Some(token) => stack.last_mut().expect("non-empty").out.push(token),
            None => {
                let done = stack.pop().expect("non-empty");
                let children = match done.kind {
                    "select_clause" => sort_select(done.out),
                    "dml_expression" => flatten_dml(done.out),
                    "object_creation_expression" => sobject_initializers(done.out),
                    "when_sobject_type" => name_when_binding(done.out),
                    _ => done.out,
                };
                let node = KRaw::Node {
                    kind: done.kind,
                    field: done.field,
                    children,
                };
                match stack.last_mut() {
                    Some(parent) => parent.out.push(node),
                    None => return node,
                }
            }
        }
    }
}

fn is_tok(k: &KRaw, text: &str) -> bool {
    matches!(k, KRaw::Token { named: false, text: t, .. } if t.eq_ignore_ascii_case(text))
}

fn is_err(k: &KRaw) -> bool {
    matches!(k, KRaw::Token { kind: "ERROR", .. })
}

/// Sort key of one SELECT item: its token texts, ASCII-folded, joined by one space.
fn fold_key(item: &KRaw) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut stack = vec![item];
    while let Some(k) = stack.pop() {
        match k {
            KRaw::Token { text, .. } => parts.push(text.to_ascii_lowercase()),
            KRaw::Node { children, .. } => stack.extend(children.iter().rev()),
        }
    }
    parts.join(" ")
}

/// `SELECT a, b, c` with the items sorted by `fold_key` and the commas re-interleaved.
/// Anything but `SELECT item (',' item)*` (for instance a clause holding an `#ERR`
/// token) keeps its source order.
fn sort_select(children: Vec<KRaw>) -> Vec<KRaw> {
    let starts_with_select = children.first().is_some_and(|c| is_tok(c, "select"));
    let commas = children.iter().filter(|c| is_tok(c, ",")).count();
    let items = children.len() - commas - usize::from(starts_with_select);
    let alternating = children
        .iter()
        .skip(usize::from(starts_with_select))
        .enumerate()
        .all(|(i, c)| (i % 2 == 1) == is_tok(c, ","));
    if !starts_with_select || items != commas + 1 || !alternating || children.iter().any(is_err) {
        return children;
    }
    let mut it = children.into_iter();
    let head = it.next().expect("starts with SELECT");
    let mut keyed = Vec::with_capacity(items);
    let mut separators = Vec::with_capacity(commas);
    for c in it {
        if is_tok(&c, ",") {
            separators.push(c);
        } else {
            keyed.push((fold_key(&c), c));
        }
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = vec![head];
    let mut separators = separators.into_iter();
    for (i, (_, item)) in keyed.into_iter().enumerate() {
        if i > 0 {
            out.extend(separators.next());
        }
        out.push(item);
    }
    out
}

/// DML verb and `as user|system` tokens lifted out of their wrapper nodes.
fn flatten_dml(children: Vec<KRaw>) -> Vec<KRaw> {
    let mut out = Vec::with_capacity(children.len() + 2);
    for c in children {
        match c {
            KRaw::Node {
                kind: "dml_type" | "dml_security_mode",
                children,
                ..
            } => {
                for inner in children {
                    match inner {
                        KRaw::Node { children, .. } => out.extend(children),
                        token => out.push(token),
                    }
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// `new Account(Name = 'x', Amount__c = 1)`: anchor each initializer's field name as a
/// `field_identifier`.
fn sobject_initializers(children: Vec<KRaw>) -> Vec<KRaw> {
    children
        .into_iter()
        .map(|c| match c {
            KRaw::Node {
                kind: "argument_list",
                field,
                children,
            } => KRaw::Node {
                kind: "argument_list",
                field,
                children: children.into_iter().map(anchor_initializer).collect(),
            },
            other => other,
        })
        .collect()
}

fn anchor_initializer(arg: KRaw) -> KRaw {
    match arg {
        KRaw::Node {
            kind: "assignment_expression",
            field,
            children,
        } => KRaw::Node {
            kind: "assignment_expression",
            field,
            children: children
                .into_iter()
                .map(|c| match c {
                    t @ KRaw::Token {
                        kind: "identifier",
                        field: Some("left"),
                        ..
                    } => KRaw::Node {
                        kind: "field_identifier",
                        field: Some("left"),
                        children: vec![t.with_field(None)],
                    },
                    other => other,
                })
                .collect(),
        },
        other => other,
    }
}

/// `when Account a`: the bound identifier becomes a declared `name`.
fn name_when_binding(children: Vec<KRaw>) -> Vec<KRaw> {
    children
        .into_iter()
        .map(|c| match c {
            t @ KRaw::Token {
                kind: "identifier", ..
            } => t.with_field(Some("name")),
            other => other,
        })
        .collect()
}
