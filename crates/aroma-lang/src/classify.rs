//! Stage 3 (D21): D18 context-free `#VAR` classification, `SymbolId` assignment and
//! ASCII case folding, `KRaw` -> `Raw` -> `Spt`.
//!
//! * A token is a leaf (non-keyword token) iff it is named and its kind is in
//!   `spec.leaf_kinds` or `spec.atomic_kinds`; every other token is a keyword. The `#ERR`
//!   token is always a keyword and is never folded.
//! * When `spec.fold_case`, leaf and keyword text is ASCII-lowercased unless the token's
//!   kind contains `string` (string literals keep their case).
//! * Only `spec.identifier_kind` leaves can be `#VAR`. With parent = the enclosing
//!   `KRaw::Node` kind and field = the token's field:
//!   1. parent in `decl_name_parents` and field `name` -> declared local: `#VAR`;
//!   2. (parent, field) in `non_var_fields` -> never `#VAR` (callee, accessed field);
//!   3. parent in `non_var_parents` -> never `#VAR` (types, labels, SOQL names, ...);
//!   4. otherwise (expression, receiver or `:bind` position): `VarMode::ContextFree` ->
//!      `#VAR` if the name resolves to an in-unit declaration (D28, amends D18: even when
//!      it is a known global), else `#VAR` unless `globals.contains`;
//!      `VarMode::Compat` -> `#VAR` (the reference's
//!      syntactic marking; its upper-case exemption lives in
//!      `FeatureConfig::uppercase_is_global`).
//!
//!   The rule never looks outside the tree it is given, so a unit body classifies
//!   identically whether indexed or pasted as a snippet (D18 invariant).
//! * `SymbolId`s (`VarMode::ContextFree` only): every declared local gets a fresh id,
//!   numbered from 0 in source order, bound in the innermost enclosing scope node
//!   (`SCOPE_KINDS`, plus the tree root). A `#VAR` use takes the id of the innermost
//!   visible declaration with the same (folded) spelling, or `None` when there is none
//!   (parameters and fields declared outside the unit). `VarMode::Compat` leaves every
//!   binding `None`, like the reference.
//!
//! The conversion uses an explicit stack, so input nesting never grows the call stack.

use crate::globals::Globals;
use crate::kraw::{KRaw, ERR_TOKEN};
use crate::spec::LangSpec;
use crate::table::ExtractedUnit;
use aroma_core::spt::{Leaf, Raw, Spt, SymbolId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VarMode {
    ContextFree,
    Compat,
}

/// Node kinds (Java and Apex grammars) that open a lexical scope for declared locals.
const SCOPE_KINDS: &[&str] = &[
    "block",
    "constructor_body",
    "trigger_body",
    "class_body",
    "for_statement",
    "enhanced_for_statement",
    "catch_clause",
    "lambda_expression",
    "try_with_resources_statement",
    "switch_block_statement_group",
    "switch_rule",
];

struct Classifier<'a> {
    spec: &'a LangSpec,
    globals: &'a Globals,
    mode: VarMode,
    /// Innermost scope last; each holds (folded spelling, id) in declaration order.
    scopes: Vec<Vec<(String, SymbolId)>>,
    next_symbol: u32,
    /// LWC loop bindings enter scope only after the start tag's collection values.
    deferred: Vec<Vec<(String, SymbolId)>>,
}

/// One open `KRaw::Node` during `Classifier::convert`.
struct Frame {
    kind: &'static str,
    scoped: bool,
    /// Scope receiving JS `var` declarations inside this function.
    function_scope: Option<usize>,
    pending: std::vec::IntoIter<KRaw>,
    out: Vec<Raw>,
}

