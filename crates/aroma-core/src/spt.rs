//! Simplified parse tree (paper Def. 3 and 4; reference `ConvertJava.java:211-308`).
//!
//! A node is an ordered list of elements: keyword tokens, non-keyword leaves, or
//! sub-trees. By default its label concatenates each keyword's text and `#` for every leaf or
//! sub-tree, e.g. `if##`, `#.#`, `(#)#`. No grammar rule names survive, which is what
//! makes everything after this module language-agnostic. Fixed labels retain only
//! direct keywords and keep leafless children as subtrees to isolate their labels.
//!
//! Two constructors exist:
//! * [`Spt::build`] collapses an adapter's [`Raw`] tree with the reference rules A/B/C;
//! * [`Spt::from_reference_json`] loads an already-collapsed `similar.py` JSON `ast`
//!   verbatim (labels are taken from the JSON, never recomputed).

use crate::features::LabelMode;
use serde::{Deserialize, Serialize};
use std::ops::Range;

pub type NodeId = u32;
pub type LeafId = u32;

/// Declaration identity attached by the classifier (D18/D22b); only usage chains read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SymbolId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Elem {
    Keyword(Box<str>),
    Leaf(LeafId),
    Tree(NodeId),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Leaf {
    pub text: Box<str>,
    /// Set by the classifier (`#VAR` candidate).
    pub is_var: bool,
    /// Declaration inside the unit this use refers to; `None` for undeclared names.
    pub binding: Option<SymbolId>,
    /// Byte span in the source file (`0..0` when unknown, e.g. reference JSON).
    pub bytes: Range<u32>,
    /// 1-based source line.
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub label: Box<str>,
    pub children: Vec<Elem>,
}

/// Arena-allocated tree for one unit. `leaves` holds N(t) in normalized traversal order and
/// `LeafId` indexes it; `nodes[root]` is the root.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Spt {
    pub nodes: Vec<Node>,
    pub leaves: Vec<Leaf>,
    pub root: NodeId,
}

/// Uncollapsed tree produced by a language adapter.
#[derive(Clone, Debug)]
pub enum Raw {
    Keyword(String),
    Leaf(Leaf),
    List(Vec<Raw>),
}

/// One collapsed list, mirroring the JSONArray returned by `getSerializedTree`.
struct Collapsed {
    label: String,
    elems: Vec<Built>,
    has_leaf: bool,
}

enum Built {
    Keyword(String),
    Leaf(Leaf),
    Tree(Collapsed),
}

/// Bottom-up collapse with the reference's three cases:
///   (A) a child list with exactly one element is replaced by that element;
///   (B) a child list without any leaf is spliced into the parent (except `{}`);
///   (C) otherwise the child stays a sub-tree and contributes `#` to the label.
fn collapse(list: Vec<Raw>, mode: LabelMode) -> Option<Collapsed> {
    let mut label = String::new();
    let mut elems = Vec::new();
    let mut has_leaf = false;
    for raw in list {
        match raw {
            Raw::Keyword(k) => {
                label.push_str(&k);
                elems.push(Built::Keyword(k));
            }
            Raw::Leaf(l) => {
                if mode == LabelMode::Arity {
                    label.push('#');
                }
                has_leaf = true;
                elems.push(Built::Leaf(l));
            }
            Raw::List(sub) => {
                let Some(child) = collapse(sub, mode) else {
                    continue;
                };
                if child.elems.len() == 1 {
                    label.push_str(&child.label);
                    has_leaf |= child.has_leaf;
                    elems.extend(child.elems);
                } else if mode == LabelMode::Arity && !child.has_leaf && child.label != "{}" {
                    label.push_str(&child.label);
                    elems.extend(child.elems);
                } else {
                    if mode == LabelMode::Arity {
                        label.push('#');
                    }
                    has_leaf = true; // the reference sets this even for a leafless `{}`
                    elems.push(Built::Tree(child));
                }
            }
        }
    }
    (!elems.is_empty()).then_some(Collapsed {
        label,
        elems,
        has_leaf,
    })
}

impl Spt {
    /// Collapse `root` (rules A/B/C, `ConvertJava.java:274-289`). A root whose only
    /// element is a sub-tree is replaced by that sub-tree (record-root rule, note 04 §2
    /// item 9), so no node is a unary wrapper around another node. `None` when the
    /// tree holds no token at all.
    pub fn build(root: Raw) -> Option<Spt> {
        Self::build_with_label_mode(root, LabelMode::Arity)
    }

    /// Fixed labels omit placeholders and retain leafless subtrees: adding an
    /// empty child must not splice that child's keywords into its parent's label.
    pub fn build_with_label_mode(root: Raw, mode: LabelMode) -> Option<Spt> {
        let list = match root {
            Raw::List(l) => l,
            other => vec![other],
        };
        let mut collapsed = collapse(list, mode)?;
        while collapsed.elems.len() == 1 && matches!(collapsed.elems[0], Built::Tree(_)) {
            let Some(Built::Tree(inner)) = collapsed.elems.pop() else {
                unreachable!()
            };
            collapsed = inner;
        }
        let mut spt = Spt::default();
        spt.root = spt.freeze(collapsed);
        Some(spt)
    }

