//! Featurization (paper section 3.2; reference similar.py:253-345).
//!
//! For every non-keyword token n the extractor emits, in this order:
//!   token     `n` or `#VAR`
//!   parent    `{label}{slot}>{n}` for up to 3 ancestors (inner to outer)
//!   var-usage `{ctx(prev use)}>>>{ctx(n)}` for up to 2 previous uses of a local
//!   sibling   `{prev token}>>{n}` for the previous token in the flat leaf stream
//! Pair features are attributed to both endpoints, which is why a method's bag holds
//! each pair twice (the paper's `(n, Next(n))` and `(Prev(n), n)`).

use crate::multiset::{Bag, FeatureId};
use crate::spt::{Elem, LeafId, NodeId, Spt};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub struct FeatureConfig {
    pub n_parents: usize,
    /// 1 in the shipped code; `example_features.txt` was generated with 2.
    pub n_siblings: usize,
    /// Paper: one PrevUse and one NextUse. Reference: the 2 previous uses.
    pub n_var_siblings: usize,
    /// Reference only: `(#)` and `{#..#}` ancestors do not consume a parent slot.
    pub skip_wrappers: bool,
    /// similar.py:287: a `var` token starting upper-case is treated as global. Java only.
    pub uppercase_is_global: bool,
}

impl FeatureConfig {
    pub const REFERENCE: Self = Self {
        n_parents: 3,
        n_siblings: 1,
        n_var_siblings: 2,
        skip_wrappers: true,
        uppercase_is_global: true,
    };
}

fn is_wrapper(label: &str) -> bool {
    label == "(#)"
        || (label.len() >= 2
            && label.starts_with('{')
            && label.ends_with('}')
            && label[1..label.len() - 1].bytes().all(|b| b == b'#'))
}

/// Per-leaf feature strings F(n), indexed by `LeafId`. Masked-out leaves (pruned away)
/// are invisible: they emit nothing and break no sibling or usage chain, exactly like
/// the dummy `" ... "` tokens left behind by similar.py's `prune_ast`.
pub fn featurize(spt: &Spt, cfg: &FeatureConfig, mask: Option<&[bool]>) -> Vec<Vec<String>> {
    let mut st = State {
        spt,
        cfg,
        mask,
        out: vec![Vec::new(); spt.leaves.len()],
        ancestors: Vec::new(),
        prev_leaves: Vec::new(),
        var_uses: HashMap::new(),
    };
    st.walk(spt.root);
    st.out
}

struct State<'a> {
    spt: &'a Spt,
    cfg: &'a FeatureConfig,
    mask: Option<&'a [bool]>,
    out: Vec<Vec<String>>,
    /// (ancestor node, 1-based slot of the child we descended into; keywords count)
    ancestors: Vec<(NodeId, usize)>,
    prev_leaves: Vec<(LeafId, String)>,
    /// Keyed by raw spelling, as in the reference (so shadowed locals collide).
    var_uses: HashMap<&'a str, Vec<(LeafId, String)>>,
}

impl<'a> State<'a> {
    fn visible(&self, l: LeafId) -> bool {
        self.mask.is_none_or(|m| m[l as usize])
    }

    fn walk(&mut self, node: NodeId) {
        let spt = self.spt;
        for (i, e) in spt.nodes[node as usize].children.iter().enumerate() {
            self.ancestors.push((node, i + 1)); // slot 0 is the label in the reference JSON
            match *e {
                Elem::Tree(c) => self.walk(c),
                Elem::Leaf(l) if self.visible(l) => self.leaf(l),
                _ => {}
            }
            self.ancestors.pop();
        }
    }

    fn pair(&mut self, a: LeafId, b: LeafId, f: String) {
        self.out[a as usize].push(f.clone());
        self.out[b as usize].push(f);
    }

    fn leaf(&mut self, l: LeafId) {
        let spt = self.spt;
        let leaf = &spt.leaves[l as usize];
        let upper = leaf.text.chars().next().is_some_and(char::is_uppercase);
        let is_var = leaf.is_var && !(self.cfg.uppercase_is_global && upper);
        let key: String = if is_var { "#VAR".into() } else { leaf.text.to_string() };

        self.out[l as usize].push(key.clone());

        let mut emitted = 0;
        for i in (0..self.ancestors.len()).rev() {
            let (p, slot) = self.ancestors[i];
            let label = spt.label(p);
            if self.cfg.skip_wrappers && is_wrapper(label) {
                continue;
            }
            self.out[l as usize].push(format!("{label}{slot}>{key}"));
            emitted += 1;
            if emitted >= self.cfg.n_parents {
                break;
            }
        }

        if is_var && self.cfg.n_var_siblings > 0 {
            if let Some(ctx) = self.var_context() {
                let uses = self.var_uses.entry(&leaf.text).or_default();
                let pairs: Vec<(LeafId, String)> = uses
                    .iter()
                    .rev()
                    .take(self.cfg.n_var_siblings)
                    .map(|(prev, prev_ctx)| (*prev, format!("{prev_ctx}>>>{ctx}")))
                    .collect();
                uses.push((l, ctx));
                for (prev, f) in pairs {
                    self.pair(prev, l, f);
                }
            }
        }

        let pairs: Vec<(LeafId, String)> = self
            .prev_leaves
            .iter()
            .rev()
            .take(self.cfg.n_siblings)
            .map(|(prev, prev_key)| (*prev, format!("{prev_key}>>{key}")))
            .collect();
        for (prev, f) in pairs {
            self.pair(prev, l, f);
        }
        self.prev_leaves.push((l, key));
    }

    /// C(n): `{label}{slot}` of the immediate parent, except under `#.#` where it is the
    /// leftmost leaf two slots to the right (`x.foo()` -> `foo`; similar.py:246-250).
    fn var_context(&self) -> Option<String> {
        let &(p, slot) = self.ancestors.last()?;
        let label = self.spt.label(p);
        if label == "#.#" {
            // reference p_ast[slot + 2] == children[slot + 1] (p_ast[0] is the label)
            let target = self.spt.nodes[p as usize].children.get(slot + 1)?;
            let leaf = self.spt.leftmost_leaf(target, self.mask)?;
            Some(self.spt.leaves[leaf as usize].text.to_string())
        } else {
            Some(format!("{label}{slot}"))
        }
    }
}

/// Feature-string interner (reference `Vocab`; ids are dense and stable per index).
#[derive(Default, Debug)]
pub struct Vocab {
    ids: HashMap<String, FeatureId>,
    words: Vec<String>,
}

impl Vocab {
    pub fn intern(&mut self, s: &str) -> FeatureId {
        if let Some(&id) = self.ids.get(s) {
            return id;
        }
        let id = self.words.len() as FeatureId;
        self.words.push(s.to_owned());
        self.ids.insert(s.to_owned(), id);
        id
    }

    pub fn word(&self, id: FeatureId) -> &str {
        &self.words[id as usize]
    }

    pub fn bag(&mut self, features: &[String]) -> Bag {
        Bag::from_ids(features.iter().map(|f| self.intern(f)))
    }

    /// Unit bag F(t) = multiset union of the per-leaf bags F(n).
    pub fn unit_bag(&mut self, per_leaf: &[Vec<String>]) -> Bag {
        Bag::from_ids(per_leaf.iter().flatten().map(|f| self.intern(f)))
    }
}
