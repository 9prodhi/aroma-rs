//! Partial-query wrapping (method body -> class member -> expression) and damage
//! measurement.
//!
//! A query snippet is rarely a whole file, so it is parsed inside synthetic wrappers,
//! tried in order: `statements` (a method body), `member` (a class member, e.g. a whole
//! method) and `expression` (a parenthesized field initializer). The first wrapper whose
//! damage is at most `limits.max_damage` wins; otherwise the attempt with the lowest damage
//! (an attempt that produced a tree beats one that did not; ties keep the earlier wrapper).
//!
//! * Damage = damaged bytes of the user's text / user text bytes. Damaged: bytes of ERROR
//!   subtrees, and tokens that tree-sitter placed outside the part the query tree is cut
//!   from (e.g. `bar();` after an extra `}` closed the wrapper method). Wrapper bytes,
//!   comments and whitespace never count; a subtree cut by the depth cap does. MISSING
//!   nodes cost nothing: tree-sitter closes an unclosed `{` or `(` of a partial snippet
//!   with the wrapper's own closer, which is exactly what a partial query needs.
//! * `statements`: the tree is the wrapper method's body block, so the query has the same
//!   `{...}` root as an indexed unit body. When the user text is itself one braced block
//!   (a unit body pasted as is), that block is the tree, so a pasted body and the indexed
//!   unit give identical features (D18 invariant).
//! * `member`: the tree is the body of the first unit (`spec.units`) inside the wrapper.
//!   A statements wrapper whose only statement is a unit declaration (tree-sitter-sfapex
//!   accepts methods as statements) is skipped in favour of `member`.
//!   JavaScript member policies compare statements and member damage first: an eligible member wins
//!   ties, but never wins with more damage. See [`js_member_span`] for the syntax
//!   guard that preserves statement keywords and call-plus-newline-block queries.
//! * `expression`: the wrapper's parentheses are removed; a single remaining child is
//!   the tree.
//! * Empty or comment-only text: `spt: None`, `damage: 0.0`, `wrapper: "none"`.
//!
//! Leaf byte spans and lines are rebased onto the user's text (bytes from 0, lines from 1).

use crate::classify::{unit_spt, VarMode};
use crate::globals::Globals;
use crate::kraw::KRaw;
use crate::spec::{JsMemberContext, JsSnippetPolicy, LangSpec};
use crate::table::{extract_units, parse, to_kraw, AdapterDiag, ExtractedUnit, Limits};
use aroma_core::spt::Spt;
use aroma_core::unit::UnitKind;
use std::ops::Range;
use tree_sitter::{Node, Tree};

#[derive(Clone, Debug)]
pub struct SnippetParse {
    pub spt: Option<Spt>,
    pub damage: f64,
    pub wrapper: &'static str,
}

/// How the shared snippet parser extracts the user tree from a wrapper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Statements,
    Member,
    Expression,
}

/// A synthetic context for parsing a partial snippet.
pub struct Wrapper {
    pub name: &'static str,
    pub shape: Shape,
    pub prefix: &'static str,
    pub suffix: &'static str,
}

/// Strategy for closing a partial snippet before applying its wrappers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repair {
    Brackets,
    Tags,
}

impl Repair {
    fn closers(self, text: &str) -> String {
        match self {
            Self::Brackets => closers(text),
            Self::Tags => tag_closers(text),
        }
    }
}

enum Attempt {
    /// No token in the user's text (statements wrapper, no damage).
    Empty,
    Parsed {
        spt: Option<Spt>,
        damage: f64,
    },
}

/// Most trailing lines [`parse_snippet`] drops when the whole text parses badly.
const MAX_TRIMMED_LINES: usize = 3;

