//! Featurization (paper §3.2; reference `similar.py:253-345`) and stable feature ids (D7).
//!
//! For every visible non-keyword token n the extractor emits, in this order:
//!   token     `n` or `#VAR`
//!   parent    `{label}{slot}>{n}` for up to `n_parents` ancestors (inner to outer)
//!   var-usage `{ctx(prev use)}>>>{ctx(n)}` for up to `n_var_siblings` previous uses
//!   sibling   `{prev token}>>{n}` for the previous `n_siblings` tokens of the leaf stream
//! Pair features are attributed to both endpoints, so a unit's bag holds each pair twice.
//! Language configs can substitute fixed labels, child-label slots and unordered
//! all-pairs usage links; the defaults above retain the reference behavior.
//!
//! Usage chains are keyed by raw spelling (`UsageKey::Spelling`, the reference) or by
//! the leaf's `SymbolId` (`UsageKey::Symbol`, D18), falling back to spelling for leaves
//! without a binding, so shadowed declarations get separate chains.

use crate::lang::Lang;
use crate::multiset::{Bag, FeatureId};
use crate::spt::{Elem, LeafId, NodeId, Spt, SymbolId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use xxhash_rust::xxh3::{xxh3_64, xxh3_64_with_seed};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UsageKey {
    Spelling,
    Symbol,
}

/// How an ancestor identifies the child containing a token.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParentSlot {
    #[default]
    Position,
    ChildLabel,
}

