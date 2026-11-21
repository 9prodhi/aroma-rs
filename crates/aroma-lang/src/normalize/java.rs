//! Java normalizer (stage 2, D21): "virtual ANTLR" regrouping of tree-sitter-java 0.23.5
//! trees so the SPT has the grouping of the reference's ANTLR Java 8 parse tree
//! (internal research note, not published). Only grouping matters: labels are token text plus `#`, and
//! collapse rules A/B remove single-child wrappers and leafless groups, so a virtual
//! node is added only where ANTLR has a rule node with two or more children.
//!
//! Rewrites (all bottom-up, explicit stack, deterministic and context-free):
//!
//! | tree-sitter | ANTLR shape produced |
//! |---|---|
//! | `local_variable_declaration` | `[localVariableDeclaration[mods… type variableDeclarators] ';']` |
//! | `method_invocation` | `[object '.' methodCall[name '(' expressionList ')']]`, bare call `[name '(' … ')']` |
//! | `argument_list` | `['(' expressionList ')']`; inlined into calls, kept as `arguments` elsewhere |
//! | `for_statement` | `['for' '(' forControl[forInit ';' cond ';' forUpdate] ')' body]` |
//! | `enhanced_for_statement` | `['for' '(' enhancedForControl[mods… type name ':' value] ')' body]` |
//! | `catch_clause` | `catch_formal_parameter` inlined: `['catch' '(' mods… catchType name ')' block]` |
//! | `modifiers` | inlined; members hoist them into `classBodyDeclaration[mods… member]` |
//! | `generic_type`, `scoped_type_identifier`, `scoped_identifier` | one flat `IDENT typeArgs? ('.' IDENT typeArgs?)*` list |
//! | `>>`, `>>>`, `<<` tokens | split into one-character `>` / `<` tokens (ANTLR has no shift tokens) |
//! | `object_creation_expression` | `['new' creator[createdName classCreatorRest[arguments classBody?]]]` |
//! | `array_creation_expression` | `['new' creator[createdName arrayCreatorRest[…]]]` |
//! | `dimensions`, `throws`, `superclass`, `super_interfaces`, `annotation_argument_list` | inlined |
//! | `switch_block` | inlined into the switch; label-only groups merge into the next group; `switch_label ':'` paired |
//! | `formal_parameters`, `resource_specification`, `throws` lists | grouped as `formalParameterList`, `resources`, `qualifiedNameList` |
//!
//! Virtual nodes are named after the ANTLR rule they emulate (`methodCall`, `forControl`,
//! …). Those names are not grammar kinds, so they never occur in `LangSpec` tables or
//! the classifier's scope kinds. Stage 3 classifies an identifier by its enclosing node
//! kind and field, so when a regrouping moves an identifier out of a parent that the
//! Java tables list, the identifier is wrapped in a single-child *anchor* node of the
//! original kind (`variable_declarator` for the enhanced-for name, because
//! `enhanced_for_statement` opens a scope). Collapse rule A removes every anchor, so the
//! SPT is unchanged while `#VAR` marks and `SymbolId`s stay exactly as without
//! regrouping. Two deliberate classification refinements: lambda parameters become
//! declarations (field `name` under `inferred_parameters`) so their uses get a
//! `SymbolId`, and a bare identifier receiver of a method reference (`list::add`) is
//! anchored as a virtual `primary` (expression position, like ANTLR's `primary`).
use super::Normalizer;
use crate::kraw::KRaw;

pub struct JavaNormalizer;

pub static NORMALIZER: JavaNormalizer = JavaNormalizer;

impl Normalizer for JavaNormalizer {
    fn normalize(&self, node: KRaw) -> KRaw {
        bottom_up(node)
    }
}

/// One open node of the bottom-up walk.
struct Frame {
    kind: &'static str,
    field: Option<&'static str>,
    pending: std::vec::IntoIter<KRaw>,
    out: Vec<KRaw>,
}

