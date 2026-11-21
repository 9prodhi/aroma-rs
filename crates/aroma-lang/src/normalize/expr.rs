//! Shared expression parser for opaque Salesforce markup and metadata text.
//!
//! Whitespace is omitted; all other successful tokens preserve their source spelling.
//! Empty (or whitespace-only) input is an empty `expression` node. A trailing error
//! wraps the parsed prefix in `expression`; errors inside constructs stay in that
//! construct. Missing input at EOF has a zero-width error range. All fields are `None`.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use crate::kraw::{KRaw, ERR_TOKEN};

const MAX_DEPTH: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    Lwc,
    Aura,
    Formula,
}

/// Parse expression text without its surrounding `{`/`{!`/`}`.
///
/// `base` is the absolute byte offset and `line` the 1-based starting line. On the
/// first syntax error, the parsed prefix is kept and the remainder becomes one
/// anonymous `ERROR` token with text [`ERR_TOKEN`]. Nesting and the depth of binary
/// chains are capped at 200. Offsets/lines saturate if they cannot fit in `u32`.
pub fn parse_expression(text: &str, dialect: Dialect, base: u32, line: u32) -> KRaw {
    let mut parser = Parser {
        text,
        dialect,
        base,
        line,
        pos: 0,
        failed: false,
    };
    if parser.peek().kind == LexKind::End {
        return node("expression", Vec::new());
    }
    let parsed = parser.expression(0, 0).tree;
    if !parser.failed && parser.peek().kind != LexKind::End {
        node("expression", vec![parsed, parser.error().tree])
    } else {
        parsed
    }
}