/// Arity labels retain `#` placeholders; fixed labels contain only direct keywords.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LabelMode {
    #[default]
    Arity,
    Fixed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VarUsage {
    #[default]
    Positional,
    /// Unordered links between all uses of a symbol. `n_var_siblings == 0`
    /// disables links; positive values enable all pairs, so insertion cannot evict
    /// existing evidence from a bounded predecessor window. Output is quadratic
    /// in the number of uses of each symbol.
    Positionless,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureConfig {
    pub n_parents: usize,
    pub n_siblings: usize,
    pub n_var_siblings: usize,
    /// `(#)` and `{#..#}` ancestors do not consume a parent slot (reference).
    pub skip_wrappers: bool,
    /// `similar.py:287`: a var token starting upper-case is treated as global.
    pub uppercase_is_global: bool,
    pub usage_key: UsageKey,
    #[serde(default)]
    pub parent_slot: ParentSlot,
    #[serde(default)]
    pub label_mode: LabelMode,
    #[serde(default)]
    pub var_usage: VarUsage,
}

impl FeatureConfig {
    /// Reference marking (`--compat`).
    pub const REFERENCE: FeatureConfig = FeatureConfig {
        n_parents: 3,
        n_siblings: 1,
        n_var_siblings: 2,
        skip_wrappers: true,
        uppercase_is_global: true,
        usage_key: UsageKey::Spelling,
        parent_slot: ParentSlot::Position,
        label_mode: LabelMode::Arity,
        var_usage: VarUsage::Positional,
    };
    /// D18 default.
    pub const DEFAULT: FeatureConfig = FeatureConfig {
        n_parents: 3,
        n_siblings: 1,
        n_var_siblings: 2,
        skip_wrappers: true,
        uppercase_is_global: false,
        usage_key: UsageKey::Symbol,
        parent_slot: ParentSlot::Position,
        label_mode: LabelMode::Arity,
        var_usage: VarUsage::Positional,
    };

    /// xxh3-64 of the legacy config text, with an explicit suffix for nonlegacy
    /// structural knobs. Default configurations retain their existing fingerprints.
    pub fn fingerprint(&self) -> u64 {
        xxh3_64(self.fingerprint_text().as_bytes())
    }

    fn fingerprint_text(&self) -> String {
        let mut text = format!(
            "v{}|{}|{}|{}|{}|{}|{:?}",
            FEATURE_SCHEMA,
            self.n_parents,
            self.n_siblings,
            self.n_var_siblings,
            self.skip_wrappers,
            self.uppercase_is_global,
            self.usage_key
        );
        if self.parent_slot != ParentSlot::Position
            || self.label_mode != LabelMode::Arity
            || self.var_usage != VarUsage::Positional
        {
            text.push_str(&format!(
                "|{:?}|{:?}|{:?}",
                self.parent_slot, self.label_mode, self.var_usage
            ));
        }
        text
    }
}

pub const FEATURE_SEED: u64 = 0x4152_4f4d_415f_7631;
pub const FEATURE_SCHEMA: u32 = 1;

/// `xxh3_64_with_seed("{lang}\u{1f}{feature}", FEATURE_SEED)`.
pub fn feature_id(lang: Lang, feature: &str) -> FeatureId {
    let mut buf = String::with_capacity(lang.as_str().len() + 1 + feature.len());
    feature_id_into(&mut buf, lang, feature)
}

fn feature_id_into(buf: &mut String, lang: Lang, feature: &str) -> FeatureId {
    buf.clear();
    buf.push_str(lang.as_str());
    buf.push('\u{1f}');
    buf.push_str(feature);
    xxh3_64_with_seed(buf.as_bytes(), FEATURE_SEED)
}

/// Per-leaf bags F(n), indexed by `LeafId`.
pub fn leaf_bags(lang: Lang, per_leaf: &[Vec<String>]) -> Vec<Bag> {
    let mut buf = String::new();
    per_leaf
        .iter()
        .map(|fs| Bag::from_ids(fs.iter().map(|f| feature_id_into(&mut buf, lang, f))))
        .collect()
}

/// Unit bag F(t) = multiset sum of the per-leaf bags.
pub fn unit_bag(lang: Lang, per_leaf: &[Vec<String>]) -> Bag {
    let mut buf = String::new();
    Bag::from_ids(
        per_leaf
            .iter()
            .flatten()
            .map(|f| feature_id_into(&mut buf, lang, f)),
    )
}

fn is_wrapper(label: &str) -> bool {
    label == "(#)"
        || (label.len() >= 2
            && label.starts_with('{')
            && label.ends_with('}')
            && label[1..label.len() - 1].bytes().all(|b| b == b'#'))
}

/// Per-leaf feature strings F(n), indexed by `LeafId`. Leaves outside `mask` (or beyond
/// its length) are invisible: they emit nothing and break no sibling or usage chain,
/// like the `" ... "` dummies of `similar.py`'s `prune_ast`.
pub fn featurize(spt: &Spt, cfg: &FeatureConfig, mask: Option<&[bool]>) -> Vec<Vec<String>> {
    let mut st = State {
        spt,
        cfg,
        mask,
        out: vec![Vec::new(); spt.leaves.len()],
        labels: spt
            .nodes
            .iter()
            .map(|node| match cfg.label_mode {
                LabelMode::Arity => node.label.to_string(),
                LabelMode::Fixed => node
                    .children
                    .iter()
                    .filter_map(|e| match e {
                        Elem::Keyword(k) => Some(k.as_ref()),
                        _ => None,
                    })
                    .collect(),
            })
            .collect(),
        ancestors: Vec::new(),
        prev_leaves: Vec::new(),
        var_uses: HashMap::new(),
    };
    if (spt.root as usize) < spt.nodes.len() {
        st.walk(spt.root);
    }
    st.out
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum UseKey<'a> {
    Symbol(SymbolId),
    Spelling(&'a str),
}

struct State<'a> {
    spt: &'a Spt,
    cfg: &'a FeatureConfig,
    labels: Vec<String>,
    mask: Option<&'a [bool]>,
    out: Vec<Vec<String>>,
    /// (ancestor node, 1-based slot of the child we descended into; keywords count)
    ancestors: Vec<(NodeId, usize)>,
    prev_leaves: Vec<(LeafId, String)>,
    var_uses: HashMap<UseKey<'a>, Vec<(LeafId, String)>>,
}

impl<'a> State<'a> {
    fn visible(&self, l: LeafId) -> bool {
        self.mask
            .is_none_or(|m| m.get(l as usize).copied().unwrap_or(false))
    }

    fn walk(&mut self, node: NodeId) {
        let spt = self.spt;
        for (i, e) in spt.nodes[node as usize].children.iter().enumerate() {
            self.ancestors.push((node, i + 1)); // slot 0 is the label in the reference JSON
            match *e {
                Elem::Tree(c) if (c as usize) < spt.nodes.len() => self.walk(c),
                Elem::Leaf(l) if (l as usize) < spt.leaves.len() && self.visible(l) => self.leaf(l),
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
        let key: String = if is_var {
            "#VAR".into()
        } else {
            leaf.text.to_string()
        };

        self.out[l as usize].push(key.clone());

        let mut emitted = 0;
        for i in (0..self.ancestors.len()).rev() {
            if emitted >= self.cfg.n_parents {
                break;
            }
            let (p, slot) = self.ancestors[i];
            let label = &self.labels[p as usize];
            if self.cfg.skip_wrappers && is_wrapper(label) {
                continue;
            }
            let context = self.parent_context(p, slot);
            self.out[l as usize].push(format!("{context}>{key}"));
            emitted += 1;
        }

        if is_var && self.cfg.n_var_siblings > 0 {
            if let Some(ctx) = self.var_context() {
                let use_key = match (self.cfg.usage_key, leaf.binding) {
                    (UsageKey::Symbol, Some(s)) => UseKey::Symbol(s),
                    _ => UseKey::Spelling(&leaf.text),
                };
                let uses = self.var_uses.entry(use_key).or_default();
                let pairs: Vec<(LeafId, String)> = uses
                    .iter()
                    .rev()
                    .take(if self.cfg.var_usage == VarUsage::Positionless {
                        usize::MAX
                    } else {
                        self.cfg.n_var_siblings
                    })
                    .map(|(prev, prev_ctx)| {
                        let (a, b) =
                            if self.cfg.var_usage == VarUsage::Positionless && prev_ctx > &ctx {
                                (&ctx, prev_ctx)
                            } else {
                                (prev_ctx, &ctx)
                            };
                        (*prev, format!("{a}>>>{b}"))
                    })
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

    fn parent_context(&self, p: NodeId, slot: usize) -> String {
        let label = &self.labels[p as usize];
        match self.cfg.parent_slot {
            ParentSlot::Position => format!("{label}{slot}"),
            ParentSlot::ChildLabel => {
                let child = match self.spt.nodes[p as usize].children.get(slot - 1) {
                    Some(Elem::Tree(n)) => self.labels.get(*n as usize).map_or("#", String::as_str),
                    _ => "#",
                };
                // Length framing avoids collisions between parent/child label boundaries.
                format!("{}:{label}{}:{child}", label.len(), child.len())
            }
        }
    }

    /// C(n): `{label}{slot}` of the immediate parent, except under `#.#` where it is the
    /// text of the leftmost visible leaf two slots to the right (`x.foo()` -> `foo`;
    /// `similar.py:246-250`). `None` (no usage feature) when that slot does not exist
    /// or holds no visible leaf; the reference would raise there.
    fn var_context(&self) -> Option<String> {
        let &(p, slot) = self.ancestors.last()?;
        if self.cfg.var_usage == VarUsage::Positionless {
            return Some(self.labels[p as usize].clone());
        }
        let label = &self.labels[p as usize];
        if label == "#.#" {
            // reference p_ast[slot + 2] == children[slot + 1] (p_ast[0] is the label)
            let target = self.spt.nodes[p as usize].children.get(slot + 1)?;
            let leaf = self.spt.leftmost_leaf(target, self.mask)?;
            Some(self.spt.leaves.get(leaf as usize)?.text.to_string())
        } else {
            Some(format!("{label}{slot}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_text_spells_every_knob() {
        assert_eq!(
            FeatureConfig::REFERENCE.fingerprint_text(),
            "v1|3|1|2|true|true|Spelling"
        );
        assert_eq!(
            FeatureConfig::DEFAULT.fingerprint_text(),
            "v1|3|1|2|true|false|Symbol"
        );
    }

    #[test]
    fn nondefault_structural_knobs_have_distinct_fingerprints() {
        let base = FeatureConfig::DEFAULT;
        let configs = [
            FeatureConfig {
                parent_slot: ParentSlot::ChildLabel,
                ..base
            },
            FeatureConfig {
                label_mode: LabelMode::Fixed,
                ..base
            },
            FeatureConfig {
                var_usage: VarUsage::Positionless,
                ..base
            },
        ];
        let mut fingerprints = std::collections::BTreeSet::from([base.fingerprint()]);
        for config in configs {
            assert!(fingerprints.insert(config.fingerprint()));
            assert_eq!(
                serde_json::from_str::<FeatureConfig>(&serde_json::to_string(&config).unwrap())
                    .unwrap(),
                config
            );
        }
    }

    #[test]
    fn wrapper_labels_match_the_reference_skip_rule() {
        for w in ["(#)", "{}", "{#}", "{###}"] {
            assert!(is_wrapper(w), "{w}");
        }
        for n in ["##", "#()", "(#)#", "{#;}", "{", "}", "#"] {
            assert!(!is_wrapper(n), "{n}");
        }
    }

    #[test]
    fn feature_id_into_ignores_previous_buffer_content() {
        let mut buf = String::from("stale text");
        assert_eq!(
            feature_id_into(&mut buf, Lang::Java, "#VAR"),
            feature_id(Lang::Java, "#VAR")
        );
    }
}
