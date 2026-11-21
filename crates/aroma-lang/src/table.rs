//! Stage 1 (D21): one shared CST -> `KRaw` walk driven by `LangSpec` tables, unit
//! extraction, ERROR/MISSING policy, depth/size/time caps (D11).
//!
//! Policy, applied to every node in this order:
//! 1. zero-width nodes (tree-sitter MISSING nodes) are dropped and counted in
//!    `AdapterDiag::missing_nodes`; they never invent evidence;
//! 2. an ERROR node becomes one anonymous token `#ERR` (kind `"ERROR"`), its bytes are
//!    added to `damaged_bytes` and its subtree is not visited;
//! 3. extras and `spec.comment_kinds` are dropped, so comments never shift child slots;
//! 4. a node without children, or of an `atomic_kinds` kind, becomes one `Token` with its
//!    full source text;
//! 5. any other node deeper than `limits.max_depth` (the walk root has depth 0) becomes one
//!    `#ERR` token, its bytes count as damaged and `depth_capped` is set (so no `KRaw::Node`
//!    is deeper than `max_depth` and no token deeper than `max_depth + 1`);
//! 6. every other node becomes a `Node` (dropped when none of its children survive).
//!
//! Every walk uses an explicit stack, so input nesting never grows the Rust call stack.

use crate::kraw::{KRaw, ERR_TOKEN};
use crate::spec::LangSpec;
use aroma_core::unit::UnitKind;
use std::ops::Range;
use std::time::{Duration, Instant};
use tree_sitter::{Node, ParseOptions, ParseState, Parser, Point, Tree};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_depth: usize,
    pub max_file_bytes: usize,
    pub parse_timeout_micros: u64,
    pub max_damage: f64,
}

impl Limits {
    /// Reference `MAX_DEPTH` 1000, 2 MiB files, 2 s parse budget, 10% damaged bytes (D11).
    pub const DEFAULT: Limits = Limits {
        max_depth: 1000,
        max_file_bytes: 2 * 1024 * 1024,
        parse_timeout_micros: 2_000_000,
        max_damage: 0.10,
    };
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AdapterDiag {
    pub error_nodes: u32,
    pub missing_nodes: u32,
    pub damaged_bytes: u32,
    pub depth_capped: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("file too large: {0} bytes")]
    TooLarge(usize),
    #[error("parse timed out")]
    Timeout,
    #[error("grammar error: {0}")]
    Language(String),
}

/// Parse `src` with the spec's grammar. Files over `limits.max_file_bytes` are refused
/// before parsing; the parse is halted once `limits.parse_timeout_micros` have elapsed
/// (checked by tree-sitter's progress callback every 100 parser operations).
pub fn parse(spec: &LangSpec, src: &str, limits: &Limits) -> Result<Tree, AdapterError> {
    if src.len() > limits.max_file_bytes {
        return Err(AdapterError::TooLarge(src.len()));
    }
    let mut parser = Parser::new();
    parser
        .set_language(&(spec.language)())
        .map_err(|e| AdapterError::Language(e.to_string()))?;
    let budget = Duration::from_micros(limits.parse_timeout_micros);
    let start = Instant::now();
    let mut halt = |_: &ParseState| start.elapsed() > budget;
    let preprocessed = (spec.preprocess)(src);
    assert_eq!(
        preprocessed.len(),
        src.len(),
        "preprocessing must preserve byte length"
    );
    let bytes = preprocessed.as_bytes();
    let mut input = |offset: usize, _: Point| -> &[u8] { bytes.get(offset..).unwrap_or(&[]) };
    parser
        .parse_with_options(
            &mut input,
            None,
            Some(ParseOptions::new().progress_callback(&mut halt)),
        )
        .ok_or(AdapterError::Timeout)
}

fn span(n: Node<'_>) -> Range<u32> {
    n.start_byte() as u32..n.end_byte() as u32
}

fn line(n: Node<'_>) -> u32 {
    n.start_position().row as u32 + 1
}

fn err_token(n: Node<'_>, field: Option<&'static str>) -> KRaw {
    KRaw::Token {
        kind: "ERROR",
        named: false,
        text: ERR_TOKEN.to_owned(),
        bytes: span(n),
        line: line(n),
        field,
    }
}

/// What the walk does with one CST node.
enum Visit {
    Skip,
    Emit(KRaw),
    Descend,
}

fn visit(
    n: Node<'_>,
    field: Option<&'static str>,
    depth: usize,
    src: &[u8],
    spec: &LangSpec,
    limits: &Limits,
    diag: &mut AdapterDiag,
) -> Visit {
    if n.start_byte() == n.end_byte() {
        diag.missing_nodes += 1;
        return Visit::Skip;
    }
    if n.is_error() {
        diag.error_nodes += 1;
        diag.damaged_bytes += (n.end_byte() - n.start_byte()) as u32;
        return Visit::Emit(err_token(n, field));
    }
    let kind = n.kind();
    if n.is_extra() || spec.comment_kinds.contains(&kind) {
        return Visit::Skip;
    }
    if n.child_count() == 0 || spec.atomic_kinds.contains(&kind) {
        let text = String::from_utf8_lossy(&src[n.start_byte()..n.end_byte()]).into_owned();
        return Visit::Emit(KRaw::Token {
            kind,
            named: n.is_named(),
            text,
            bytes: span(n),
            line: line(n),
            field,
        });
    }
    if depth > limits.max_depth {
        diag.depth_capped = true;
        diag.damaged_bytes += (n.end_byte() - n.start_byte()) as u32;
        return Visit::Emit(err_token(n, field));
    }
    Visit::Descend
}