/// Parse a partial query with the wrapper strategies described in the module docs.
///
/// A snippet copied out of a method often ends inside a construct that no closer can
/// repair (e.g. a SOQL query cut before `FROM`); tree-sitter's recovery can then turn the
/// whole text into ERROR. When the full text does not parse within `limits.max_damage`,
/// prefixes without the last 1..=[`MAX_TRIMMED_LINES`] lines are tried as well and the
/// best attempt wins (a tree beats none; then lower damage; ties keep the longer text).
/// Dropped bytes are user text that was not used, so they count as damage.
pub fn parse_snippet(
    spec: &LangSpec,
    text: &str,
    globals: &Globals,
    mode: VarMode,
    limits: &Limits,
) -> SnippetParse {
    let full = parse_whole(spec, text, globals, mode, limits);
    if full.spt.is_some() && full.damage <= limits.max_damage {
        return full;
    }
    let total = text.len().max(1) as f64;
    let mut best = full;
    let mut kept = text.trim_end();
    for _ in 0..MAX_TRIMMED_LINES {
        let Some(cut) = kept.rfind('\n') else {
            break;
        };
        kept = kept[..cut].trim_end();
        if kept.trim().is_empty() {
            break;
        }
        let mut p = parse_whole(spec, kept, globals, mode, limits);
        if p.spt.is_none() {
            continue;
        }
        p.damage = ((text.len() - kept.len()) as f64 + p.damage * kept.len() as f64) / total;
        if (p.spt.is_none(), p.damage) < (best.spt.is_none(), best.damage) {
            best = p;
        }
    }
    best
}

/// The wrapper strategies over the whole `text` (no trimming).
fn parse_whole(
    spec: &LangSpec,
    text: &str,
    globals: &Globals,
    mode: VarMode,
    limits: &Limits,
) -> SnippetParse {
    let mut best: Option<SnippetParse> = None;
    // A bare LWC path is ambiguous with literal HTML text. Prefer the expression
    // wrapper for valid property paths; markup and prose retain statements first.
    let expression_first = spec.lang == aroma_core::lang::Lang::LwcHtml
        && text.trim().split('.').all(|part| {
            let mut bytes = part.bytes();
            bytes
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic() || matches!(b, b'_' | b'$'))
                && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$'))
        });
    let mut order = if expression_first {
        [2, 0, 1]
    } else {
        [0, 1, 2]
    };
    let wrappers = if spec.lang == aroma_core::lang::Lang::SfMeta {
        crate::spec::sfmeta::snippet_wrappers(text)
    } else {
        spec.wrappers
    };
    let mut cached = [None, None, None];
    if spec.js_snippets.is_some() {
        let statements = attempt(spec, text, globals, mode, limits, &wrappers[0]);
        let member = attempt(spec, text, globals, mode, limits, &wrappers[1]);
        let statements_damage = match &statements {
            Attempt::Empty => 0.0,
            Attempt::Parsed { damage, .. } => *damage,
        };
        let prefer_member = matches!(
            &member,
            Attempt::Parsed { spt: Some(_), damage } if *damage <= statements_damage
        );
        if prefer_member {
            order = [1, 0, 2];
        }
        cached[0] = Some(statements);
        // A member with more damage must not win later as a fallback either.
        cached[1] = Some(if prefer_member {
            member
        } else {
            Attempt::Parsed {
                spt: None,
                damage: 1.0,
            }
        });
    }
    for index in order {
        let w = &wrappers[index];
        let (spt, damage) = match cached[index]
            .take()
            .unwrap_or_else(|| attempt(spec, text, globals, mode, limits, w))
        {
            Attempt::Empty => {
                return SnippetParse {
                    spt: None,
                    damage: 0.0,
                    wrapper: "none",
                }
            }
            Attempt::Parsed { spt, damage } => (spt, damage),
        };
        // A long JS decorated member can recover as statements with only the
        // short decorator damaged (<10%). Try all wrappers for an undamaged
        // member before accepting that recovery; other adapters keep precedence.
        if spt.is_some()
            && damage <= limits.max_damage
            && (spec.js_snippets.is_none() || damage == 0.0)
        {
            return SnippetParse {
                spt,
                damage,
                wrapper: w.name,
            };
        }
        let better = match &best {
            None => true,
            Some(b) => (spt.is_none(), damage) < (b.spt.is_none(), b.damage),
        };
        if better {
            best = Some(SnippetParse {
                spt,
                damage,
                wrapper: w.name,
            });
        }
    }
    best.unwrap_or(SnippetParse {
        spt: None,
        damage: 1.0,
        wrapper: "none",
    })
}