impl Classifier<'_> {
    fn fold(&self, kind: &str, text: String) -> String {
        if self.spec.fold_case && !kind.contains("string") {
            text.to_ascii_lowercase()
        } else {
            text
        }
    }

    /// `Some(declared)` when an identifier is `#VAR`, `None` when it is not.
    fn var_role(&self, parent: Option<&str>, field: Option<&str>, text: &str) -> Option<bool> {
        let spec = self.spec;
        if !spec.markup_bindings.is_empty() {
            return match field {
                Some("markup_decl") => Some(true),
                Some("markup_property") => None,
                _ => self.resolve(text).map(|_| false),
            };
        }
        if spec.lang == aroma_core::lang::Lang::SfMeta {
            return (field != Some("sfmeta_field") && !crate::normalize::sfmeta::field_api(text))
                .then(|| self.resolve(text).map(|_| false))
                .flatten();
        }
        if let Some(p) = parent {
            if field == Some("name") && spec.decl_name_parents.contains(&p) {
                return Some(true);
            }
            if spec
                .non_var_fields
                .iter()
                .any(|&(k, f)| k == p && Some(f) == field)
            {
                return None;
            }
            if spec.non_var_parents.contains(&p) {
                return None;
            }
        }
        match self.mode {
            VarMode::Compat => Some(false),
            // D28 (amends D18): scope resolution first. A use bound to an in-unit
            // declaration is #VAR even if its name is a known global.
            VarMode::ContextFree => {
                if self.resolve(text).is_some() {
                    Some(false)
                } else {
                    (!self.globals.contains(spec, text)).then_some(false)
                }
            }
        }
    }

    fn declare(&mut self, name: &str, scope: Option<usize>) -> SymbolId {
        let id = SymbolId(self.next_symbol);
        self.next_symbol += 1;
        if let Some(scope) = scope.and_then(|i| self.scopes.get_mut(i)) {
            // An explicit declaration scope (JS `var` hoisting, function names).
            scope.push((name.to_owned(), id));
        } else if !self.spec.markup_bindings.is_empty() && !self.deferred.is_empty() {
            // Markup loop bindings enter scope after the start tag's own values.
            if let Some(deferred) = self.deferred.last_mut() {
                deferred.push((name.to_owned(), id));
            }
        } else if let Some(scope) = self.scopes.last_mut() {
            scope.push((name.to_owned(), id));
        }
        id
    }

    fn resolve(&self, name: &str) -> Option<SymbolId> {
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.iter().rev().find(|(n, _)| n == name).map(|&(_, id)| id))
    }

    /// Leaf or keyword for one stage-1 token whose enclosing node kind is `parent`.
    fn token(
        &mut self,
        parent: Option<&str>,
        token: KRaw,
        declaration_scope: Option<usize>,
    ) -> Raw {
        let KRaw::Token {
            kind,
            named,
            text,
            bytes,
            line,
            field,
        } = token
        else {
            unreachable!("token() is only called with KRaw::Token")
        };
        if !named && text == ERR_TOKEN {
            return Raw::Keyword(text);
        }
        let text = self.fold(kind, text);
        let spec = self.spec;
        let is_leaf =
            named && (spec.leaf_kinds.contains(&kind) || spec.atomic_kinds.contains(&kind));
        if !is_leaf {
            return Raw::Keyword(text);
        }
        let role = if kind == spec.identifier_kind {
            self.var_role(parent, field, &text)
        } else {
            None
        };
        let binding = match (self.mode, role) {
            (VarMode::ContextFree, Some(true)) => Some(self.declare(&text, declaration_scope)),
            (VarMode::ContextFree, Some(false)) => self.resolve(&text),
            (VarMode::Compat, Some(true)) if !spec.markup_bindings.is_empty() => {
                self.declare(&text, declaration_scope);
                None
            }
            _ => None,
        };
        Raw::Leaf(Leaf {
            text: text.into(),
            is_var: role.is_some(),
            binding,
            bytes,
            line,
        })
    }

    fn open(&mut self, kind: &'static str, children: Vec<KRaw>) -> Frame {
        let scoped = SCOPE_KINDS.contains(&kind)
            || self.spec.js_scope_kinds.contains(&kind)
            || (!self.spec.markup_bindings.is_empty() && kind == "element");
        if scoped {
            self.scopes.push(Vec::new());
        }
        if !self.spec.markup_bindings.is_empty() && kind == "start_tag" {
            self.deferred.push(Vec::new());
        }
        let function_scope = self
            .spec
            .function_scope_kinds
            .contains(&kind)
            .then(|| self.scopes.len().saturating_sub(1));
        Frame {
            kind,
            scoped,
            function_scope,
            out: Vec::with_capacity(children.len()),
            pending: children.into_iter(),
        }
    }

    /// Explicit-stack conversion of a whole tree.
    fn convert(&mut self, root: KRaw) -> Raw {
        let (kind, children) = match root {
            token @ KRaw::Token { .. } => return self.token(None, token, None),
            KRaw::Node { kind, children, .. } => (kind, children),
        };
        let mut stack = vec![self.open(kind, children)];
        loop {
            let top = stack
                .last_mut()
                .expect("the root frame stays until it returns");
            match top.pending.next() {
                Some(token @ KRaw::Token { .. }) => {
                    let parent = top.kind;
                    let declaration_scope = if !self.spec.function_scope_kinds.is_empty()
                        && token.field() == Some("name")
                        && self.spec.decl_name_parents.contains(&parent)
                    {
                        if parent == "function_declaration" {
                            // A declaration binds in the enclosing scope; its
                            // parameters/body still have their own function scope.
                            Some(self.scopes.len() - 2)
                        } else if stack
                            .iter()
                            .rev()
                            .take_while(|f| f.function_scope.is_none())
                            .find(|f| {
                                matches!(
                                    f.kind,
                                    "variable_declaration"
                                        | "lexical_declaration"
                                        | "for_in_statement"
                                )
                            })
                            .is_some_and(|f| {
                                f.kind == "variable_declaration"
                                    || (f.kind == "for_in_statement"
                                        && f.out
                                            .iter()
                                            .any(|c| matches!(c, Raw::Keyword(k) if k == "var")))
                            })
                        {
                            // `var` crosses blocks, but never a nested function.
                            Some(
                                stack
                                    .iter()
                                    .rev()
                                    .find_map(|f| f.function_scope)
                                    .unwrap_or(0),
                            )
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    let raw = self.token(Some(parent), token, declaration_scope);
                    if let Some(top) = stack.last_mut() {
                        top.out.push(raw);
                    }
                }
                Some(KRaw::Node { kind, children, .. }) => {
                    let frame = self.open(kind, children);
                    stack.push(frame);
                }
                None => {
                    let done = stack.pop().expect("non-empty");
                    if !self.spec.markup_bindings.is_empty() && done.kind == "start_tag" {
                        if let (Some(declarations), Some(scope)) =
                            (self.deferred.pop(), self.scopes.last_mut())
                        {
                            scope.extend(declarations);
                        }
                    }
                    if done.scoped {
                        self.scopes.pop();
                    }
                    let list = Raw::List(done.out);
                    match stack.last_mut() {
                        Some(parent) => parent.out.push(list),
                        None => return list,
                    }
                }
            }
        }
    }
}

/// Stage 3: classify and fold one (normalized) stage-1 tree. The root opens a scope.
pub fn to_raw(node: KRaw, spec: &LangSpec, globals: &Globals, mode: VarMode) -> Raw {
    let mut c = Classifier {
        spec,
        globals,
        mode,
        scopes: vec![Vec::new()],
        next_symbol: 0,
        deferred: Vec::new(),
    };
    // Metadata declarations are visible throughout the selected unit, including
    // forward references. Never consult sibling units or file-level Flow variables.
    if spec.lang == aroma_core::lang::Lang::SfMeta {
        let mut stack = vec![&node];
        while let Some(tree) = stack.pop() {
            match tree {
                KRaw::Node { children, .. } => stack.extend(children.iter().rev()),
                KRaw::Token {
                    field: Some("sfmeta_decl"),
                    text,
                    ..
                } => {
                    if c.resolve(text).is_none() {
                        c.declare(text, None);
                    }
                }
                _ => {}
            }
        }
    }
    c.convert(node)
}

/// normalize -> to_raw -> `Spt::build` for one extracted unit body.
pub fn unit_spt(
    unit: &ExtractedUnit,
    spec: &LangSpec,
    globals: &Globals,
    mode: VarMode,
) -> Option<Spt> {
    let normalized = spec.normalizer.normalize(unit.body.clone());
    let config = (spec.feature_config)(aroma_core::features::FeatureConfig::DEFAULT);
    Spt::build_with_label_mode(to_raw(normalized, spec, globals, mode), config.label_mode)
}

#[cfg(test)]
mod tests {
    use super::SCOPE_KINDS;
    use crate::spec::spec_for;
    use aroma_core::lang::Lang;

    #[test]
    fn scope_kinds_exist_in_a_pinned_grammar() {
        for kind in SCOPE_KINDS {
            let known = Lang::SPECCED
                .iter()
                .any(|&l| (spec_for(l).unwrap().language)().id_for_node_kind(kind, true) != 0);
            assert!(known, "scope kind {kind:?} is in neither grammar");
        }
    }
}