/// Rewrites every node after its children (explicit stack: input nesting never grows the
/// call stack). The root keeps its field.
fn bottom_up(root: KRaw) -> KRaw {
    let (kind, field, children) = match root {
        KRaw::Node {
            kind,
            field,
            children,
        } => (kind, field, children),
        token @ KRaw::Token { .. } => return token,
    };
    let mut stack = vec![Frame {
        kind,
        field,
        pending: children.into_iter(),
        out: Vec::new(),
    }];
    loop {
        let next = stack
            .last_mut()
            .expect("the root frame stays until it returns")
            .pending
            .next();
        match next {
            Some(KRaw::Node {
                kind,
                field,
                children,
            }) => stack.push(Frame {
                kind,
                field,
                pending: children.into_iter(),
                out: Vec::new(),
            }),
            Some(token) => {
                let top = stack.last_mut().expect("non-empty");
                push_token(&mut top.out, token);
            }
            None => {
                let done = stack.pop().expect("non-empty");
                let node = rewrite(done.kind, done.field, done.out);
                match stack.last_mut() {
                    Some(parent) => parent.out.push(node),
                    None => return node,
                }
            }
        }
    }
}

/// ANTLR's Java lexer has no `<<`, `>>` or `>>>` token (`expression ('<' '<' | '>' '>'
/// '>' | '>' '>') expression`), so shift operators become one token per character. The
/// label text is unchanged; the right operand's child index moves like the reference's.
fn push_token(out: &mut Vec<KRaw>, token: KRaw) {
    match token {
        KRaw::Token {
            named: false,
            text,
            bytes,
            line,
            field,
            ..
        } if matches!(text.as_str(), "<<" | ">>" | ">>>") => {
            for (i, c) in text.char_indices() {
                let at = bytes.start + i as u32;
                out.push(KRaw::Token {
                    kind: if c == '<' { "<" } else { ">" },
                    named: false,
                    text: c.to_string(),
                    bytes: at..at + 1,
                    line,
                    field,
                });
            }
        }
        other => out.push(other),
    }
}

fn node(kind: &'static str, children: Vec<KRaw>) -> KRaw {
    KRaw::Node {
        kind,
        field: None,
        children,
    }
}

/// Single-child wrapper that keeps an identifier's original parent kind for stage 3;
/// collapse rule A removes it from the SPT.
fn anchor(kind: &'static str, child: KRaw) -> KRaw {
    node(kind, vec![child])
}

fn is_tok(k: &KRaw, text: &str) -> bool {
    matches!(k, KRaw::Token { named: false, text: t, .. } if t == text)
}

fn is_name(k: &KRaw) -> bool {
    matches!(
        k,
        KRaw::Token {
            named: true,
            field: Some("name"),
            ..
        }
    )
}

fn children_of(k: KRaw) -> Vec<KRaw> {
    match k {
        KRaw::Node { children, .. } => children,
        token @ KRaw::Token { .. } => vec![token],
    }
}

/// Replaces every direct child node whose kind is in `kinds` by its children.
fn splice(children: Vec<KRaw>, kinds: &[&str]) -> Vec<KRaw> {
    let mut out = Vec::with_capacity(children.len());
    for c in children {
        match c {
            KRaw::Node { kind, children, .. } if kinds.contains(&kind) => out.extend(children),
            other => out.push(other),
        }
    }
    out
}

/// ANTLR `variableDeclaratorId: IDENTIFIER ('[' ']')*`: the `name` token and a following
/// `dimensions` node become one `variableDeclaratorId` list. When `moved` (the name
/// leaves its stage-1 parent `parent`), a plain name is anchored under `parent`.
fn declarator_id(children: Vec<KRaw>, parent: &'static str, moved: bool) -> Vec<KRaw> {
    let mut out: Vec<KRaw> = Vec::with_capacity(children.len());
    let mut iter = children.into_iter().peekable();
    while let Some(c) = iter.next() {
        if !is_name(&c) {
            out.push(c);
            continue;
        }
        if iter.peek().is_some_and(|n| n.kind() == "dimensions") {
            let dims = iter.next().map(children_of).unwrap_or_default();
            let mut id = vec![anchor(parent, c)];
            id.extend(dims);
            out.push(node("variableDeclaratorId", id));
        } else if moved {
            out.push(anchor(parent, c));
        } else {
            out.push(c);
        }
    }
    out
}