fn attempt(
    spec: &LangSpec,
    text: &str,
    globals: &Globals,
    mode: VarMode,
    limits: &Limits,
    w: &Wrapper,
) -> Attempt {
    const FAILED: Attempt = Attempt::Parsed {
        spt: None,
        damage: 1.0,
    };
    let wrapped = format!(
        "{}{}{}{}",
        w.prefix,
        text,
        spec.repair.closers(text),
        w.suffix
    );
    let user = w.prefix.len() as u32..(w.prefix.len() + text.len()) as u32;
    let user_len = f64::from(user.end - user.start).max(1.0);
    let Ok(tree) = parse(spec, &wrapped, limits) else {
        return FAILED;
    };
    let unit = if spec.repair == Repair::Tags {
        let root = tree.root_node();
        let anchor = if spec.lang == aroma_core::lang::Lang::SfMeta {
            root.child_by_field_name("root")
        } else {
            root.named_child(0)
        };
        let Some(anchor) = anchor else {
            return FAILED;
        };
        let span = anchor.start_byte() as u32..anchor.end_byte() as u32;
        let mut diag = AdapterDiag::default();
        let mut cursor = anchor.walk();
        let mut children = Vec::new();
        // XML adds a content container below the synthetic element. Strip that
        // container too, preserving the exact selected element for D18.
        let container = if spec.lang == aroma_core::lang::Lang::SfMeta {
            named_child_of_kind(anchor, "content").unwrap_or(anchor)
        } else {
            anchor
        };
        for child in container.children(&mut cursor) {
            // Strip the synthetic template and its formatting whitespace. Repaired
            // closing tags stay attached to their user element for useful structure.
            if child.end_byte() <= user.start as usize || child.start_byte() >= user.end as usize {
                continue;
            }
            children.extend(to_kraw(child, wrapped.as_bytes(), spec, limits, &mut diag));
        }
        let damage =
            f64::from(damaged_bytes(&tree, &user, &span).max(diag.damaged_bytes)) / user_len;
        if children.is_empty() && damage == 0.0 {
            return Attempt::Empty;
        }
        let body = if children.len() == 1 {
            children.pop().expect("one child").with_field(None)
        } else {
            KRaw::Node {
                kind: "document",
                field: None,
                children,
            }
        };
        ExtractedUnit {
            kind: UnitKind::Snippet,
            bytes: user.clone(),
            lines: 1..2 + text.lines().count() as u32,
            body,
            damage,
        }
    } else if w.shape == Shape::Member {
        let member_span = if let Some(policy) = spec.js_snippets {
            let Some(span) = js_member_span(&tree, &wrapped, &user, policy) else {
                return FAILED;
            };
            Some(span)
        } else {
            None
        };
        let Some(unit) = extract_units(&tree, &wrapped, spec, limits)
            .into_iter()
            .find(|u| u.bytes.start >= user.start)
        else {
            return FAILED;
        };
        let damage = f64::from(damaged_bytes(
            &tree,
            &user,
            member_span.as_ref().unwrap_or(&unit.bytes),
        )) / user_len;
        ExtractedUnit { damage, ..unit }
    } else {
        let anchor = if w.shape == Shape::Statements {
            statements_anchor(&tree, spec)
        } else {
            expression_anchor(&tree, spec)
        };
        let Some(anchor) = anchor else {
            return FAILED;
        };
        let span = anchor.start_byte() as u32..anchor.end_byte() as u32;
        let mut damage = f64::from(damaged_bytes(&tree, &user, &span)) / user_len;
        let mut diag = AdapterDiag::default();
        let kraw = to_kraw(anchor, wrapped.as_bytes(), spec, limits, &mut diag);
        if diag.depth_capped {
            damage = damage.max(f64::from(diag.damaged_bytes) / user_len);
        }
        let body = match kraw.map(|k| user_body(k, &user, w.shape, spec)) {
            Some(Body::Tree(body)) => body,
            Some(Body::Empty) if w.shape == Shape::Statements && damage == 0.0 => {
                return Attempt::Empty;
            }
            Some(Body::Empty | Body::LoneUnit) | None => return FAILED,
        };
        ExtractedUnit {
            kind: UnitKind::Snippet,
            bytes: user.clone(),
            lines: 2..2 + text.lines().count().max(1) as u32,
            body,
            damage,
        }
    };
    let prefix_lines = w.prefix.matches('\n').count() as u32;
    let spt = unit_spt(&unit, spec, globals, mode).map(|mut spt| {
        for leaf in &mut spt.leaves {
            leaf.bytes = rebase(&leaf.bytes, &user);
            leaf.line = leaf.line.saturating_sub(prefix_lines).max(1);
        }
        spt
    });
    Attempt::Parsed {
        spt,
        damage: unit.damage.min(1.0),
    }
}

