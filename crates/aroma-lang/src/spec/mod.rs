//! `LangSpec`: the const tables that drive the shared adapter stages (D12/D21). Every
//! kind and field listed here is checked against the pinned grammar by the tests below.
pub mod apex;
pub mod aura;
pub mod aura_js;
pub mod java;
pub mod lwc_html;
pub mod lwc_js;
pub mod sfmeta;
pub mod vf;

use crate::normalize::Normalizer;
use crate::snippet::{Repair, Wrapper};
pub use crate::table::UnitShape;
use aroma_core::features::FeatureConfig;
use aroma_core::lang::Lang;
use aroma_core::lexical::TermRule;
use aroma_core::unit::UnitKind;

/// How a markup attribute binds an identifier to the enclosing element's scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkupBinding {
    /// The attribute's value names a loop variable (`for:item="row"`, `var="row"`).
    Decl,
    /// The attribute name is a prefix (`iterator:*`); the suffix is the variable.
    DeclPrefix,
}

/// Syntax context in which a JavaScript member snippet is parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsMemberContext {
    Class,
    Object,
}

/// JavaScript-family wrapper selection and extraction rules.
pub struct JsSnippetPolicy {
    pub member_context: JsMemberContext,
    pub method_kind: &'static str,
    /// Bare member names with these spellings retain statement precedence.
    pub statement_keywords: &'static [&'static str],
}

/// Keywords excluded from JavaScript member wrapper preference.
pub const JS_STATEMENT_KEYWORDS: &[&str] = &[
    "if", "for", "while", "do", "switch", "try", "catch", "with", "function", "return", "var",
    "let", "const", "throw", "break", "continue", "class", "import", "export",
];

/// Tables and hooks driving the shared parse, index, and query pipelines.
pub struct LangSpec {
    pub lang: Lang,
    pub language: fn() -> tree_sitter::Language,
    /// Length-preserving parser input; all token text still comes from the original source.
    pub preprocess: fn(&str) -> std::borrow::Cow<'_, str>,
    /// Match a repository-relative path with `/` separators.
    pub matches: fn(&str) -> bool,
    /// Select the body and optional leading evidence of a unit.
    pub unit: for<'t> fn(&LangSpec, tree_sitter::Node<'t>, &[u8]) -> Option<UnitShape<'t>>,
    /// Adjust the profile for both indexed units and queries of this language.
    pub feature_config: fn(FeatureConfig) -> FeatureConfig,
    /// Lexical terms used by both indexing and querying.
    pub term_rule: TermRule,
    /// Placeholder for omitted source lines.
    pub omission_marker: &'static str,
    /// How to close partial snippets before trying wrappers.
    pub repair: Repair,
    /// Snippet wrappers, in preference order.
    pub wrappers: &'static [Wrapper; 3],
    /// Named leaf kinds that are non-keyword tokens (identifiers, literals).
    pub leaf_kinds: &'static [&'static str],
    /// Kinds whose whole text is one token even if the grammar gives them children.
    pub atomic_kinds: &'static [&'static str],
    pub comment_kinds: &'static [&'static str],
    pub identifier_kind: &'static str,
    /// Unit node kinds and the kind recorded for them.
    pub units: &'static [(&'static str, UnitKind)],
    pub body_field: &'static str,
    /// Markup loop bindings: attribute name (or prefix, suffixed with ':') -> kind.
    /// Consulted by the normalizer (which tags values as `markup_decl`) and by the
    /// classifier, which opens an element scope only for markup languages that bind.
    pub markup_bindings: &'static [(&'static str, MarkupBinding)],
    /// Additional lexical scopes for JavaScript grammars; empty for other families.
    pub js_scope_kinds: &'static [&'static str],
    /// Function scopes receiving JS `var` declarations. Nonempty enables JS hoisting.
    pub function_scope_kinds: &'static [&'static str],
    /// Member-aware damage comparison, JS anchors and single-expression handling.
    pub js_snippets: Option<&'static JsSnippetPolicy>,
    /// identifier in field "name" of these -> declared local
    pub decl_name_parents: &'static [&'static str],
    /// identifier directly under these -> never #VAR
    pub non_var_parents: &'static [&'static str],
    /// (parent kind, field) -> never #VAR
    pub non_var_fields: &'static [(&'static str, &'static str)],
    /// their "name" joins repo globals
    pub type_decl_kinds: &'static [&'static str],
    /// lower-case when fold_case
    pub builtin_globals: &'static [&'static str],
    pub global_suffixes: &'static [&'static str],
    /// ASCII case-fold identifiers and keywords (never string literals).
    pub fold_case: bool,
    pub adapter_version: &'static str,
    pub grammar_version: &'static str,
    pub normalizer: &'static dyn Normalizer,
    pub is_test_path: fn(&std::path::Path) -> bool,
}