/// `variableDeclarators: variableDeclarator (',' variableDeclarator)*`: everything from
/// the first declarator on becomes one list.
fn group_declarators(mut items: Vec<KRaw>) -> Vec<KRaw> {
    match items.iter().position(|k| k.kind() == "variable_declarator") {
        Some(first) => {
            let declarators = items.split_off(first);
            items.push(node("variableDeclarators", declarators));
            items
        }
        None => items,
    }
}

/// `'(' X ')'` → `'(' kind[X] ')'` when X is non-empty (a single item collapses again).
fn group_inside_parens(children: Vec<KRaw>, kind: &'static str) -> Vec<KRaw> {
    let n = children.len();
    if n < 3 || !is_tok(&children[0], "(") || !is_tok(&children[n - 1], ")") {
        return children;
    }
    let mut it = children.into_iter();
    let open = it.next().expect("n >= 3");
    let mut inner: Vec<KRaw> = it.collect();
    let close = inner.pop().expect("n >= 3");
    vec![open, node(kind, inner), close]
}

fn rewrite(kind: &'static str, field: Option<&'static str>, children: Vec<KRaw>) -> KRaw {
    let children = match kind {
        // Members: ANTLR `classBodyDeclaration: modifier* memberDeclaration`.
        "method_declaration"
        | "constructor_declaration"
        | "field_declaration"
        | "constant_declaration"
        | "class_declaration"
        | "interface_declaration"
        | "enum_declaration"
        | "record_declaration"
        | "annotation_type_declaration" => return member(kind, field, children),
        "local_variable_declaration" => local_variable_declaration(children),
        "variable_declarator" => declarator_id(children, "variable_declarator", false),
        "formal_parameter" => declarator_id(splice(children, &["modifiers"]), kind, false),
        "spread_parameter" => splice(children, &["modifiers"]),
        "resource" => declarator_id(splice(children, &["modifiers"]), kind, false),
        "formal_parameters" => group_inside_parens(children, "formalParameterList"),
        "argument_list" => group_inside_parens(children, "expressionList"),
        "annotation_argument_list" => group_inside_parens(children, "elementValuePairs"),
        "annotation" => splice(children, &["annotation_argument_list"]),
        "resource_specification" => resource_specification(children),
        "throws" => throws(children),
        "method_invocation" => method_invocation(children),
        "explicit_constructor_invocation" => explicit_constructor_invocation(children),
        "object_creation_expression" => object_creation(children),
        "array_creation_expression" => array_creation(children),
        "array_type" => splice(children, &["dimensions"]),
        "generic_type" | "scoped_type_identifier" => {
            splice(children, &["generic_type", "scoped_type_identifier"])
        }
        "scoped_identifier" => splice(children, &["scoped_identifier"]),
        "for_statement" => for_statement(children),
        "enhanced_for_statement" => enhanced_for(children),
        "catch_clause" => catch_clause(children),
        "switch_block_statement_group" => switch_group(children),
        "switch_block" => switch_block(children),
        "switch_expression" => splice(children, &["switch_block"]),
        "lambda_expression" => lambda(children),
        "inferred_parameters" => children
            .into_iter()
            .map(|c| match c {
                t @ KRaw::Token { named: true, .. } => t.with_field(Some("name")),
                other => other,
            })
            .collect(),
        "method_reference" => method_reference(children),
        _ => children,
    };
    KRaw::Node {
        kind,
        field,
        children,
    }
}

/// Hoists `modifiers` out of a member into `classBodyDeclaration[mods… member]` and
/// applies the member's own ANTLR shape (`dimensions`/`throws`/`superclass`/
/// `super_interfaces`/`extends_interfaces` inlined, `genericMethodDeclaration`,
/// `fieldDeclaration` with its declarators grouped).
fn member(kind: &'static str, field: Option<&'static str>, children: Vec<KRaw>) -> KRaw {
    let mut mods = Vec::new();
    let mut rest = Vec::with_capacity(children.len());
    for c in children {
        match c {
            KRaw::Node {
                kind: "modifiers",
                children,
                ..
            } => mods.extend(children),
            other => rest.push(other),
        }
    }
    let rest = splice(
        rest,
        &[
            "dimensions",
            "throws",
            "superclass",
            "super_interfaces",
            "extends_interfaces",
        ],
    );
    let decl = match kind {
        "field_declaration" | "constant_declaration" => node(kind, group_declarators(rest)),
        "method_declaration" | "constructor_declaration"
            if rest.first().is_some_and(|c| c.kind() == "type_parameters") =>
        {
            let mut it = rest.into_iter();
            let params = it.next().expect("checked above");
            let name = if kind == "method_declaration" {
                "genericMethodDeclaration"
            } else {
                "genericConstructorDeclaration"
            };
            node(name, vec![params, node(kind, it.collect())])
        }
        _ => node(kind, rest),
    };
    if mods.is_empty() {
        return decl.with_field(field);
    }
    mods.push(decl);
    KRaw::Node {
        kind: "classBodyDeclaration",
        field,
        children: mods,
    }
}