/// Conservative JavaScript member disambiguation, using the wrapper CST rather
/// than a text regex (decorator arguments can span lines and contain keywords).
/// Object contexts select function-valued pairs instead of class members.
/// The first non-comment class child must be a method or field, and its first
/// non-decorator token must not be a statement keyword, even when JavaScript
/// permits that spelling as a method name. Methods with a line break between
/// parameters and body retain statements precedence: `foo()\n{ ... }` is a call
/// followed by a block. This intentionally also leaves Allman-style methods as
/// statements. The caller additionally requires member damage <= statements damage.
fn js_member_span(
    tree: &Tree,
    src: &str,
    user: &Range<u32>,
    policy: &JsSnippetPolicy,
) -> Option<Range<u32>> {
    let body = match policy.member_context {
        JsMemberContext::Class => {
            let class = named_child_of_kind(tree.root_node(), "class_declaration")?;
            class.child_by_field_name("body")?
        }
        JsMemberContext::Object => {
            let statement = named_child_of_kind(tree.root_node(), "expression_statement")?;
            let expression = named_child_of_kind(statement, "parenthesized_expression")?;
            named_child_of_kind(expression, "object")?
        }
    };
    let mut cursor = body.walk();
    let member = body
        .named_children(&mut cursor)
        .find(|n| !n.is_extra() && n.kind() != "comment")?;
    let eligible = match policy.member_context {
        JsMemberContext::Class => matches!(member.kind(), "method_definition" | "field_definition"),
        JsMemberContext::Object => {
            member.kind() == "pair"
                && member
                    .child_by_field_name("value")
                    .is_some_and(|value| value.kind() == "function_expression")
        }
    };
    if !eligible || member.start_byte() < user.start as usize {
        return None;
    }
    if member.kind() == "method_definition" {
        let parameters = member.child_by_field_name("parameters")?;
        let block = member.child_by_field_name("body")?;
        if parameters.end_position().row != block.start_position().row {
            return None;
        }
    }
    let mut cursor = member.walk();
    let first = loop {
        let n = cursor.node();
        let skip = n.is_extra() || matches!(n.kind(), "comment" | "decorator");
        if !skip && n.child_count() == 0 && !n.is_missing() {
            break &src[n.byte_range()];
        }
        if !skip && cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return None;
            }
        }
    };
    if policy.statement_keywords.contains(&first) {
        return None;
    }
    // Member separators belong to the pasted member: a class field's optional
    // semicolon and an object property's trailing comma are siblings in the CST.
    let end = member
        .next_sibling()
        .filter(|n| {
            (member.kind() == "field_definition" && n.kind() == ";")
                || (policy.member_context == JsMemberContext::Object && n.kind() == ",")
        })
        .map_or(member.end_byte(), |n| n.end_byte());
    Some(member.start_byte() as u32..end as u32)
}

/// Closers for the brackets the user's text leaves open, innermost first, e.g. `"\n)}}"`.
///
/// Partial snippets usually stop inside several blocks; tree-sitter recovers only the
/// innermost one with the wrapper's own closer, so the rest would turn into ERROR nodes.
/// The closers are wrapper bytes: they are never part of the user's range and never count
/// as damage. String and character literals (`'…'`, `"…"`, with `\` escapes) and comments
/// (`//…`, `/*…*/`) are skipped. On a mismatched closer the text is left as is (empty
/// result) so the damage measurement still sees the user's mistake. A block opened right
/// after the keyword `try` (any case, for Apex) closes as `} finally {}`, because a `try`
/// without `catch`/`finally` is a syntax error in both languages.
fn closers(text: &str) -> String {
    let b = text.as_bytes();
    let mut stack: Vec<&'static str> = Vec::new();
    // the identifier/keyword immediately before the current position (None after any
    // other significant character)
    let mut last_word: Option<&str> = None;
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_alphabetic() || b[i] == b'_' || b[i] == b'$' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'$') {
                i += 1;
            }
            last_word = Some(&text[start..i]);
            continue;
        }
        // whitespace and comments keep `last_word`; anything else clears it
        let mut keep_word = b[i].is_ascii_whitespace();
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                keep_word = true;
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                keep_word = true;
                i += 2;
                while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                i += 1;
            }
            q @ (b'\'' | b'"') => {
                i += 1;
                while i < b.len() && b[i] != q && b[i] != b'\n' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'{' => stack.push(
                if last_word.is_some_and(|w| w.eq_ignore_ascii_case("try")) {
                    "} finally {}"
                } else {
                    "}"
                },
            ),
            b'(' => stack.push(")"),
            b'[' => stack.push("]"),
            c @ (b'}' | b')' | b']') => {
                if stack.pop().map(|s| s.as_bytes()[0]) != Some(c) {
                    return String::new();
                }
            }
            _ => {}
        }
        if !keep_word {
            last_word = None;
        }
        i += 1;
    }
    if stack.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n");
    out.extend(stack.iter().rev().copied());
    out
}

