//! Apex (tree-sitter-sfapex 3.0.2, `apex` grammar) tables, refined by T13 against
//! `apex/src/node-types.json`. Identifiers and keywords are ASCII case-folded before
//! classification, so every global here is lower-case. Inline SOQL/SOSL names sit under
//! `field_identifier`, `storage_identifier`, `dotted_identifier`, `field_list`,
//! `type_of_clause` or `sobject_return` (never `#VAR`); SOQL functions (`COUNT(Id)`) keep
//! their name in field `function_name`; `:bind` expressions are ordinary expressions.
//! `boolean` is atomic so `true`/`false` are one leaf token (the grammar gives the node
//! an anonymous keyword child), like Java's literals.

use super::LangSpec;
use crate::snippet::{Shape, Wrapper};
use aroma_core::lang::Lang;
use aroma_core::unit::UnitKind;
use std::path::Path;

fn language() -> tree_sitter::Language {
    tree_sitter_sfapex::apex::LANGUAGE.into()
}

/// Any path containing `test`, ASCII case-insensitive.
fn is_test_path(path: &Path) -> bool {
    path.to_string_lossy().to_ascii_lowercase().contains("test")
}

/// Apex standard namespaces and common standard sObjects (lower-case).
pub const APEX_BUILTIN_GLOBALS: &[&str] = &[
    "system",
    "database",
    "schema",
    "trigger",
    "test",
    "limits",
    "json",
    "jsonparser",
    "string",
    "integer",
    "long",
    "decimal",
    "double",
    "boolean",
    "date",
    "datetime",
    "time",
    "id",
    "blob",
    "math",
    "crypto",
    "encodingutil",
    "userinfo",
    "label",
    "apexpages",
    "messaging",
    "http",
    "httprequest",
    "httpresponse",
    "restcontext",
    "restrequest",
    "restresponse",
    "url",
    "type",
    "eventbus",
    "auth",
    "approval",
    "search",
    "sobject",
    "sobjecttype",
    "list",
    "set",
    "map",
    "object",
    "exception",
    "dmlexception",
    "queryexception",
    "pattern",
    "matcher",
    "flow",
    "process",
    "quiddity",
    "request",
    "security",
    "accesstype",
    "connectapi",
    "cache",
    "featuremanagement",
    "site",
    "network",
    "logginglevel",
    "statuscode",
    // common standard sObjects
    "account",
    "contact",
    "opportunity",
    "lead",
    "case",
    "user",
    "task",
    "event",
    "campaign",
    "campaignmember",
    "contract",
    "order",
    "orderitem",
    "product2",
    "pricebook2",
    "pricebookentry",
    "opportunitylineitem",
    "opportunitycontactrole",
    "recordtype",
    "profile",
    "group",
    "groupmember",
    "organization",
    "contentversion",
    "contentdocument",
    "contentdocumentlink",
    "attachment",
    "note",
    "emailmessage",
    "asyncapexjob",
    "crontrigger",
    "permissionset",
    "permissionsetassignment",
    "userrole",
    "accountcontactrelation",
    "address",
];

pub static SPEC: LangSpec = LangSpec {
    lang: Lang::Apex,
    language,
    preprocess: super::identity_preprocess,
    unit: crate::table::table_unit,
    feature_config: super::identity_features,
    term_rule: aroma_core::lexical::TermRule::Identifier,
    omission_marker: aroma_core::render::OMITTED_SLASH,
    repair: crate::snippet::Repair::Brackets,
    wrappers: &WRAPPERS,
    matches: |rel| super::has_extension(rel, &["cls", "trigger", "apex"]),
    leaf_kinds: &[
        "identifier",
        "type_identifier",
        "string_literal",
        "int",
        "decimal",
        "decimal_floating_point_literal",
        "boolean",
        "currency_literal",
        "date",
        "date_time",
    ],
    atomic_kinds: &["string_literal", "multi_line_string_literal", "boolean"],
    comment_kinds: &["line_comment", "block_comment"],
    identifier_kind: "identifier",
    units: &[
        ("method_declaration", UnitKind::Method),
        ("constructor_declaration", UnitKind::Constructor),
        ("trigger_declaration", UnitKind::Trigger),
        ("accessor_declaration", UnitKind::Accessor),
    ],
    body_field: "body",
    markup_bindings: &[],
    js_scope_kinds: &[],
    function_scope_kinds: &[],
    js_snippets: None,
    decl_name_parents: &[
        "variable_declarator",
        "formal_parameter",
        "enhanced_for_statement",
        "when_sobject_type",
    ],
    non_var_parents: &[
        "field_identifier",
        "storage_identifier",
        "dotted_identifier",
        "class_declaration",
        "interface_declaration",
        "enum_declaration",
        "trigger_declaration",
        "method_declaration",
        "constructor_declaration",
        "annotation",
        "accessor_declaration",
        "scoped_type_identifier",
        "generic_type",
        "alias_expression",
        "enum_constant",
        "explicit_constructor_invocation",
        "field_list",
        "type_of_clause",
        "sobject_return",
    ],
    non_var_fields: &[
        ("method_invocation", "name"),
        ("field_access", "field"),
        ("function_expression", "function_name"),
        ("count_expression", "function_name"),
        ("geo_location_type", "function_name"),
        ("annotation_key_value", "key"),
    ],
    type_decl_kinds: &[
        "class_declaration",
        "interface_declaration",
        "enum_declaration",
    ],
    builtin_globals: APEX_BUILTIN_GLOBALS,
    global_suffixes: &["__c", "__r", "__mdt", "__e", "__x", "__share", "__history"],
    fold_case: true,
    adapter_version: "apex-1",
    grammar_version: "tree-sitter-sfapex 3.0.2",
    normalizer: &crate::normalize::apex::NORMALIZER,
    is_test_path,
};

const WRAPPERS: [Wrapper; 3] = [
    Wrapper {
        name: "statements",
        shape: Shape::Statements,
        prefix: "public class __AromaQ { public void __aromaQ() {\n",
        suffix: "\n} }",
    },
    Wrapper {
        name: "member",
        shape: Shape::Member,
        prefix: "public class __AromaQ {\n",
        suffix: "\n}",
    },
    Wrapper {
        name: "expression",
        shape: Shape::Expression,
        prefix: "public class __AromaQ { public Object __aromaQ = (\n",
        suffix: "\n); }",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refined_rows_name_real_kinds_and_fields() {
        let language = (SPEC.language)();
        for (kind, field) in SPEC.non_var_fields {
            assert_ne!(language.id_for_node_kind(kind, true), 0, "{kind}");
            assert!(language.field_id_for_name(field).is_some(), "{field}");
        }
        for kind in [
            "field_list",
            "type_of_clause",
            "sobject_return",
            "boolean",
            "when_sobject_type",
        ] {
            assert_ne!(language.id_for_node_kind(kind, true), 0, "{kind}");
        }
    }

    #[test]
    fn units_suffixes_and_versions() {
        assert_eq!(
            SPEC.units,
            &[
                ("method_declaration", UnitKind::Method),
                ("constructor_declaration", UnitKind::Constructor),
                ("trigger_declaration", UnitKind::Trigger),
                ("accessor_declaration", UnitKind::Accessor),
            ]
        );
        assert_eq!(
            SPEC.global_suffixes,
            &["__c", "__r", "__mdt", "__e", "__x", "__share", "__history"]
        );
        assert_eq!(SPEC.builtin_globals, APEX_BUILTIN_GLOBALS);
        assert_eq!(SPEC.adapter_version, "apex-1");
        assert!(SPEC.fold_case);
    }
}
