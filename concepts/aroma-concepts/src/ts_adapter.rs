//! tree-sitter CST -> `Raw` tree. This is the only language-dependent step (paper
//! section 3.1: "this process ... requires knowledge about the grammar and static
//! scoping rules of the language").
//!
//! Keyword tokens (paper Def. 1) are anonymous tree-sitter nodes plus named leaves the
//! rules do not classify as identifiers/literals. Non-keyword tokens (Def. 2) are the
//! named identifier and literal leaves. Comments (`is_extra`) never become children,
//! so they cannot shift child indices.
//!
//! NOTE: tree-sitter groups nodes differently from ANTLR (e.g. `local_variable_declaration`
//! and `method_invocation` are flat), so labels and child indices differ from the
//! reference until a "virtual ANTLR" normaliser regroups those kinds.

use crate::spt::{Leaf, Raw, Spt};
use std::ops::Range;
use tree_sitter::{Node, Tree};

pub trait LanguageRules {
    /// Named leaf kinds that are non-keyword tokens (identifiers, literals).
    fn is_leaf_kind(&self, kind: &str) -> bool;
    /// Kinds whose whole text is one token even if the grammar gives them children
    /// (string literals with escape sequences, text blocks).
    fn is_atomic(&self, kind: &str) -> bool;
    /// Does this identifier occurrence denote a local variable (`#VAR` candidate)?
    /// `text` is the token after case folding.
    fn is_local_var(&self, node: Node<'_>, text: &str) -> bool;
    /// Indexing-unit boundary (method, trigger, LWC handler, flow element ...).
    fn is_unit(&self, node: Node<'_>) -> bool;
    /// Apex/SOQL are case-insensitive: fold identifiers and keywords, never strings.
    fn fold_case(&self) -> bool {
        false
    }
}

pub fn to_raw(node: Node<'_>, src: &[u8], rules: &dyn LanguageRules) -> Option<Raw> {
    let kind = node.kind();
    if node.is_extra() || kind.ends_with("comment") {
        return None;
    }
    if node.child_count() == 0 || rules.is_atomic(kind) {
        let text = node.utf8_text(src).ok()?;
        if text.is_empty() {
            return None; // MISSING nodes are zero-width: never invent evidence
        }
        let fold = rules.fold_case() && !kind.contains("string");
        let text = if fold { text.to_ascii_lowercase() } else { text.to_owned() };
        let is_leaf = node.is_named() && (rules.is_leaf_kind(kind) || rules.is_atomic(kind));
        return Some(if is_leaf {
            let is_var = rules.is_local_var(node, &text);
            Raw::Leaf(Leaf {
                text: text.into(),
                is_var,
                bytes: node.byte_range(),
                line: node.start_position().row as u32 + 1,
            })
        } else {
            Raw::Keyword(text)
        });
    }
    let mut cursor = node.walk();
    let children: Vec<Raw> = node
        .children(&mut cursor)
        .filter_map(|c| to_raw(c, src, rules))
        .collect();
    Some(Raw::List(children))
}

/// One SPT per unit body, in source order. Nested units (anonymous classes) are also
/// emitted, matching the reference which logs every methodDeclaration it visits.
pub fn units(tree: &Tree, src: &[u8], rules: &dyn LanguageRules) -> Vec<(Range<usize>, Spt)> {
    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        if rules.is_unit(n) {
            if let Some(spt) = n
                .child_by_field_name("body")
                .and_then(|b| to_raw(b, src, rules))
                .and_then(Spt::build)
            {
                out.push((n.byte_range(), spt));
            }
        }
        let mut c = n.walk();
        stack.extend(n.children(&mut c));
    }
    out.sort_by_key(|(r, _)| r.start);
    out
}

/// Java rules approximating the reference's syntactic variable marking
/// (ConvertJava.java:41-43: variableDeclaratorId, primary, catchClause, lambdaParameters).
/// The upper-case exemption (similar.py:287) is applied by the featurizer.
pub struct Java;

impl LanguageRules for Java {
    fn is_leaf_kind(&self, k: &str) -> bool {
        matches!(
            k,
            "identifier"
                | "type_identifier"
                | "decimal_integer_literal"
                | "hex_integer_literal"
                | "octal_integer_literal"
                | "binary_integer_literal"
                | "decimal_floating_point_literal"
                | "hex_floating_point_literal"
                | "character_literal"
                | "string_literal"
                | "true"
                | "false"
        ) // `null_literal` stays a keyword: ANTLR's display name for it is the literal 'null'
    }

    fn is_atomic(&self, k: &str) -> bool {
        matches!(k, "string_literal" | "character_literal" | "text_block")
    }

    fn is_local_var(&self, n: Node<'_>, _text: &str) -> bool {
        if n.kind() != "identifier" {
            return false;
        }
        let Some(p) = n.parent() else { return false };
        match p.kind() {
            "variable_declarator" | "formal_parameter" | "catch_formal_parameter"
            | "inferred_parameters" | "lambda_expression" | "enhanced_for_statement"
            | "resource" => true,
            // receiver is a variable, method/field name is not
            "method_invocation" | "field_access" => p.child_by_field_name("object") == Some(n),
            "method_declaration" | "class_declaration" | "interface_declaration"
            | "enum_declaration" | "annotation" | "marker_annotation" | "scoped_identifier"
            | "labeled_statement" | "break_statement" | "continue_statement"
            | "method_reference" => false,
            _ => true, // bare identifier used as an expression operand (`primary`)
        }
    }