/// The implemented adapter for a language, if available.
pub fn spec_for(lang: Lang) -> Option<&'static LangSpec> {
    match lang {
        Lang::Java => Some(&java::SPEC),
        Lang::Apex => Some(&apex::SPEC),
        Lang::LwcJs => Some(&lwc_js::SPEC),
        Lang::AuraJs => Some(&aura_js::SPEC),
        Lang::LwcHtml => Some(&lwc_html::SPEC),
        // New metadata builds use the structural model. Existing indexes select
        // their recorded mode, including the legacy lexical default.
        Lang::SfMeta => Some(&sfmeta::SPEC),
        Lang::Vf => Some(&vf::SPEC),
        Lang::Aura => Some(&aura::SPEC),
    }
}

/// Route a path among enabled adapters; the first match in `Lang::SPECCED` wins.
pub fn lang_for_path(rel: &str, enabled: &[Lang]) -> Option<Lang> {
    Lang::SPECCED
        .iter()
        .copied()
        .find(|lang| enabled.contains(lang) && spec_for(*lang).is_some_and(|s| (s.matches)(rel)))
}

/// Resolve an exact language name or a snippet-sensitive `lwc`/`aura` alias.
/// Known languages without an adapter return a clear unsupported-language error.
pub fn resolve_query_lang(name: &str, snippet: &str) -> Result<Lang, String> {
    let markup = snippet.trim_start().starts_with('<');
    let lang = match name {
        "lwc" => Some(if markup { Lang::LwcHtml } else { Lang::LwcJs }),
        "aura" => Some(if markup { Lang::Aura } else { Lang::AuraJs }),
        _ => Lang::parse(name),
    }.ok_or_else(|| format!("unknown language {name:?}; valid names: java, apex, lwcjs, lwchtml, sfmeta, aura, aurajs, vf, lwc"))?;
    spec_for(lang).ok_or_else(|| format!("{} is not supported yet", lang.as_str()))?;
    Ok(lang)
}

/// Default parser input: borrow the original source without allocating.
pub fn identity_preprocess(source: &str) -> std::borrow::Cow<'_, str> {
    std::borrow::Cow::Borrowed(source)
}

/// Default feature hook: retain the selected profile unchanged.
pub fn identity_features(config: FeatureConfig) -> FeatureConfig {
    config
}