struct Frame {
    kind: &'static str,
    field: Option<&'static str>,
    children: Vec<KRaw>,
}

fn finish(frame: Frame) -> Option<KRaw> {
    (!frame.children.is_empty()).then_some(KRaw::Node {
        kind: frame.kind,
        field: frame.field,
        children: frame.children,
    })
}

/// Stage 1 walk of `node` (explicit stack, no recursion). The returned root has field
/// `None`; children carry their tree-sitter field names. `None` when nothing survives
/// the policy above (zero-width node, comment, or a subtree of only such nodes).
pub fn to_kraw(
    node: Node<'_>,
    src: &[u8],
    spec: &LangSpec,
    limits: &Limits,
    diag: &mut AdapterDiag,
) -> Option<KRaw> {
    match visit(node, None, 0, src, spec, limits, diag) {
        Visit::Skip => return None,
        Visit::Emit(k) => return Some(k),
        Visit::Descend => {}
    }
    let mut cursor = node.walk();
    let mut stack = vec![Frame {
        kind: node.kind(),
        field: None,
        children: Vec::new(),
    }];
    // The root is a non-leaf node (Descend), so it has at least one child.
    let mut has_node = cursor.goto_first_child();
    loop {
        if has_node {
            let n = cursor.node();
            let field = cursor.field_name();
            match visit(n, field, stack.len(), src, spec, limits, diag) {
                Visit::Skip => {}
                Visit::Emit(k) => stack.last_mut()?.children.push(k),
                Visit::Descend => {
                    stack.push(Frame {
                        kind: n.kind(),
                        field,
                        children: Vec::new(),
                    });
                    has_node = cursor.goto_first_child();
                    continue;
                }
            }
            has_node = cursor.goto_next_sibling();
            continue;
        }
        // All children of the innermost open frame are done.
        let frame = stack.pop()?;
        let done = finish(frame);
        match stack.last_mut() {
            None => return done,
            Some(parent) => {
                parent.children.extend(done);
                cursor.goto_parent();
                has_node = cursor.goto_next_sibling();
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExtractedUnit {
    pub kind: UnitKind,
    pub bytes: Range<u32>,
    /// 1-based, end exclusive.
    pub lines: Range<u32>,
    /// Stage 1 tree of the selected body and any leading evidence.
    pub body: KRaw,
    /// `damaged_bytes / (leading + body bytes)` of the selected walks (D11: > `max_damage` is skipped).
    pub damage: f64,
}

/// Pre-order walk of the whole tree (explicit stack via the cursor), calling `f` on every
/// node; nodes inside ERROR subtrees are visited too.
fn for_each_node<'t>(tree: &'t Tree, mut f: impl FnMut(Node<'t>)) {
    let mut cursor = tree.walk();
    loop {
        f(cursor.node());
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return;
            }
        }
    }
}

/// The structural evidence selected for one unit by a language adapter.
pub struct UnitShape<'t> {
    pub kind: UnitKind,
    pub body: Node<'t>,
    /// Evidence preceding the body, in source order (for example, decorators).
    pub leading: Vec<Node<'t>>,
}

/// Default unit selection: kind table plus body field, with no leading evidence.
pub fn table_unit<'t>(spec: &LangSpec, node: Node<'t>, _src: &[u8]) -> Option<UnitShape<'t>> {
    let &(_, kind) = spec.units.iter().find(|(k, _)| *k == node.kind())?;
    Some(UnitShape {
        kind,
        body: node.child_by_field_name(spec.body_field)?,
        leading: Vec::new(),
    })
}

/// Units selected by `spec.unit`, in source (pre-)order; nested units included.
pub fn extract_units(
    tree: &Tree,
    src: &str,
    spec: &LangSpec,
    limits: &Limits,
) -> Vec<ExtractedUnit> {
    let mut out = Vec::new();
    for_each_node(tree, |n| {
        let Some(shape) = (spec.unit)(spec, n, src.as_bytes()) else {
            return;
        };
        let mut diag = AdapterDiag::default();
        let Some(body) = to_kraw(shape.body, src.as_bytes(), spec, limits, &mut diag) else {
            return;
        };
        let mut body_len = shape.body.end_byte() - shape.body.start_byte();
        let start = shape.leading.first().copied().unwrap_or(n);
        let kraw = if shape.leading.is_empty() {
            body
        } else {
            let mut children = Vec::new();
            for leading in shape.leading {
                body_len += leading.end_byte() - leading.start_byte();
                children.extend(to_kraw(leading, src.as_bytes(), spec, limits, &mut diag));
            }
            children.push(body);
            KRaw::Node {
                kind: "unit",
                field: None,
                children,
            }
        };
        out.push(ExtractedUnit {
            kind: shape.kind,
            bytes: start.start_byte() as u32..n.end_byte() as u32,
            lines: line(start)..n.end_position().row as u32 + 2,
            body: kraw,
            damage: f64::from(diag.damaged_bytes) / body_len.max(1) as f64,
        });
    });
    out
}

/// Names of the types declared in one file (`spec.type_decl_kinds`, field `name`), in
/// source order, ASCII-folded when `spec.fold_case`. They become repository globals.
pub fn declared_types(tree: &Tree, src: &str, spec: &LangSpec) -> Vec<String> {
    let mut out = Vec::new();
    for_each_node(tree, |n| {
        if !spec.type_decl_kinds.contains(&n.kind()) {
            return;
        }
        if let Some(name) = n.child_by_field_name("name") {
            let text = &src[name.start_byte()..name.end_byte()];
            out.push(if spec.fold_case {
                text.to_ascii_lowercase()
            } else {
                text.to_owned()
            });
        }
    });
    out
}