/// ANTLR `blockStatement: localVariableDeclaration ';'` with
/// `localVariableDeclaration: variableModifier* typeType variableDeclarators`.
fn local_variable_declaration(children: Vec<KRaw>) -> Vec<KRaw> {
    let mut items = splice(children, &["modifiers"]);
    let semi = match items.last() {
        Some(k) if is_tok(k, ";") => items.pop(),
        _ => None,
    };
    let mut out = vec![node("localVariableDeclaration", group_declarators(items))];
    out.extend(semi);
    out
}

/// ANTLR `resourceSpecification: '(' resources ';'? ')'`, `resources: resource (';'
/// resource)*`.
fn resource_specification(children: Vec<KRaw>) -> Vec<KRaw> {
    let n = children.len();
    if n < 3 || !is_tok(&children[0], "(") || !is_tok(&children[n - 1], ")") {
        return children;
    }
    let mut it = children.into_iter();
    let open = it.next().expect("n >= 3");
    let mut inner: Vec<KRaw> = it.collect();
    let close = inner.pop().expect("n >= 3");
    let trailing = match inner.last() {
        Some(k) if is_tok(k, ";") => inner.pop(),
        _ => None,
    };
    let mut out = vec![open, node("resources", inner)];
    out.extend(trailing);
    out.push(close);
    out
}

/// `throws T1, T2` → `'throws' qualifiedNameList[T1 ',' T2]` (the member inlines it).
fn throws(children: Vec<KRaw>) -> Vec<KRaw> {
    let mut it = children.into_iter();
    match it.next() {
        Some(kw) if is_tok(&kw, "throws") => vec![kw, node("qualifiedNameList", it.collect())],
        Some(other) => std::iter::once(other).chain(it).collect(),
        None => Vec::new(),
    }
}

/// ANTLR `expression '.' methodCall` with `methodCall: IDENTIFIER '(' expressionList? ')'`
/// (the call's own parentheses, not an `arguments` node), or `expression '.'
/// explicitGenericInvocation` when type arguments precede the name.
fn method_invocation(children: Vec<KRaw>) -> Vec<KRaw> {
    let Some(name_at) = children.iter().position(is_name) else {
        return children;
    };
    let mut prefix = children;
    let mut rest = prefix.split_off(name_at);
    let name = rest.remove(0);
    if prefix.is_empty() {
        // bare `methodCall`: the name stays a direct child of `method_invocation`
        let mut out = vec![name];
        out.extend(splice(rest, &["argument_list"]));
        return out;
    }
    let name = anchor("method_invocation", name);
    if prefix.last().is_some_and(|k| k.kind() == "type_arguments") {
        let targs = prefix.pop().expect("checked above");
        let mut suffix = vec![name];
        suffix.extend(rest);
        prefix.push(node(
            "explicitGenericInvocation",
            vec![targs, node("explicitGenericInvocationSuffix", suffix)],
        ));
        return prefix;
    }
    let mut call = vec![name];
    call.extend(splice(rest, &["argument_list"]));
    prefix.push(node("methodCall", call));
    prefix
}

/// `this(args);` / `super(args);` → `[methodCall[this '(' … ')'] ';']`.
fn explicit_constructor_invocation(children: Vec<KRaw>) -> Vec<KRaw> {
    let mut call = children;
    let semi = match call.last() {
        Some(k) if is_tok(k, ";") => call.pop(),
        _ => None,
    };
    let mut out = vec![node("methodCall", splice(call, &["argument_list"]))];
    out.extend(semi);
    out
}