/// Closers for the HTML/XML tags the user's text leaves open, innermost first,
/// e.g. `"</div></template>"`.
///
/// Used by [`Repair::Tags`] for markup snippets (HTML, XML, Aura, Visualforce) that
/// are often pasted truncated with open tags. The text is scanned once, keeping a
/// stack of open element names; every element still open at the end gets a matching
/// `</name>` in output order. Element names may contain `-`, `:`, `.`, `_` and digits
/// (e.g. `lightning-datatable`, `aura:if`, `apex:pageBlock`) and keep their case.
///
/// Constructs that are skipped whole (no `<`/`>` inside them is interpreted):
/// comments `<!-- … -->`, CDATA `<![CDATA[ … ]]>`, processing instructions `<? … ?>`,
/// `<!DOCTYPE …>`, quoted attribute values (`'…'` and `"…"`), and brace expressions
/// `{…}` / `{!…}` inside a tag (LWC `onclick={handleClick}`, Aura `value="{!v.a > 1}"`,
/// which may contain `>`). Self-closing tags (`<x … />`) and HTML void elements
/// (area, base, br, col, embed, hr, img, input, link, meta, param, source, track,
/// wbr; ASCII case-insensitive) never push. A `</name>` matching the top pops it; one
/// matching a deeper entry pops down to it (tolerant HTML); one matching nothing means
/// the snippet is an interior fragment, which is not repaired (D43) and yields `""`.
///
/// An unterminated construct at the end of the text (open comment/CDATA/PI/DOCTYPE,
/// open quote, or a tag cut mid-attribute like `<div class="a`) would need closer
/// machinery that is out of scope: the function returns `""` and lets the existing
/// trailing-line trim in [`parse_snippet`] handle it.
fn tag_closers(text: &str) -> String {
    let b = text.as_bytes();
    let mut stack: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        // `<!-- … -->` comment.
        if b[i] == b'<'
            && b.get(i + 1) == Some(&b'!')
            && b.get(i + 2) == Some(&b'-')
            && b.get(i + 3) == Some(&b'-')
        {
            let Some(end) = text[i + 4..].find("-->") else {
                return String::new();
            };
            i += 4 + end + 3;
            continue;
        }
        // `<![CDATA[ … ]]>`.
        if b[i..].starts_with(b"<![CDATA[") {
            let Some(end) = text[i + 9..].find("]]>") else {
                return String::new();
            };
            i += 9 + end + 3;
            continue;
        }
        // `<? … ?>` processing instruction.
        if b[i] == b'<' && b.get(i + 1) == Some(&b'?') {
            let Some(end) = text[i + 2..].find("?>") else {
                return String::new();
            };
            i += 2 + end + 2;
            continue;
        }
        // `<!DOCTYPE …>` or other `<!…>` declaration.
        if b[i] == b'<' && b.get(i + 1) == Some(&b'!') {
            let Some(end) = text[i + 2..].find('>') else {
                return String::new();
            };
            i += 2 + end + 1;
            continue;
        }
        // Closing tag `</name …>`.
        if b[i] == b'<' && b.get(i + 1) == Some(&b'/') {
            let mut j = i + 2;
            while j < b.len() && is_tag_name_char(b[j]) {
                j += 1;
            }
            let name = &text[i + 2..j];
            let next = match scan_to_tag_end(text, j) {
                Some(k) => k,
                None => return String::new(),
            };
            // Tolerant HTML: pop down to the matching entry; no match = interior fragment.
            match stack.iter().rposition(|n| *n == name) {
                Some(pos) => stack.truncate(pos),
                None => return String::new(),
            }
            i = next;
            continue;
        }
        // Start tag `<name …>`.
        if b[i] == b'<' && b.get(i + 1).is_some_and(|c| is_tag_name_start(*c)) {
            let mut j = i + 1;
            while j < b.len() && is_tag_name_char(b[j]) {
                j += 1;
            }
            let name = &text[i + 1..j];
            let next = match scan_to_tag_end(text, j) {
                Some(k) => k,
                None => return String::new(),
            };
            // self-closing? (a `/` right before the `>` at `next - 1`, ignoring whitespace)
            let before_gt = text[j..next - 1].trim_end();
            let self_closing = before_gt.ends_with('/');
            if !self_closing && !is_void_element(name) {
                stack.push(name);
            }
            i = next;
            continue;
        }
        i += 1;
    }
    if stack.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for name in stack.iter().rev() {
        out.push_str("</");
        out.push_str(name);
        out.push('>');
    }
    out
}

