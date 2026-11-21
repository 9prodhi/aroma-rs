//! Known globals for the D18 rule: language builtins, repository type names and global
//! suffixes. An identifier in expression or receiver position that is a known global is
//! never `#VAR` under `VarMode::ContextFree`. Corpus units and queries consult the same
//! set (persisted as `globals.json`), so both paths classify identically.

use crate::spec::LangSpec;
use serde::{Deserialize, Serialize};

/// A sorted, deduplicated name list searched by binary search. A `Vec` (not a
/// `BTreeSet`) because every one-shot query deserializes it from `globals.json`: one
/// allocation per name instead of one allocation plus a tree insert (about 15 ms for 50k
/// names on the 233k-unit perf fixture). The JSON shape is the same sorted array.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "RawGlobals")]
pub struct Globals {
    /// `spec.builtin_globals` plus repository type names, ASCII-folded when the
    /// language folds case; sorted and deduplicated.
    names: Vec<String>,
}

#[derive(Deserialize)]
struct RawGlobals {
    names: Vec<String>,
}

impl From<RawGlobals> for Globals {
    fn from(raw: RawGlobals) -> Globals {
        Globals::from_names(raw.names)
    }
}

fn fold(spec: &LangSpec, text: &str) -> String {
    if spec.fold_case {
        text.to_ascii_lowercase()
    } else {
        text.to_owned()
    }
}

impl Globals {
    /// Builtins of `spec` plus the given repository type names.
    pub fn new(spec: &LangSpec, repo_types: impl IntoIterator<Item = String>) -> Globals {
        let mut names: Vec<String> = spec.builtin_globals.iter().map(|g| fold(spec, g)).collect();
        names.extend(repo_types.into_iter().map(|t| fold(spec, &t)));
        Globals::from_names(names)
    }

    /// Sorts (cheap on the already-sorted persisted list) and deduplicates.
    fn from_names(mut names: Vec<String>) -> Globals {
        names.sort_unstable();
        names.dedup();
        Globals { names }
    }

    /// builtins ∪ repo types ∪ suffix match, after case folding when `spec.fold_case`.
    pub fn contains(&self, spec: &LangSpec, text: &str) -> bool {
        let t = fold(spec, text);
        self.names.binary_search(&t).is_ok()
            || spec.global_suffixes.iter().any(|s| t.ends_with(s))
            || (spec.lang == aroma_core::lang::Lang::LwcJs
                && t.bytes().any(|b| b.is_ascii_uppercase())
                && t.bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::spec_for;
    use aroma_core::lang::Lang;

    #[test]
    fn persisted_names_are_sorted_deduplicated_and_found() {
        let spec = spec_for(Lang::Java).unwrap();
        let g: Globals =
            serde_json::from_str(r#"{"names":["Zeta","Alpha","Zeta","Mid"]}"#).unwrap();
        assert!(g.contains(spec, "Alpha") && g.contains(spec, "Zeta") && g.contains(spec, "Mid"));
        assert!(!g.contains(spec, "Beta"));
        assert_eq!(
            serde_json::to_string(&g).unwrap(),
            r#"{"names":["Alpha","Mid","Zeta"]}"#
        );
        let built = Globals::new(spec, ["Zeta".to_string(), "Alpha".to_string()]);
        let back: Globals = serde_json::from_str(&serde_json::to_string(&built).unwrap()).unwrap();
        assert_eq!(back, built);
    }
}