/// ANTLR `NEW creator`, `creator: nonWildcardTypeArguments? createdName classCreatorRest`,
/// `classCreatorRest: arguments classBody?`; qualified `outer.new Inner()` is `expression
/// '.' NEW innerCreator` with `innerCreator: IDENTIFIER typeArgs? classCreatorRest`.
fn object_creation(children: Vec<KRaw>) -> Vec<KRaw> {
    let Some(new_at) = children.iter().position(|k| is_tok(k, "new")) else {
        return children;
    };
    let mut prefix = children;
    let after = prefix.split_off(new_at + 1);
    let mut head = Vec::new();
    let mut rest = Vec::new();
    for c in after {
        if matches!(c.kind(), "argument_list" | "class_body") || !rest.is_empty() {
            rest.push(c);
        } else {
            head.push(c);
        }
    }
    let class_rest = node("classCreatorRest", rest);
    if new_at == 0 {
        head.push(class_rest);
        prefix.push(node("creator", head));
    } else {
        let mut inner = splice(head, &["generic_type"]);
        inner.push(class_rest);
        prefix.push(node("innerCreator", inner));
    }
    prefix
}

/// ANTLR `NEW creator`, `creator: createdName arrayCreatorRest`, `arrayCreatorRest` = the
/// flat `[ expr ]… [ ]… arrayInitializer?` tail.
fn array_creation(children: Vec<KRaw>) -> Vec<KRaw> {
    let Some(new_at) = children.iter().position(|k| is_tok(k, "new")) else {
        return children;
    };
    let mut out = children;
    let after = out.split_off(new_at + 1);
    let mut head = Vec::new();
    let mut tail = Vec::new();
    for c in after {
        let starts_tail = matches!(
            c.kind(),
            "dimensions_expr" | "dimensions" | "array_initializer"
        );
        if starts_tail || !tail.is_empty() {
            tail.push(c);
        } else {
            head.push(c);
        }
    }
    head.push(node(
        "arrayCreatorRest",
        splice(tail, &["dimensions_expr", "dimensions"]),
    ));
    out.push(node("creator", head));
    out
}

/// Positions of the `(` after `for` and of the last `)`.
fn for_parens(children: &[KRaw]) -> Option<(usize, usize)> {
    let open = children.iter().position(|k| is_tok(k, "("))?;
    let close = children.iter().rposition(|k| is_tok(k, ")"))?;
    (close > open).then_some((open, close))
}

/// `forInit` / `forUpdate`: a lone `localVariableDeclaration`, or an `expressionList`.
fn for_part(items: Vec<KRaw>) -> Option<KRaw> {
    if items.is_empty() {
        return None;
    }
    if items.len() == 1 && items[0].kind() == "localVariableDeclaration" {
        return items.into_iter().next();
    }
    Some(node("expressionList", items))
}

/// Number of `;` separating the `for` header parts (tree-sitter's init declaration
/// carries the first one).
fn for_semicolons(inner: &[KRaw]) -> usize {
    inner
        .iter()
        .map(|c| match c {
            KRaw::Node {
                kind: "local_variable_declaration",
                children,
                ..
            } => children.iter().filter(|k| is_tok(k, ";")).count(),
            other => usize::from(is_tok(other, ";")),
        })
        .sum()
}

/// ANTLR `FOR '(' forControl ')' statement`, `forControl: forInit? ';' expression? ';'
/// expressionList?`. A damaged header (not exactly two `;`) keeps tree-sitter's grouping.
fn for_statement(children: Vec<KRaw>) -> Vec<KRaw> {
    let Some((open, close)) = for_parens(&children) else {
        return children;
    };
    if for_semicolons(&children[open + 1..close]) != 2 {
        return children;
    }
    let mut head = children;
    let mut inner = head.split_off(open + 1);
    let tail = inner.split_off(close - open - 1);
    let mut parts: [Vec<KRaw>; 3] = Default::default();
    let mut semis = Vec::with_capacity(2);
    for c in inner {
        let items = match c {
            KRaw::Node {
                kind: "local_variable_declaration",
                children,
                ..
            } => children,
            other => vec![other],
        };
        for k in items {
            if is_tok(&k, ";") {
                semis.push(k);
            } else {
                parts[semis.len()].push(k);
            }
        }
    }
    let [init, cond, update] = parts;
    let mut semis = semis.into_iter();
    let mut control = Vec::new();
    control.extend(for_part(init));
    control.extend(semis.next());
    control.extend(cond);
    control.extend(semis.next());
    control.extend(for_part(update));
    head.push(node("forControl", control));
    head.extend(tail);
    head
}