    /// Pre-order freeze, so `leaves` follows the normalized tree order.
    fn freeze(&mut self, c: Collapsed) -> NodeId {
        let id = self.nodes.len() as NodeId;
        self.nodes.push(Node {
            label: c.label.into(),
            children: Vec::new(),
        });
        let mut children = Vec::with_capacity(c.elems.len());
        for e in c.elems {
            children.push(match e {
                Built::Keyword(k) => Elem::Keyword(k.into()),
                Built::Leaf(l) => {
                    let lid = self.leaves.len() as LeafId;
                    self.leaves.push(l);
                    Elem::Leaf(lid)
                }
                Built::Tree(t) => Elem::Tree(self.freeze(t)),
            });
        }
        self.nodes[id as usize].children = children;
        id
    }

    pub fn label(&self, n: NodeId) -> &str {
        &self.nodes[n as usize].label
    }

    /// Reference `get_leftmost_leaf`: first visible leaf under `e`, in source order.
    /// Leaves outside `mask` (or beyond its length) are invisible; dangling ids are skipped.
    pub fn leftmost_leaf(&self, e: &Elem, mask: Option<&[bool]>) -> Option<LeafId> {
        match *e {
            Elem::Keyword(_) => None,
            Elem::Leaf(l) => ((l as usize) < self.leaves.len() && visible(mask, l)).then_some(l),
            Elem::Tree(n) => self
                .nodes
                .get(n as usize)?
                .children
                .iter()
                .find_map(|c| self.leftmost_leaf(c, mask)),
        }
    }

    /// Unformatted in-order print (paper §3.1). Hidden leaves print as `...`.
    pub fn tokens(&self, mask: Option<&[bool]>) -> Vec<&str> {
        let mut out = Vec::new();
        self.collect_tokens(self.root, mask, &mut out);
        out
    }

    fn collect_tokens<'a>(&'a self, n: NodeId, mask: Option<&[bool]>, out: &mut Vec<&'a str>) {
        let Some(node) = self.nodes.get(n as usize) else {
            return;
        };
        for c in &node.children {
            match *c {
                Elem::Keyword(ref k) => out.push(k),
                Elem::Leaf(l) if visible(mask, l) => {
                    if let Some(leaf) = self.leaves.get(l as usize) {
                        out.push(&leaf.text);
                    }
                }
                Elem::Leaf(_) => out.push("..."),
                Elem::Tree(t) => self.collect_tokens(t, mask, out),
            }
        }
    }

    /// Load a `similar.py` record `ast` (`[label, child...]`, child = list or token
    /// object). A token is a leaf iff `"leaf"` is `true`; it is a variable iff the
    /// `"var"` key is present (`similar.py:285`); `"line"` gives its line. `None` on
    /// any other shape (including the `null` holes of a reference *pruned* tree, whose
    /// slot numbering could not be preserved), on an empty root, or when nesting
    /// exceeds 1000 levels.
    pub fn from_reference_json(v: &serde_json::Value) -> Option<Spt> {
        let mut spt = Spt::default();
        spt.root = spt.load_json(v, 0)?;
        if spt.nodes[spt.root as usize].children.is_empty() {
            return None;
        }
        Some(spt)
    }

    fn load_json(&mut self, v: &serde_json::Value, depth: usize) -> Option<NodeId> {
        const MAX_JSON_DEPTH: usize = 1000;
        if depth > MAX_JSON_DEPTH {
            return None;
        }
        let arr = v.as_array()?;
        let label = arr.first()?.as_str()?;
        let id = self.nodes.len() as NodeId;
        self.nodes.push(Node {
            label: label.into(),
            children: Vec::new(),
        });
        let mut children = Vec::with_capacity(arr.len().saturating_sub(1));
        for child in &arr[1..] {
            match child {
                serde_json::Value::Array(_) => {
                    children.push(Elem::Tree(self.load_json(child, depth + 1)?))
                }
                serde_json::Value::Object(o) => {
                    let text = o.get("token")?.as_str()?;
                    let is_leaf = o
                        .get("leaf")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    if is_leaf {
                        let line = o
                            .get("line")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0);
                        let lid = self.leaves.len() as LeafId;
                        self.leaves.push(Leaf {
                            text: text.into(),
                            is_var: o.contains_key("var"),
                            binding: None,
                            bytes: 0..0,
                            line: u32::try_from(line).unwrap_or(u32::MAX),
                        });
                        children.push(Elem::Leaf(lid));
                    } else {
                        children.push(Elem::Keyword(text.into()));
                    }
                }
                _ => return None,
            }
        }
        self.nodes[id as usize].children = children;
        Some(id)
    }
}

fn visible(mask: Option<&[bool]>, l: LeafId) -> bool {
    mask.is_none_or(|m| m.get(l as usize).copied().unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_treats_missing_mask_entries_as_hidden() {
        assert!(visible(None, 7));
        assert!(visible(Some(&[false, true]), 1));
        assert!(!visible(Some(&[false, true]), 0));
        assert!(!visible(Some(&[true]), 3));
    }
}