/// Find expression spans, including their delimiters in `start..end`, and borrow
/// their inner text. LWC uses `{...}`; Aura uses `{!...}` and `{#...}`; Formula uses
/// `{!...}`.
///
/// Quotes and backslash escapes *inside* an expression hide braces. Outer quotes
/// are ordinary text, so a quoted attribute value can be passed directly. Nested
/// braces are balanced iteratively; an unterminated span is omitted.
pub fn find_expressions(text: &str, dialect: Dialect) -> Vec<(usize, usize, &str)> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut at = 0;
    // LWC uses `{…}`; Aura uses `{!…}` and `{#…}`; Formula uses `{!…}`.
    while at < bytes.len() {
        if bytes[at] != b'{' {
            at += 1;
            continue;
        }
        let inner_skip = if dialect == Dialect::Lwc { 1 } else { 2 };
        if inner_skip == 2 {
            let next = bytes.get(at + 1);
            let aura_open = dialect == Dialect::Aura && next == Some(&b'#');
            if next != Some(&b'!') && !aura_open {
                at += 1;
                continue;
            }
        }
        let start = at;
        let inner = start + inner_skip;
        at = inner;
        let mut depth = 1usize;
        let mut quote = None;
        while at < bytes.len() {
            let b = bytes[at];
            if let Some(q) = quote {
                if b == b'\\' {
                    at += usize::from(at + 1 < bytes.len()) + 1;
                    continue;
                }
                if b == q {
                    quote = None;
                }
            } else {
                match b {
                    b'\'' | b'"' => quote = Some(b),
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            spans.push((start, at + 1, &text[inner..at]));
                            at += 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            at += 1;
        }
    }
    spans
}

fn node(kind: &'static str, children: Vec<KRaw>) -> KRaw {
    KRaw::Node {
        kind,
        field: None,
        children,
    }
}

/// Most distinct call names [`call_kind`] interns; later new names share [`OTHER_CALL`].
const MAX_CALL_KINDS: usize = 4096;
/// Kind for call names seen after the interning table is full.
const OTHER_CALL: &str = "CALL";

// KRaw requires static kinds. Keep one process-lifetime allocation per distinct
// canonical call name, shared across parses (rather than leaking on every call), and
// stop interning at MAX_CALL_KINDS so hostile input cannot grow memory without bound.
fn call_kind(name: &str) -> &'static str {
    static NAMES: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let canonical = name.to_ascii_uppercase();
    let mut names = NAMES
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(&kind) = names.get(canonical.as_str()) {
        return kind;
    }
    if names.len() >= MAX_CALL_KINDS {
        return OTHER_CALL;
    }
    let kind = Box::leak(canonical.into_boxed_str());
    names.insert(kind);
    kind
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LexKind {
    Name,
    Number,
    String,
    Symbol(&'static str),
    Invalid,
    End,
}

#[derive(Clone, Copy)]
struct Lexeme {
    kind: LexKind,
    start: usize,
    end: usize,
}

struct Parsed {
    tree: KRaw,
    height: usize,
}

impl Parsed {
    fn token(tree: KRaw) -> Self {
        Self { tree, height: 0 }
    }

    fn group(kind: &'static str, children: Vec<Self>) -> Self {
        let height = children.iter().map(|c| c.height).max().unwrap_or(0) + 1;
        Self {
            tree: node(kind, children.into_iter().map(|c| c.tree).collect()),
            height,
        }
    }
}

struct Parser<'a> {
    text: &'a str,
    dialect: Dialect,
    base: u32,
    line: u32,
    pos: usize,
    failed: bool,
}

impl Parser<'_> {
    fn offset(&self, pos: usize) -> u32 {
        self.base
            .saturating_add(u32::try_from(pos).unwrap_or(u32::MAX))
    }

    fn advance(&mut self, end: usize) {
        let lines = self.text.as_bytes()[self.pos..end]
            .iter()
            .filter(|&&b| b == b'\n')
            .count();
        self.line = self
            .line
            .saturating_add(u32::try_from(lines).unwrap_or(u32::MAX));
        self.pos = end;
    }

    fn peek(&mut self) -> Lexeme {
        let bytes = self.text.as_bytes();
        let mut end = self.pos;
        while bytes.get(end).is_some_and(u8::is_ascii_whitespace) {
            end += 1;
        }
        self.advance(end);
        lex(self.text, self.pos)
    }

    fn at(&mut self, symbol: &'static str) -> bool {
        self.peek().kind == LexKind::Symbol(symbol)
    }

    fn take(&mut self, kind: &'static str, named: bool) -> Parsed {
        let lexeme = self.peek();
        let tree = KRaw::Token {
            kind,
            named,
            text: self.text[lexeme.start..lexeme.end].into(),
            bytes: self.offset(lexeme.start)..self.offset(lexeme.end),
            line: self.line,
            field: None,
        };
        self.advance(lexeme.end);
        Parsed::token(tree)
    }

    fn error(&mut self) -> Parsed {
        self.peek();
        let tree = KRaw::Token {
            kind: "ERROR",
            named: false,
            text: ERR_TOKEN.into(),
            bytes: self.offset(self.pos)..self.offset(self.text.len()),
            line: self.line,
            field: None,
        };
        self.failed = true;
        self.pos = self.text.len();
        Parsed::token(tree)
    }

    fn expression(&mut self, min_precedence: u8, depth: usize) -> Parsed {
        if depth >= MAX_DEPTH {
            return self.error();
        }
        let mut lhs = self.primary(depth);
        while !self.failed {
            let lexeme = self.peek();
            let ternary = self.dialect == Dialect::Aura
                && lexeme.kind == LexKind::Symbol("?")
                && min_precedence <= 1;
            let binary = self.binary_operator(lexeme);
            if !ternary && binary.is_none_or(|(_, precedence)| precedence < min_precedence) {
                break;
            }
            // Left-associative chains also need a tree-depth bound: their parser
            // stack is shallow, but an unbounded KRaw chain would overflow on drop.
            if lhs.height >= MAX_DEPTH - depth {
                return Parsed::group("expression", vec![lhs, self.error()]);
            }
            if ternary {
                let mut children = vec![lhs, self.take("?", false)];
                children.push(self.expression(0, depth + 1));
                if !self.failed {
                    if self.at(":") {
                        children.push(self.take(":", false));
                        children.push(self.expression(1, depth + 1));
                    } else {
                        children.push(self.error());
                    }
                }
                lhs = Parsed::group("ternary", children);
            } else if let Some((kind, precedence)) = binary {
                let op = self.take(kind, false);
                let rhs = self.expression(precedence + 1, depth + 1);
                lhs = Parsed::group("binary", vec![lhs, op, rhs]);
            }
        }
        lhs
    }

    fn binary_operator(&self, lexeme: Lexeme) -> Option<(&'static str, u8)> {
        if self.dialect == Dialect::Lwc {
            return None;
        }
        let kind = match lexeme.kind {
            LexKind::Symbol(kind) => kind,
            LexKind::Name if self.dialect == Dialect::Aura => {
                match &self.text[lexeme.start..lexeme.end] {
                    name if name.eq_ignore_ascii_case("OR") => "OR",
                    name if name.eq_ignore_ascii_case("AND") => "AND",
                    _ => return None,
                }
            }
            _ => return None,
        };
        let precedence = match kind {
            "||" | "OR" => 2,
            "&&" | "AND" => 3,
            "=" | "==" | "!=" | "<>" => 4,
            "<" | "<=" | ">" | ">=" => 5,
            "+" | "-" | "&" => 6,
            "*" | "/" | "^" => 7,
            _ => return None,
        };
        Some((kind, precedence))
    }

    fn primary(&mut self, depth: usize) -> Parsed {
        let lexeme = self.peek();
        let spelling = &self.text[lexeme.start..lexeme.end];
        let literal_name = ["true", "false", "null"]
            .iter()
            .any(|word| spelling.eq_ignore_ascii_case(word));
        if self.dialect != Dialect::Lwc {
            match lexeme.kind {
                LexKind::Number | LexKind::String => return self.take("literal", true),
                LexKind::Name if literal_name => return self.take("literal", true),
                LexKind::Symbol(kind @ ("-" | "!")) => {
                    let op = self.take(kind, false);
                    let operand = self.expression(8, depth + 1);
                    return Parsed::group("unary", vec![op, operand]);
                }
                LexKind::Symbol("(") => {
                    let mut children = vec![self.take("(", false)];
                    children.push(self.expression(0, depth + 1));
                    self.close(&mut children);
                    return Parsed::group("paren", children);
                }
                LexKind::Name if spelling.eq_ignore_ascii_case("NOT") => {
                    let end = self.text[lexeme.end..].trim_start();
                    if !end.starts_with('(') {
                        let op = self.take("NOT", false);
                        let operand = self.expression(8, depth + 1);
                        return Parsed::group("unary", vec![op, operand]);
                    }
                }
                _ => {}
            }
        }
        if lexeme.kind != LexKind::Name || literal_name {
            return self.error();
        }
        let name = self.take("identifier", true);
        if self.dialect != Dialect::Lwc && self.at("(") {
            let mut name = name;
            if let KRaw::Token { kind, named, .. } = &mut name.tree {
                *kind = call_kind(spelling);
                *named = false;
            }
            let mut children = vec![name, self.take("(", false)];
            if !self.at(")") {
                loop {
                    children.push(self.expression(0, depth + 1));
                    if self.failed || !self.at(",") {
                        break;
                    }
                    children.push(self.take(",", false));
                }
            }
            self.close(&mut children);
            return Parsed::group("call", children);
        }
        let mut children = vec![name];
        while self.at(".") {
            children.push(self.take(".", false));
            if self.peek().kind != LexKind::Name {
                children.push(self.error());
                break;
            }
            children.push(self.take("identifier", true));
        }
        if children.len() == 1 {
            // The initial name is always present.
            children.remove(0)
        } else {
            Parsed::group("path", children)
        }
    }

    fn close(&mut self, children: &mut Vec<Parsed>) {
        if !self.failed {
            if self.at(")") {
                children.push(self.take(")", false));
            } else {
                children.push(self.error());
            }
        }
    }
}