/// Case-insensitive extension matching for the Java and Apex adapters.
fn has_extension(rel: &str, extensions: &[&str]) -> bool {
    let file = rel.rsplit('/').next().unwrap_or(rel);
    file.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty() && extensions.iter().any(|e| ext.eq_ignore_ascii_case(e))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn named_kinds(spec: &LangSpec) -> Vec<&'static str> {
        let mut v: Vec<&'static str> = Vec::new();
        v.extend(spec.leaf_kinds);
        v.extend(spec.atomic_kinds);
        v.extend(spec.comment_kinds);
        v.push(spec.identifier_kind);
        v.extend(spec.units.iter().map(|(k, _)| *k));
        v.extend(spec.decl_name_parents);
        v.extend(spec.non_var_parents);
        v.extend(spec.non_var_fields.iter().map(|(k, _)| *k));
        v.extend(spec.type_decl_kinds);
        v.extend(spec.js_scope_kinds);
        v.extend(spec.function_scope_kinds);
        if let Some(policy) = spec.js_snippets {
            v.push(policy.method_kind);
        }
        v
    }

    #[test]
    fn both_grammars_load() {
        for &lang in Lang::SPECCED {
            let spec = spec_for(lang).unwrap();
            assert_eq!(spec.lang, lang);
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&(spec.language)())
                .unwrap_or_else(|e| panic!("{lang:?}: {e}"));
            let src = match lang {
                Lang::Java => "class A { void m() { int x = 1; } }",
                Lang::Apex => "public class A { void m() { Integer x = 1; } }",
                Lang::LwcJs => "class A { m() { const x = 1; } }",
                Lang::AuraJs => "({ run: function(cmp) { return cmp; } })",
                Lang::LwcHtml => "<template><p>{greeting}</p></template>",
                Lang::SfMeta => "<CustomField><type>Text</type></CustomField>",
                Lang::Vf => "<apex:page><apex:outputText value=\"{!greeting}\"/></apex:page>",
                Lang::Aura => "<aura:component><aura:iteration var=\"row\" items=\"{!v.rows}\"><aura:if value=\"{!v.show}\"><p>{#v.name}</p></aura:if></aura:iteration></aura:component>",
            };
            let tree = parser.parse(src, None).unwrap();
            assert!(
                !tree.root_node().has_error(),
                "{lang:?}: {}",
                tree.root_node().to_sexp()
            );
        }
    }

    #[test]
    fn every_listed_kind_exists_in_the_grammar() {
        for &lang in Lang::SPECCED {
            let spec = spec_for(lang).unwrap();
            let language = (spec.language)();
            for kind in named_kinds(spec) {
                // Expression tokens are produced by the shared expression parser,
                // rather than by the host HTML grammar.
                if matches!(lang, Lang::LwcHtml | Lang::SfMeta | Lang::Vf | Lang::Aura)
                    && matches!(kind, "identifier" | "literal")
                {
                    continue;
                }
                assert_ne!(
                    language.id_for_node_kind(kind, true),
                    0,
                    "{lang:?}: named kind {kind:?} missing"
                );
            }
            let mut fields = Vec::new();
            if !spec.body_field.is_empty() {
                fields.extend([spec.body_field, "name"]);
            }
            fields.extend(spec.non_var_fields.iter().map(|(_, f)| *f));
            for f in fields {
                assert!(
                    language.field_id_for_name(f).is_some(),
                    "{lang:?}: field {f:?} missing"
                );
            }
        }
    }

    #[test]
    fn javascript_scope_and_wrapper_tables_are_consistent() {
        for &lang in Lang::SPECCED {
            let spec = spec_for(lang).unwrap();
            for kind in spec.function_scope_kinds {
                assert!(spec.js_scope_kinds.contains(kind), "{lang:?}: {kind}");
            }
            if let Some(policy) = spec.js_snippets {
                assert!(!spec.function_scope_kinds.is_empty(), "{lang:?}");
                assert_eq!(spec.repair, Repair::Brackets);
                assert_eq!(spec.wrappers[0].shape, crate::snippet::Shape::Statements);
                assert_eq!(spec.wrappers[1].shape, crate::snippet::Shape::Member);
                assert_eq!(spec.wrappers[2].shape, crate::snippet::Shape::Expression);
                assert!(policy.statement_keywords.contains(&"if"));
            } else {
                assert!(spec.js_scope_kinds.is_empty(), "{lang:?}");
                assert!(spec.function_scope_kinds.is_empty(), "{lang:?}");
            }
        }
    }

    #[test]
    fn matches_agree_with_lang() {
        for (path, lang) in [
            ("src/A.java", Lang::Java),
            ("classes/A.cls", Lang::Apex),
            ("triggers/A.trigger", Lang::Apex),
            ("scripts/A.apex", Lang::Apex),
            ("pages/Account.page", Lang::Vf),
            ("pages/Account.component", Lang::Vf),
            ("aura/MyBundle/MyBundle.cmp", Lang::Aura),
            ("aura/MyApp/MyApp.app", Lang::Aura),
            ("aura/MyEvt/MyEvt.evt", Lang::Aura),
            ("aura/MyIntf/MyIntf.intf", Lang::Aura),
            ("aura/MyDesign/MyDesign.design", Lang::Aura),
        ] {
            assert!((spec_for(lang).unwrap().matches)(path));
            assert_eq!(lang_for_path(path, Lang::SPECCED), Some(lang));
            assert_eq!(
                lang_for_path(&path.to_ascii_uppercase(), Lang::SPECCED),
                Some(lang)
            );
        }
    }

    #[test]
    fn folded_builtins_are_lower_case() {
        for &lang in Lang::SPECCED {
            let spec = spec_for(lang).unwrap();
            if spec.fold_case {
                for g in spec.builtin_globals.iter().chain(spec.global_suffixes) {
                    assert_eq!(g.to_ascii_lowercase(), *g, "{lang:?}: {g}");
                }
            }
        }
    }

    #[test]
    fn test_paths() {
        let apex = spec_for(Lang::Apex).unwrap();
        assert!((apex.is_test_path)(Path::new(
            "force-app/main/default/classes/AccountServiceTest.cls"
        )));
        assert!((apex.is_test_path)(Path::new("TESTS/Foo.cls")));
        assert!(!(apex.is_test_path)(Path::new(
            "force-app/main/default/classes/AccountService.cls"
        )));
        let java = spec_for(Lang::Java).unwrap();
        assert!((java.is_test_path)(Path::new(
            "src/test/java/a/FooTest.java"
        )));
        assert!((java.is_test_path)(Path::new("lib/FooTests.java")));
        assert!(!(java.is_test_path)(Path::new(
            "src/main/java/a/Contest.java"
        )));
    }

    #[test]
    fn spec_versions_name_the_pinned_grammars() {
        assert_eq!(
            spec_for(Lang::Java).unwrap().grammar_version,
            "tree-sitter-java 0.23.5"
        );
        assert_eq!(
            spec_for(Lang::Apex).unwrap().grammar_version,
            "tree-sitter-sfapex 3.0.2"
        );
        assert!(spec_for(Lang::Apex).unwrap().fold_case);
        assert!(!spec_for(Lang::Java).unwrap().fold_case);
    }
}
