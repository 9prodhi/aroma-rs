//! Simplified Parse Tree (paper Def. 3 and 4; reference ConvertJava.java:211-308).
//!
//! A node is an ordered list of elements: keyword tokens, non-keyword leaves, or
//! sub-trees. Its label concatenates each keyword's text and `#` for every leaf or
//! sub-tree, e.g. `if##`, `#.#`, `(#)#`. No grammar rule names survive, which is
//! what makes everything after this module language-agnostic.

use std::ops::Range;

pub type NodeId = u32;
pub type LeafId = u32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Elem {
    Keyword(Box<str>),
    Leaf(LeafId),
    Tree(NodeId),
}

#[derive(Clone, Debug)]
pub struct Leaf {
    pub text: Box<str>,
    /// Set by the language adapter (reference: token's rule is in `localVarContexts`).
    pub is_var: bool,
    pub bytes: Range<usize>,
    /// 1-based source line, used by the line-granular renderer.
    pub line: u32,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub label: Box<str>,
    pub children: Vec<Elem>,
}

/// Arena-allocated tree for one indexing unit (a method body in the paper).
#[derive(Clone, Debug, Default)]
pub struct Spt {
    pub nodes: Vec<Node>,
    /// N(t): non-keyword tokens in source order. `LeafId` indexes this vector.
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
fn collapse(list: Vec<Raw>) -> Option<Collapsed> {
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
                label.push('#');
                has_leaf = true;
                elems.push(Built::Leaf(l));
            }
            Raw::List(sub) => {
                let Some(child) = collapse(sub) else { continue };
                if child.elems.len() == 1 {
                    label.push_str(&child.label); // "#" for a leaf/tree, text for a keyword
                    has_leaf |= child.has_leaf;
                    elems.extend(child.elems);
                } else if !child.has_leaf && child.label != "{}" {
                    label.push_str(&child.label);
                    elems.extend(child.elems);
                } else {
                    label.push('#');
                    has_leaf = true; // reference sets this even for a leafless `{}`
                    elems.push(Built::Tree(child));
                }
            }
        }
    }
    (!elems.is_empty()).then_some(Collapsed { label, elems, has_leaf })
}

impl Spt {
    pub fn build(root: Raw) -> Option<Spt> {
        let list = match root {
            Raw::List(l) => l,
            other => vec![other],
        };
        let collapsed = collapse(list)?;
        let mut spt = Spt::default();
        spt.root = spt.freeze(collapsed);
        Some(spt)
    }

    /// Pre-order freeze, so `leaves` ends up in source order.
    fn freeze(&mut self, c: Collapsed) -> NodeId {
        let id = self.nodes.len() as NodeId;
        self.nodes.push(Node { label: c.label.into(), children: Vec::new() });
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
    pub fn leftmost_leaf(&self, e: &Elem, mask: Option<&[bool]>) -> Option<LeafId> {
        match *e {
            Elem::Keyword(_) => None,
            Elem::Leaf(l) => mask.is_none_or(|m| m[l as usize]).then_some(l),
            Elem::Tree(n) => self.nodes[n as usize]
                .children
                .iter()
                .find_map(|c| self.leftmost_leaf(c, mask)),
        }
    }

    /// Unformatted in-order print (paper section 3.1: "print the token names").
    pub fn tokens(&self, mask: Option<&[bool]>) -> Vec<&str> {
        let mut out = Vec::new();
        self.collect_tokens(self.root, mask, &mut out);
        out
    }

    fn collect_tokens<'a>(&'a self, n: NodeId, mask: Option<&[bool]>, out: &mut Vec<&'a str>) {
        for c in &self.nodes[n as usize].children {
            match *c {
                Elem::Keyword(ref k) => out.push(k),
                Elem::Leaf(l) if mask.is_none_or(|m| m[l as usize]) => {
                    out.push(&self.leaves[l as usize].text)
                }
                Elem::Leaf(_) => out.push("..."),
                Elem::Tree(t) => self.collect_tokens(t, mask, out),
            }
        }
    }
}
