//! Java (tree-sitter-java 0.23.5) tables, refined by T12 against the grammar's
//! `node-types.json` and the virtual-ANTLR normalizer (`crate::normalize::java`).
//!
//! Classification (stage 3) reads these tables on the *normalized* tree. The normalizer
//! keeps every identifier under its original parent kind (anchors), and adds two
//! declaration sites the T00 tables lacked: lambda parameters (field `name` under
//! `inferred_parameters`) and pattern variables (`x instanceof Foo f`, field `name`).
//! Java units stay `method_declaration` only, like the reference (`ConvertJava` dumps
//! `methodBody` records only).

use super::LangSpec;
use crate::snippet::{Shape, Wrapper};
use aroma_core::lang::Lang;
use aroma_core::unit::UnitKind;
use std::path::Path;

fn language() -> tree_sitter::Language {
    tree_sitter_java::LANGUAGE.into()
}

/// A path component `test`/`tests`, or a file stem starting with `Test` or ending with
/// `Test`/`Tests`/`IT`.
fn is_test_path(path: &Path) -> bool {
    let dir_hit = path.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s.eq_ignore_ascii_case("test") || s.eq_ignore_ascii_case("tests")
    });
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    dir_hit
        || stem.starts_with("Test")
        || stem.ends_with("Test")
        || stem.ends_with("Tests")
        || stem.ends_with("IT")
}

/// `java.lang` types plus the JDK classes most often used as static receivers
/// (`Collections.sort(..)`, `Collectors.toList()`): never `#VAR` in receiver position
/// under the D18 rule. Case-sensitive (Java does not fold case).
const BUILTIN_GLOBALS: &[&str] = &[
    // java.lang
    "Boolean",
    "Byte",
    "Character",
    "Class",
    "Double",
    "Enum",
    "Error",
    "Exception",
    "Float",
    "Integer",
    "Iterable",
    "Long",
    "Math",
    "Number",
    "Object",
    "Runtime",
    "RuntimeException",
    "Short",
    "StrictMath",
    "String",
    "StringBuffer",
    "StringBuilder",
    "System",
    "Thread",
    "Throwable",
    "Void",
    // java.util, java.util.stream, java.util.concurrent, java.nio.file, java.time
    "ArrayList",
    "Arrays",
    "Collections",
    "Collectors",
    "CompletableFuture",
    "Duration",
    "Executors",
    "Files",
    "HashMap",
    "HashSet",
    "Instant",
    "IntStream",
    "LinkedHashMap",
    "LinkedList",
    "List",
    "LocalDate",
    "LocalDateTime",
    "Locale",
    "Map",
    "Objects",
    "Optional",
    "Path",
    "Paths",
    "Pattern",
    "Set",
    "Stream",
    "ThreadLocalRandom",
    "TimeUnit",
    "TreeMap",
    "TreeSet",
    "UUID",
];

pub static SPEC: LangSpec = LangSpec {
    lang: Lang::Java,
    language,
    preprocess: super::identity_preprocess,
    unit: crate::table::table_unit,
    feature_config: super::identity_features,
    term_rule: aroma_core::lexical::TermRule::Identifier,
    omission_marker: aroma_core::render::OMITTED_SLASH,
    repair: crate::snippet::Repair::Brackets,
    wrappers: &WRAPPERS,
    matches: |rel| super::has_extension(rel, &["java"]),
    // `null_literal` stays a keyword: ANTLR's display name for it is the literal 'null'.
    leaf_kinds: &[
        "identifier",
        "type_identifier",
        "decimal_integer_literal",
        "hex_integer_literal",
        "octal_integer_literal",
        "binary_integer_literal",
        "decimal_floating_point_literal",
        "hex_floating_point_literal",
        "character_literal",
        "string_literal",
        "true",
        "false",
    ],
    // string_literal covers text blocks (`multiline_string_fragment` children).
    atomic_kinds: &["string_literal", "character_literal"],
    comment_kinds: &["line_comment", "block_comment"],
    identifier_kind: "identifier",
    units: &[("method_declaration", UnitKind::Method)],
    body_field: "body",
    markup_bindings: &[],
    js_scope_kinds: &[],
    function_scope_kinds: &[],
    js_snippets: None,
    decl_name_parents: &[
        "variable_declarator",
        "formal_parameter",
        "catch_formal_parameter",
        "enhanced_for_statement",
        "resource",
        "inferred_parameters",
        "instanceof_expression",
    ],
    non_var_parents: &[
        "method_declaration",
        "class_declaration",
        "interface_declaration",
        "enum_declaration",
        "record_declaration",
        "annotation",
        "marker_annotation",
        "scoped_identifier",
        "labeled_statement",
        "break_statement",
        "continue_statement",
        "method_reference",
        "enum_constant",
    ],
    non_var_fields: &[
        ("method_invocation", "name"),
        ("field_access", "field"),
        ("element_value_pair", "key"),
    ],
    type_decl_kinds: &[
        "class_declaration",
        "interface_declaration",
        "enum_declaration",
        "record_declaration",
        "annotation_type_declaration",
    ],
    builtin_globals: BUILTIN_GLOBALS,
    global_suffixes: &[],
    fold_case: false,
    adapter_version: "java-1",
    grammar_version: "tree-sitter-java 0.23.5",
    normalizer: &crate::normalize::java::NORMALIZER,
    is_test_path,
};

const WRAPPERS: [Wrapper; 3] = [
    Wrapper {
        name: "statements",
        shape: Shape::Statements,
        prefix: "class __AromaQ { void __aromaQ() {\n",
        suffix: "\n} }",
    },
    Wrapper {
        name: "member",
        shape: Shape::Member,
        prefix: "class __AromaQ {\n",
        suffix: "\n}",
    },
    Wrapper {
        name: "expression",
        shape: Shape::Expression,
        prefix: "class __AromaQ { Object __aromaQ = (\n",
        suffix: "\n); }",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decl_and_non_var_tables_name_real_fields() {
        let language = (SPEC.language)();
        // the D18 table rows added by T12 use fields the grammar really has
        for (kind, field) in [
            ("instanceof_expression", "name"),
            ("element_value_pair", "key"),
            ("enum_constant", "name"),
        ] {
            assert_ne!(language.id_for_node_kind(kind, true), 0, "{kind}");
            assert!(language.field_id_for_name(field).is_some(), "{field}");
        }
    }

    #[test]
    fn builtins_are_unique_and_capitalized() {
        let mut seen = std::collections::BTreeSet::new();
        for g in BUILTIN_GLOBALS {
            assert!(seen.insert(*g), "duplicate builtin {g}");
            assert!(
                g.chars().next().is_some_and(|c| c.is_ascii_uppercase()),
                "{g}"
            );
        }
    }

    #[test]
    fn adapter_version_is_bumped_for_the_normalizer() {
        assert_eq!(SPEC.adapter_version, "java-1");
        assert_eq!(SPEC.units, &[("method_declaration", UnitKind::Method)]);
    }
}