fn name_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || matches!(b, b'_' | b'$')
}

fn lex(text: &str, start: usize) -> Lexeme {
    let bytes = text.as_bytes();
    let mut end = start;
    let Some(&first) = bytes.get(start) else {
        return Lexeme {
            kind: LexKind::End,
            start,
            end,
        };
    };
    let kind = if name_start(first) {
        end += 1;
        while bytes
            .get(end)
            .is_some_and(|&b| name_start(b) || b.is_ascii_digit())
        {
            end += 1;
        }
        LexKind::Name
    } else if first.is_ascii_digit()
        || (first == b'.' && bytes.get(start + 1).is_some_and(u8::is_ascii_digit))
    {
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        if bytes.get(end) == Some(&b'.') {
            end += 1;
            while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                end += 1;
            }
        }
        if matches!(bytes.get(end), Some(b'e' | b'E')) {
            end += 1;
            if matches!(bytes.get(end), Some(b'+' | b'-')) {
                end += 1;
            }
            let exponent_start = end;
            while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                end += 1;
            }
            if end == exponent_start {
                return Lexeme {
                    kind: LexKind::Invalid,
                    start,
                    end,
                };
            }
        }
        LexKind::Number
    } else if matches!(first, b'\'' | b'"') {
        end += 1;
        let mut closed = false;
        while end < bytes.len() {
            let b = bytes[end];
            end += 1;
            if b == b'\\' {
                end += usize::from(end < bytes.len());
            } else if b == first {
                closed = true;
                break;
            }
        }
        if closed {
            LexKind::String
        } else {
            LexKind::Invalid
        }
    } else {
        // Match longest first. Unknown UTF-8 is one invalid lexeme; its end must
        // remain a character boundary even though error recovery consumes it all.
        let symbol = [
            "||", "&&", "==", "!=", "<>", "<=", ">=", ".", "(", ")", ",", "?", ":", "-", "!", "=",
            "<", ">", "+", "&", "*", "/", "^",
        ]
        .into_iter()
        .find(|symbol| text[start..].starts_with(symbol));
        if let Some(symbol) = symbol {
            end += symbol.len();
            LexKind::Symbol(symbol)
        } else {
            end += text[start..].chars().next().map_or(0, char::len_utf8);
            LexKind::Invalid
        }
    };
    Lexeme { kind, start, end }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn tokens(tree: &KRaw) -> Vec<&KRaw> {
        let mut stack = vec![tree];
        let mut out = Vec::new();
        while let Some(tree) = stack.pop() {
            match tree {
                KRaw::Token { .. } => out.push(tree),
                KRaw::Node { children, .. } => stack.extend(children.iter().rev()),
            }
        }
        out
    }

    fn shape(tree: &KRaw) -> String {
        match tree {
            KRaw::Node { kind, children, .. } => format!(
                "({kind}{})",
                children
                    .iter()
                    .map(|child| format!(" {}", shape(child)))
                    .collect::<String>()
            ),
            KRaw::Token {
                kind, named, text, ..
            } => {
                if *named {
                    format!("{kind}:{text}")
                } else {
                    kind.to_string()
                }
            }
        }
    }

    fn parse(text: &str) -> KRaw {
        parse_expression(text, Dialect::Formula, 0, 1)
    }

    fn assert_source(tree: &KRaw, source: &str, base: u32, line: u32) {
        let mut previous_end = base;
        let mut errors = 0;
        for token in tokens(tree) {
            let KRaw::Token {
                kind,
                text,
                bytes,
                line: token_line,
                field,
                named,
            } = token
            else {
                unreachable!()
            };
            assert!(bytes.start >= previous_end);
            assert!(bytes.end >= bytes.start);
            assert!(bytes.end <= base + source.len() as u32);
            let start = (bytes.start - base) as usize;
            let end = (bytes.end - base) as usize;
            assert_eq!(
                *token_line,
                line + source[..start].bytes().filter(|&b| b == b'\n').count() as u32
            );
            assert_eq!(*field, None);
            if *kind == "ERROR" {
                errors += 1;
                assert_eq!(text, ERR_TOKEN);
                assert!(!named);
                assert_eq!(end, source.len());
            } else {
                assert_eq!(text, &source[start..end]);
                assert_eq!(*named, matches!(*kind, "identifier" | "literal"));
            }
            previous_end = bytes.end;
        }
        assert!(errors <= 1);
    }

    #[test]
    fn context_examples() {
        let examples = [
            ("{item.Id}", Dialect::Lwc, "path"),
            ("{greeting}", Dialect::Lwc, "identifier"),
            ("{!v.items}", Dialect::Aura, "path"),
            ("{!c.handleClick}", Dialect::Aura, "path"),
            ("{!v.count > 1 ? 'many' : 'one'}", Dialect::Aura, "ternary"),
            ("{!$Label.c.MyLabel}", Dialect::Aura, "path"),
            ("{!IF(ISBLANK(v.x), 'a', 'b')}", Dialect::Aura, "call"),
            ("{!$Record.Name}", Dialect::Formula, "path"),
        ];
        for (source, dialect, expected) in examples {
            let spans = find_expressions(source, dialect);
            assert_eq!(spans.len(), 1, "{source}");
            let (start, end, inner) = spans[0];
            assert_eq!((start, end), (0, source.len()));
            let base = source.len() - inner.len() - 1;
            let tree = parse_expression(inner, dialect, base as u32, 1);
            assert_eq!(tree.kind(), expected, "{source}");
            assert!(!tokens(&tree).iter().any(|t| t.kind() == "ERROR"));
            assert_source(&tree, inner, base as u32, 1);
        }
        for source in [
            "AND(ISPICKVAL(StageName, \"Closed Won\"), ISBLANK(CloseDate))",
            "Amount * 0.1 + IF(Probability > 50, 1, 0)",
            "$Record.Status__c <> PRIORVALUE(Status__c)",
            "NOT(ISNEW()) && ISCHANGED(OwnerId)",
            "TEXT(Type) & \" - \" & Name",
        ] {
            let tree = parse(source);
            assert!(!tokens(&tree).iter().any(|t| t.kind() == "ERROR"));
            assert_source(&tree, source, 0, 1);
        }
    }

    #[test]
    fn construct_shapes() {
        for (source, expected) in [
            ("12.5", "literal:12.5"),
            ("thing", "identifier:thing"),
            (
                "$Label.c.X",
                "(path identifier:$Label . identifier:c . identifier:X)",
            ),
            (
                "if(isblank(v.x), 'a', 'b')",
                "(call IF ( (call ISBLANK ( (path identifier:v . identifier:x) )) , literal:'a' , literal:'b' ))",
            ),
            ("-x", "(unary - identifier:x)"),
            ("!x", "(unary ! identifier:x)"),
            ("NOT x", "(unary NOT identifier:x)"),
            ("(x)", "(paren ( identifier:x ))"),
            ("a + b", "(binary identifier:a + identifier:b)"),
        ] {
            assert_eq!(shape(&parse(source)), expected, "{source}");
        }
        assert_eq!(
            shape(&parse_expression("a ? b : c", Dialect::Aura, 0, 1)),
            "(ternary identifier:a ? identifier:b : identifier:c)"
        );
    }

    #[test]
    fn precedence_and_associativity() {
        assert_eq!(
            shape(&parse("a + b * c")),
            "(binary identifier:a + (binary identifier:b * identifier:c))"
        );
        assert_eq!(
            shape(&parse("a - b - c")),
            "(binary (binary identifier:a - identifier:b) - identifier:c)"
        );
        // ^ shares the multiplicative level, as specified by this grammar.
        assert_eq!(
            shape(&parse("a * b ^ c")),
            "(binary (binary identifier:a * identifier:b) ^ identifier:c)"
        );
        assert_eq!(
            shape(&parse("!a || b && c = d < e + f * g")),
            "(binary (unary ! identifier:a) || (binary identifier:b && (binary identifier:c = (binary identifier:d < (binary identifier:e + (binary identifier:f * identifier:g))))))"
        );
        assert_eq!(
            shape(&parse_expression("a ? b : c ? d : e", Dialect::Aura, 0, 1)),
            "(ternary identifier:a ? identifier:b : (ternary identifier:c ? identifier:d : identifier:e))"
        );
        assert_eq!(
            shape(&parse_expression("a ? b ? c : d : e", Dialect::Aura, 0, 1)),
            "(ternary identifier:a ? (ternary identifier:b ? identifier:c : identifier:d) : identifier:e)"
        );
        for op in ["=", "==", "!=", "<>", "<", "<=", ">", ">=", "/", "&"] {
            assert_eq!(parse(&format!("a {op} b")).kind(), "binary");
        }
    }

    #[test]
    fn literals_escapes_offsets_and_lines() {
        for source in [
            "0",
            "123",
            "1.25",
            ".5",
            "1.",
            "2e10",
            "1.2E-3",
            "TRUE",
            "false",
            "NuLl",
            "'a\\'b'",
            "\"a\\\"b\"",
            "'a\\\\b'",
        ] {
            let tree = parse(source);
            assert_eq!(tree.kind(), "literal", "{source}");
            assert_source(&tree, source, 0, 1);
        }
        let source = "\n if(\n v.x,\n 'hé\\'llo\nthere',\n .5)";
        let tree = parse_expression(source, Dialect::Formula, 97, 8);
        assert_source(&tree, source, 97, 8);
        let KRaw::Token { bytes, line, .. } = tokens(&tree)[0] else {
            unreachable!()
        };
        assert_eq!(bytes, &(99..101));
        assert_eq!(*line, 9);
    }

    #[test]
    fn call_keywords_fold_kinds_but_keep_spelling() {
        for dialect in [Dialect::Aura, Dialect::Formula] {
            let lower = parse_expression("isblank(x)", dialect, 0, 1);
            let upper = parse_expression("ISBLANK(x)", dialect, 0, 1);
            assert_eq!(shape(&lower), shape(&upper));
            assert!(matches!(tokens(&lower)[0], KRaw::Token {
                kind: "ISBLANK", named: false, text, ..
            } if text == "isblank"));
            let arbitrary = parse_expression("custom_fn(x)", dialect, 0, 1);
            assert_eq!(tokens(&arbitrary)[0].kind(), "CUSTOM_FN");
        }
        assert!(std::ptr::eq(call_kind("custom_fn"), call_kind("CUSTOM_FN")));
        assert_eq!(parse("NOT(ISNEW())").kind(), "call");
    }

    #[test]
    fn dialect_restrictions() {
        for source in [
            "1",
            "'x'",
            "true",
            "null",
            "-a",
            "!a",
            "(a)",
            "a+b",
            "f()",
            "a ? b : c",
            "a[0]",
        ] {
            let tree = parse_expression(source, Dialect::Lwc, 13, 4);
            assert!(
                tokens(&tree).iter().any(|t| t.kind() == "ERROR"),
                "{source}"
            );
            assert_source(&tree, source, 13, 4);
        }
        for op in ["or", "AND"] {
            let source = format!("a {op} b");
            assert_eq!(
                parse_expression(&source, Dialect::Aura, 0, 1).kind(),
                "binary"
            );
            assert_eq!(parse(&source).kind(), "expression");
        }
        assert_eq!(parse("OR(a, b)").kind(), "call");
        assert_eq!(parse("AND(a, b)").kind(), "call");
        assert_eq!(parse("a ? b : c").kind(), "expression");
    }

    #[test]
    fn errors_keep_prefix_and_cover_remainder_once() {
        for (source, expected, error_start) in [
            ("a + 'unfinished", "(binary identifier:a + ERROR)", 4),
            ("a + \"unfinished\\", "(binary identifier:a + ERROR)", 4),
            ("(a", "(paren ( identifier:a ERROR)", 2),
            ("a)", "(expression identifier:a ERROR)", 1),
            ("f(a,", "(call F ( identifier:a , ERROR)", 4),
            ("f(a b)", "(call F ( identifier:a ERROR)", 4),
            ("a.", "(path identifier:a . ERROR)", 2),
            ("x @ y + z", "(expression identifier:x ERROR)", 2),
            ("a +", "(binary identifier:a + ERROR)", 3),
            ("f(,x)", "(call F ( ERROR)", 2),
            ("1e+", "ERROR", 0),
            ("é", "ERROR", 0),
        ] {
            let tree = parse_expression(source, Dialect::Formula, 31, 5);
            assert_eq!(shape(&tree), expected, "{source}");
            assert_source(&tree, source, 31, 5);
            assert!(matches!(tokens(&tree).last(), Some(KRaw::Token {
                kind: "ERROR", bytes, ..
            }) if bytes.start == 31 + error_start));
        }
        for source in ["a ? b", "a ?", "a ? b :", "a ? b x : c"] {
            let tree = parse_expression(source, Dialect::Aura, 0, 1);
            assert_eq!(tree.kind(), "ternary");
            assert_source(&tree, source, 0, 1);
            assert_eq!(tokens(&tree).last().unwrap().kind(), "ERROR");
        }
    }

    #[test]
    fn empty_input_is_empty_expression() {
        for dialect in [Dialect::Lwc, Dialect::Aura, Dialect::Formula] {
            for source in ["", " \t\r\n"] {
                assert_eq!(
                    parse_expression(source, dialect, 19, 3),
                    node("expression", vec![])
                );
            }
        }
    }

    #[test]
    fn unrepresentable_offsets_and_lines_saturate() {
        let tree = parse_expression("\nx + y", Dialect::Formula, u32::MAX, u32::MAX);
        for token in tokens(&tree) {
            assert!(matches!(token, KRaw::Token { bytes, line, .. }
                if bytes == &(u32::MAX..u32::MAX) && *line == u32::MAX));
        }
    }

    #[test]
    fn depth_cap_handles_nesting_unary_and_long_binary_chains() {
        for source in [
            "(".repeat(5_000),
            format!("{}x{}", "(".repeat(5_000), ")".repeat(5_000)),
            format!("{}x", "!".repeat(5_000)),
            format!("{}x{}", "f(".repeat(5_000), ")".repeat(5_000)),
            "x+".repeat(5_000),
            format!("{}x", "x ? x : ".repeat(5_000)),
        ] {
            let tree = parse_expression(&source, Dialect::Aura, 10, 3);
            assert_source(&tree, &source, 10, 3);
            assert_eq!(tokens(&tree).last().unwrap().kind(), "ERROR");
            // Traversal and normal drop both finish without overflowing the stack.
            let mut stack = vec![(&tree, 0)];
            while let Some((tree, depth)) = stack.pop() {
                assert!(depth <= MAX_DEPTH + 1);
                if let KRaw::Node { children, .. } = tree {
                    stack.extend(children.iter().map(|child| (child, depth + 1)));
                }
            }
        }
        assert!(
            !tokens(&parse(&format!("{}x{}", "(".repeat(199), ")".repeat(199))))
                .iter()
                .any(|t| t.kind() == "ERROR")
        );
    }

    #[test]
    fn finds_quoted_braces_adjacent_expressions_and_attribute_values() {
        let source = r#""{!IF(v.x, '}', "{escaped\"}" )}{!v.y}""#;
        let spans = find_expressions(source, Dialect::Aura);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0], (1, 32, r#"IF(v.x, '}', "{escaped\"}" )"#));
        assert_eq!(spans[1], (32, 38, "v.y"));
        for (start, end, inner) in spans {
            assert_eq!(inner, &source[start + 2..end - 1]);
        }
        assert_eq!(
            find_expressions("{a}{b}", Dialect::Lwc),
            vec![(0, 3, "a"), (3, 6, "b")]
        );
        assert_eq!(
            find_expressions("'hello {item.Id}'", Dialect::Lwc),
            vec![(7, 16, "item.Id")]
        );
        assert_eq!(
            find_expressions("é {!$Record.Name} {!v.x}", Dialect::Formula),
            vec![(3, 18, "$Record.Name"), (19, 25, "v.x")]
        );
        assert!(find_expressions("{a}", Dialect::Aura).is_empty());
        for source in ["{!x", "{!'x}\\", "{!\"}"] {
            assert!(find_expressions(source, Dialect::Aura).is_empty());
        }
        assert_eq!(
            find_expressions("{!{x}}", Dialect::Aura),
            vec![(0, 6, "{x}")]
        );
        // Aura also tokenizes `{#…}` unbound expressions.
        let (s0, e0, inner0) = find_expressions("x{#v.items}y", Dialect::Aura)[0];
        assert_eq!(&"x{#v.items}y"[s0..e0], "{#v.items}");
        assert_eq!(inner0, "v.items");
        let (s1, e1, inner1) = find_expressions("'{#v.title}'", Dialect::Aura)[0];
        assert_eq!(&"'{#v.title}'"[s1..e1], "{#v.title}");
        assert_eq!(inner1, "v.title");
        let spans = find_expressions("{#IF(a, '{', '{escaped}' )}", Dialect::Aura);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].2, "IF(a, '{', '{escaped}' )");
        assert!(find_expressions("{#x}", Dialect::Formula).is_empty());
    }

    proptest! {
        #[test]
        fn random_ascii_never_panics_and_offsets_stay_in_bounds(
            bytes in prop::collection::vec(0u8..=127, 0..=200),
            base in 0u32..1_000_000,
            line in 1u32..1_000,
        ) {
            let source = String::from_utf8(bytes).unwrap();
            for dialect in [Dialect::Lwc, Dialect::Aura, Dialect::Formula] {
                let tree = parse_expression(&source, dialect, base, line);
                assert_source(&tree, &source, base, line);
                for (start, end, inner) in find_expressions(&source, dialect) {
                    prop_assert!(start < end && end <= source.len());
                    let prefix = if dialect == Dialect::Lwc { 1 } else { 2 };
                    prop_assert_eq!(inner, &source[start + prefix..end - 1]);
                }
            }
        }

        #[test]
        fn random_utf8_preserves_character_boundaries(
            chars in prop::collection::vec(any::<char>(), 0..=200),
        ) {
            let source: String = chars.into_iter().collect();
            for dialect in [Dialect::Lwc, Dialect::Aura, Dialect::Formula] {
                let tree = parse_expression(&source, dialect, 23, 7);
                assert_source(&tree, &source, 23, 7);
                for (start, end, inner) in find_expressions(&source, dialect) {
                    let prefix = if dialect == Dialect::Lwc { 1 } else { 2 };
                    prop_assert_eq!(inner, &source[start + prefix..end - 1]);
                }
            }
        }
    }
}