    fn is_unit(&self, n: Node<'_>) -> bool {
        n.kind() == "method_declaration" && n.child_by_field_name("body").is_some()
    }
}

/// Apex rules for tree-sitter-sfapex 3.0.2 (`apex`), implementing plan decision D18:
/// one context-free `#VAR` rule shared by corpus units and partial queries.
///
/// An identifier is a local variable iff it is a declared name (declarator, parameter,
/// for-each variable) or it sits in expression/receiver/bind position and is not a known
/// global. Known globals = Apex standard namespaces + common sObjects + `__c`-style names +
/// every type declared in the indexed repository (so static receivers such as
/// `AccountService.doWork()` keep their name). Corpus and queries see the same set.
/// Identifiers are case-folded before classification; string literals never are.
/// Inline SOQL arrives as a `query_expression` subtree: field and object names sit under
/// `field_identifier` / `storage_identifier` and stay literal; `:bind` expressions are
/// ordinary expressions, so bound locals join usage chains.
pub struct Apex {
    pub globals: std::collections::HashSet<String>,
}

pub const APEX_BUILTIN_GLOBALS: &[&str] = &[
    "system", "database", "schema", "trigger", "test", "limits", "json", "jsonparser", "string",
    "integer", "long", "decimal", "double", "boolean", "date", "datetime", "time", "id", "blob",
    "math", "crypto", "encodingutil", "userinfo", "label", "apexpages", "messaging", "http",
    "httprequest", "httpresponse", "restcontext", "restrequest", "restresponse", "url", "type",
    "eventbus", "auth", "approval", "search", "sobject", "sobjecttype", "list", "set", "map",
    "object", "exception", "dmlexception", "queryexception", "pattern", "matcher", "flow",
    "process", "quiddity", "request", "security", "accesstype", "connectapi", "cache",
    "featuremanagement", "site", "network", "logginglevel", "statuscode",
    // common standard sObjects
    "account", "contact", "opportunity", "lead", "case", "user", "task", "event", "campaign",
    "campaignmember", "contract", "order", "orderitem", "product2", "pricebook2", "pricebookentry",
    "opportunitylineitem", "opportunitycontactrole", "recordtype", "profile", "group",
    "groupmember", "organization", "contentversion", "contentdocument", "contentdocumentlink",
    "attachment", "note", "emailmessage", "asyncapexjob", "crontrigger", "permissionset",
    "permissionsetassignment", "userrole", "accountcontactrelation", "address",
];

impl Apex {
    pub fn new(repo_types: impl IntoIterator<Item = String>) -> Self {
        let mut globals: std::collections::HashSet<String> =
            APEX_BUILTIN_GLOBALS.iter().map(|s| s.to_string()).collect();
        globals.extend(repo_types.into_iter().map(|t| t.to_ascii_lowercase()));
        Apex { globals }
    }

    fn is_global(&self, text: &str) -> bool {
        const SUFFIXES: &[&str] = &["__c", "__r", "__mdt", "__e", "__x", "__share", "__history"];
        self.globals.contains(text) || SUFFIXES.iter().any(|s| text.ends_with(s))
    }

    /// Declared type names in one file (first indexing pass).
    pub fn declared_types(tree: &Tree, src: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![tree.root_node()];
        while let Some(n) = stack.pop() {
            if matches!(n.kind(), "class_declaration" | "interface_declaration" | "enum_declaration") {
                if let Some(name) = n.child_by_field_name("name").and_then(|x| x.utf8_text(src).ok()) {
                    out.push(name.to_ascii_lowercase());
                }
            }
            let mut c = n.walk();
            stack.extend(n.children(&mut c));
        }
        out
    }
}

impl LanguageRules for Apex {
    fn is_leaf_kind(&self, k: &str) -> bool {
        matches!(
            k,
            "identifier" | "type_identifier" | "string_literal" | "int" | "decimal"
                | "decimal_floating_point_literal" | "boolean" | "currency_literal" | "date"
                | "date_time"
        )
    }

    fn is_atomic(&self, k: &str) -> bool {
        matches!(k, "string_literal" | "multi_line_string_literal")
    }

    fn is_local_var(&self, n: Node<'_>, text: &str) -> bool {
        if n.kind() != "identifier" {
            return false;
        }
        let Some(p) = n.parent() else { return false };
        let is_field = |f: &str| p.child_by_field_name(f) == Some(n);
        match p.kind() {
            "variable_declarator" | "formal_parameter" | "enhanced_for_statement" if is_field("name") => true,
            "method_invocation" if is_field("name") => false,
            "field_access" if is_field("field") => false,
            "field_identifier" | "storage_identifier" | "dotted_identifier" | "class_declaration"
            | "interface_declaration" | "enum_declaration" | "trigger_declaration"
            | "method_declaration" | "constructor_declaration" | "annotation" | "accessor_declaration"
            | "scoped_type_identifier" | "generic_type" | "alias_expression" | "enum_constant"
            | "explicit_constructor_invocation" | "element_value_pair" => false,
            _ => !self.is_global(text), // expression, receiver or `:bind` position
        }
    }

    fn is_unit(&self, n: Node<'_>) -> bool {
        matches!(
            n.kind(),
            "method_declaration" | "constructor_declaration" | "trigger_declaration" | "accessor_declaration"
        ) && n.child_by_field_name("body").is_some()
    }

    fn fold_case(&self) -> bool {
        true
    }
}