/// ANTLR `FOR '(' enhancedForControl ')' statement`, `enhancedForControl:
/// variableModifier* typeType variableDeclaratorId ':' expression`.
fn enhanced_for(children: Vec<KRaw>) -> Vec<KRaw> {
    let Some((open, close)) = for_parens(&children) else {
        return children;
    };
    let mut head = children;
    let mut inner = head.split_off(open + 1);
    let tail = inner.split_off(close - open - 1);
    // `enhanced_for_statement` opens a scope, so the name is anchored under
    // `variable_declarator` (also a declaration parent) instead.
    let control = declarator_id(splice(inner, &["modifiers"]), "variable_declarator", true);
    head.push(node("enhancedForControl", control));
    head.extend(tail);
    head
}

/// ANTLR `catchClause: CATCH '(' variableModifier* catchType IDENTIFIER ')' block` (flat).
fn catch_clause(children: Vec<KRaw>) -> Vec<KRaw> {
    let mut out = Vec::with_capacity(children.len() + 2);
    for c in children {
        match c {
            KRaw::Node {
                kind: "catch_formal_parameter",
                children,
                ..
            } => out.extend(declarator_id(
                splice(children, &["modifiers"]),
                "catch_formal_parameter",
                true,
            )),
            other => out.push(other),
        }
    }
    out
}

/// ANTLR `switchLabel: CASE expression ':' | DEFAULT ':'`: tree-sitter leaves the `:`
/// outside `switch_label`.
fn switch_group(children: Vec<KRaw>) -> Vec<KRaw> {
    let mut out: Vec<KRaw> = Vec::with_capacity(children.len());
    for c in children {
        if is_tok(&c, ":") {
            if let Some(KRaw::Node {
                kind: "switch_label",
                children: label,
                ..
            }) = out.last_mut()
            {
                label.push(c);
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// ANTLR `switchBlockStatementGroup: switchLabel+ blockStatement+`: tree-sitter gives
/// every statement-less label its own group, so those labels join the next group;
/// trailing labels stay direct children (ANTLR's `switchLabel*` before `}`).
fn switch_block(children: Vec<KRaw>) -> Vec<KRaw> {
    let mut out = Vec::with_capacity(children.len());
    let mut pending: Vec<KRaw> = Vec::new();
    for c in children {
        match c {
            KRaw::Node {
                kind: "switch_block_statement_group",
                field,
                children,
            } => {
                if children.iter().all(|k| k.kind() == "switch_label") {
                    pending.extend(children);
                } else {
                    let mut merged = std::mem::take(&mut pending);
                    merged.extend(children);
                    out.push(KRaw::Node {
                        kind: "switch_block_statement_group",
                        field,
                        children: merged,
                    });
                }
            }
            other => {
                if is_tok(&other, "}") {
                    out.append(&mut pending);
                }
                out.push(other);
            }
        }
    }
    out.append(&mut pending);
    out
}

/// Lambda parameters are declarations: a bare `x -> …` parameter is wrapped as
/// `inferred_parameters` (collapse rule A removes the wrapper) and every identifier in
/// `inferred_parameters` gets field `name`.
fn lambda(children: Vec<KRaw>) -> Vec<KRaw> {
    children
        .into_iter()
        .map(|c| match c {
            t @ KRaw::Token {
                named: true,
                field: Some("parameters"),
                ..
            } => KRaw::Node {
                kind: "inferred_parameters",
                field: Some("parameters"),
                children: vec![t.with_field(Some("name"))],
            },
            other => other,
        })
        .collect()
}

/// ANTLR `expression '::' typeArguments? IDENTIFIER`: a bare identifier receiver is an
/// expression operand (`primary`), the method name is not.
fn method_reference(children: Vec<KRaw>) -> Vec<KRaw> {
    let mut out = children;
    if let Some(first) = out.first() {
        if first.kind() == "identifier" && matches!(first, KRaw::Token { .. }) {
            let receiver = out.remove(0);
            out.insert(0, anchor("primary", receiver));
        }
    }
    out
}