fn is_tag_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b':'
}

fn is_tag_name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b':' | b'.')
}

/// Skip quoted attribute values and brace expressions from `start` to the end of the
/// tag (the first unquoted `>`), returning the position after it. Inside a tag a `>`
/// inside a quoted value or a `{…}`/`{!…}` brace expression is ignored, so the real
/// end of the tag is found. Returns `None` when the tag is cut before a `>` (open
/// quote or brace, or an incomplete attribute), which is out of scope.
fn scan_to_tag_end(text: &str, start: usize) -> Option<usize> {
    let b = text.as_bytes();
    // A `"`, `'` or `{` opens a construct; `{` is always a brace expression in a tag.
    let mut open: Vec<u8> = Vec::new();
    let mut k = start;
    while k < b.len() {
        match b[k] {
            b'"' | b'\'' => {
                if open.is_empty() || open.last() == Some(&b'{') {
                    open.push(b[k]);
                } else if open.last() == Some(&b[k]) {
                    open.pop();
                }
                // inside a different quote it is a literal byte: leave it
            }
            b'{' => {
                if !matches!(open.last(), Some(&b'"') | Some(&b'\'')) {
                    open.push(b'{');
                }
            }
            b'}' if open.last() == Some(&b'{') => {
                open.pop();
            }
            b'>' if open.is_empty() => return Some(k + 1),
            _ => {}
        }
        k += 1;
    }
    None
}

fn is_void_element(name: &str) -> bool {
    const VOID: &[&str] = &[
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
        "source", "track", "wbr",
    ];
    let lower = name.to_ascii_lowercase();
    VOID.contains(&lower.as_str())
}

/// Byte span relative to the user's text, clamped into it.
fn rebase(bytes: &Range<u32>, user: &Range<u32>) -> Range<u32> {
    let len = user.end - user.start;
    let at = |b: u32| b.saturating_sub(user.start).min(len);
    at(bytes.start)..at(bytes.end)
}

fn overlap(a: &Range<u32>, b: &Range<u32>) -> u32 {
    a.end.min(b.end).saturating_sub(a.start.max(b.start))
}

/// Damaged bytes of the user's text: every ERROR subtree (outermost only) plus every
/// token that tree-sitter placed outside `anchor` (the part the query tree is cut from),
/// both intersected with `user`. Comments and whitespace never count.
fn damaged_bytes(tree: &Tree, user: &Range<u32>, anchor: &Range<u32>) -> u32 {
    let mut total = 0;
    let mut cursor = tree.walk();
    loop {
        let n = cursor.node();
        let span = n.start_byte() as u32..n.end_byte() as u32;
        let descend = if n.is_error() {
            total += overlap(&span, user);
            false
        } else if n.is_extra() {
            false
        } else if n.child_count() == 0 {
            if span.start < anchor.start || span.end > anchor.end {
                total += overlap(&span, user);
            }
            false
        } else {
            true
        };
        if descend && cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return total;
            }
        }
    }
}

fn named_child_of_kind<'t>(n: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = n.walk();
    let found = n.named_children(&mut cursor).find(|c| c.kind() == kind);
    found
}

/// The wrapper method's body block.
fn statements_anchor<'t>(tree: &'t Tree, spec: &LangSpec) -> Option<Node<'t>> {
    let class = named_child_of_kind(tree.root_node(), "class_declaration")?;
    let body = class.child_by_field_name("body")?;
    let method_kind = spec
        .js_snippets
        .map_or("method_declaration", |p| p.method_kind);
    let method = named_child_of_kind(body, method_kind)?;
    method.child_by_field_name(spec.body_field)
}

/// The wrapper field's parenthesized initializer.
fn expression_anchor<'t>(tree: &'t Tree, spec: &LangSpec) -> Option<Node<'t>> {
    if spec.js_snippets.is_some() {
        let decl = named_child_of_kind(tree.root_node(), "lexical_declaration")?;
        return named_child_of_kind(decl, "variable_declarator")?.child_by_field_name("value");
    }
    let class = named_child_of_kind(tree.root_node(), "class_declaration")?;
    let body = class.child_by_field_name("body")?;
    let field = named_child_of_kind(body, "field_declaration")?;
    field
        .child_by_field_name("declarator")?
        .child_by_field_name("value")
}

enum Body {
    Empty,
    /// The statements wrapper holds exactly one unit declaration (tree-sitter-sfapex
    /// accepts a method declaration as a statement): left to the member wrapper.
    LoneUnit,
    Tree(KRaw),
}

/// Remove the wrapper's own delimiters from the anchor (see module docs).
fn user_body(anchor: KRaw, user: &Range<u32>, shape: Shape, spec: &LangSpec) -> Body {
    let (kind, children) = match anchor {
        KRaw::Node { kind, children, .. } => (kind, children),
        token @ KRaw::Token { .. } => return Body::Tree(token.with_field(None)),
    };
    let synthetic_first = matches!(
        children.first(),
        Some(KRaw::Token { bytes, .. }) if bytes.start < user.start
    );
    let synthetic_last = children.len() > usize::from(synthetic_first)
        && matches!(
            children.last(),
            Some(KRaw::Token { bytes, .. }) if bytes.start >= user.end
        );
    let inner = usize::from(synthetic_first)..children.len() - usize::from(synthetic_last);
    if inner.is_empty() {
        return Body::Empty;
    }
    let single = inner.len() == 1;
    match shape {
        Shape::Statements => {
            let lone = &children[inner.start];
            if spec.js_snippets.is_some() && single {
                if let KRaw::Node {
                    kind: "expression_statement",
                    children: expr,
                    ..
                } = lone
                {
                    if expr.len() == 1 {
                        return Body::Tree(expr[0].clone().with_field(None));
                    }
                }
            }
            if single
                && spec.js_snippets.is_none()
                && spec.units.iter().any(|(k, _)| *k == lone.kind())
            {
                return Body::LoneUnit;
            }
            let pasted_block = single && matches!(lone, KRaw::Node { kind: k, .. } if *k == kind);
            if pasted_block {
                let block = children.into_iter().nth(inner.start).expect("in range");
                Body::Tree(block.with_field(None))
            } else {
                Body::Tree(KRaw::Node {
                    kind,
                    field: None,
                    children,
                })
            }
        }
        Shape::Expression | Shape::Member => {
            let mut kept: Vec<KRaw> = children
                .into_iter()
                .skip(inner.start)
                .take(inner.len())
                .collect();
            if single {
                Body::Tree(kept.remove(0).with_field(None))
            } else {
                Body::Tree(KRaw::Node {
                    kind,
                    field: None,
                    children: kept,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Repair;

    fn close(text: &str) -> String {
        Repair::Tags.closers(text)
    }

    #[test]
    fn nested_open_tags_close_innermost_first() {
        assert_eq!(close("<template><div><span>"), "</span></div></template>");
    }

    #[test]
    fn self_closing_tag_never_pushes() {
        assert_eq!(close("<img src=\"x\" /><div>"), "</div>");
        // whitespace before the `/>` still counts as self-closing
        assert_eq!(close("<hr /> <div><br/></div><p>"), "</p>");
    }

    proptest::proptest! {
        #[test]
        fn repair_never_panics_on_arbitrary_unicode(text in "\\PC{0,200}") {
            let _ = Repair::Tags.closers(&text);
            let _ = Repair::Brackets.closers(&text);
        }

        #[test]
        fn repair_never_panics_on_markup_with_multibyte(
            text in "[<>/!\\[\\]{}=\"' a-z›é日—-]{0,120}"
        ) {
            let _ = Repair::Tags.closers(&text);
            let _ = Repair::Brackets.closers(&text);
        }
    }

    #[test]
    fn multibyte_text_never_splits_a_character() {
        // Regression: `›` (3 bytes) once made the byte scan slice inside a character.
        let crumb =
            "<span class=\"breadcrumb-item\" key={item.key}>\n<span aria-hidden=\"true\">›</span>";
        assert_eq!(close(crumb), "</span>");
        assert_eq!(close("<p>héllo — “quoted” 日本語 <b>"), "</b></p>");
        assert_eq!(close("<!-- ça › va --><div>€"), "</div>");
        assert_eq!(close("<x a='›'>›<![CDATA[ › ]]>"), "</x>");
        assert_eq!(close("<x>›<"), "</x>");
        assert_eq!(close("›"), "");
    }

    #[test]
    fn non_void_self_closing_tags_never_push() {
        assert_eq!(
            close("<template><c-child record-id={recordId} />"),
            "</template>"
        );
        assert_eq!(
            close("<aura:component><aura:set attribute=\"x\"/>"),
            "</aura:component>"
        );
        assert_eq!(close("<Flow><start/><decisions>"), "</decisions></Flow>");
    }

    #[test]
    fn void_elements_without_slash_never_push() {
        assert_eq!(close("<br><img src=\"x\"><input><link><div>"), "</div>");
        // case-insensitive void detection
        assert_eq!(close("<IMG SRC=\"x\"><DIV>"), "</DIV>");
    }

    #[test]
    fn balanced_input_yields_empty() {
        assert_eq!(close("<div></div><span></span>"), "");
    }

    #[test]
    fn comment_containing_tags_is_skipped() {
        assert_eq!(close("<!-- <div> <span> --><div>"), "</div>");
        // unterminated comment at the end is out of scope
        assert_eq!(close("<!-- <div> <span>"), "");
    }

    #[test]
    fn cdata_containing_close_tag_is_skipped() {
        assert_eq!(close("<![CDATA[ </x> <y> ]]><div>"), "</div>");
        assert_eq!(close("<![CDATA[ </x>"), "");
    }

    #[test]
    fn quoted_attribute_containing_gt_is_skipped() {
        assert_eq!(close("<div title=\"a>b\"><span>"), "</span></div>");
        assert_eq!(close("<div title='a>b'><span>"), "</span></div>");
    }

    #[test]
    fn lwc_brace_expression_containing_gt_is_skipped() {
        assert_eq!(close("<template lwc:if={x > 1}><div>"), "</div></template>");
        assert_eq!(close("<button onclick={handleClick}>Go</button>"), "");
    }

    #[test]
    fn aura_brace_in_quoted_attribute_containing_gt_is_skipped() {
        assert_eq!(
            close("<aura:if value=\"{!v.a > 1}\"><div>"),
            "</div></aura:if>"
        );
    }

    #[test]
    fn mismatched_close_is_an_interior_fragment() {
        assert_eq!(close("<div></span>"), "");
    }

    #[test]
    fn closing_tag_pops_down_to_deeper_match() {
        // tolerant HTML: `</p>` closes past the open `<span>`, so only `<div>` stays
        assert_eq!(close("<div><p><span></p>"), "</div>");
    }

    #[test]
    fn namespaced_and_hyphenated_names_keep_case() {
        assert_eq!(
            close("<aura:iteration><apex:pageBlock><lightning-datatable>"),
            "</lightning-datatable></apex:pageBlock></aura:iteration>"
        );
        assert_eq!(close("<Flow>"), "</Flow>");
    }

    #[test]
    fn xml_metadata_fragment_closes_open_elements() {
        assert_eq!(
            close("<Flow><decisions><name>A</name><rules>"),
            "</rules></decisions></Flow>"
        );
    }

    #[test]
    fn cut_mid_tag_is_out_of_scope() {
        assert_eq!(close("<div class=\"a"), "");
        assert_eq!(close("<div"), "");
        assert_eq!(close("<div class="), "");
        assert_eq!(close("<div class=\"a\" title="), "");
    }
}
