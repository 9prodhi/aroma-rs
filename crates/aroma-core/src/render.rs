//! Rendering (similar.py `ast_to_code_with_full_lines`, note 04 §3.7b), adapted from the
//! reference's per-token line numbers to the byte spans of our leaves.
//!
//! Every source line of the unit is classified by the leaves whose byte span touches it
//! (a leaf spanning several lines, e.g. a text block, touches all of them):
//!
//! ```text
//!   kept    holds at least one kept leaf            -> printed verbatim
//!   dropped holds leaves, none of them kept          -> part of an omitted run
//!   bare    holds no leaf (blank, comment, `}` ...)  -> printed, unless it lies inside
//!                                                       an omitted run
//! ```
//!
//! Each maximal run of dropped lines (bare lines inside the run included) is printed as
//! ONE `// your code ...` line, also at the start and at the end of the unit. Bare lines
//! between an omitted run and the next kept line (closing braces) are printed after the
//! placeholder. Output is line-granular like the reference, so a printed line may show
//! tokens that were not selected; `kept_spans` exposes the exact kept byte ranges.

use crate::spt::{Elem, NodeId, Spt};
use std::ops::Range;

/// The placeholder printed for each run of omitted lines.
pub const OMITTED_SLASH: &str = "// your code ...";
/// The placeholder for omitted markup source lines.
pub const OMITTED_MARKUP: &str = "<!-- your code ... -->";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum LineClass {
    Bare,
    Dropped,
    Kept,
}

fn is_kept(kept: &[bool], leaf: usize) -> bool {
    kept.get(leaf).copied().unwrap_or(false)
}

/// 1-based lines `[first, last]` touched by a leaf: its start line plus the newlines
/// inside its byte span when that span lies inside `source` (else its start line only).
fn leaf_lines(source: &str, line: u32, bytes: &Range<u32>) -> (u32, u32) {
    let extra = source
        .get(bytes.start as usize..bytes.end as usize)
        .map_or(0, |s| s.bytes().filter(|&b| b == b'\n').count() as u32);
    (line, line.saturating_add(extra))
}

/// `source` is the whole file; `lines` the unit's 1-based, end-exclusive line range.
/// Leaves outside `lines` are ignored; `kept` entries missing at the end count as
/// dropped.
pub fn render_full_lines(
    source: &str,
    lines: Range<u32>,
    spt: &Spt,
    kept: &[bool],
    omission_marker: &str,
) -> String {
    let start = lines.start.max(1);
    let end = lines.end.max(start);
    let mut class = vec![LineClass::Bare; (end - start) as usize];
    for (i, leaf) in spt.leaves.iter().enumerate() {
        let c = if is_kept(kept, i) {
            LineClass::Kept
        } else {
            LineClass::Dropped
        };
        let (first, last) = leaf_lines(source, leaf.line, &leaf.bytes);
        for ln in first.max(start)..=last.min(end - 1) {
            let slot = &mut class[(ln - start) as usize];
            *slot = (*slot).max(c);
        }
    }

    let mut out = String::new();
    let mut omitted = false; // an omitted run is open and not yet printed
    let mut held: Vec<&str> = Vec::new(); // bare lines seen since that run started
    let unit_lines = source
        .lines()
        .enumerate()
        .skip(start as usize - 1)
        .take((end - start) as usize);
    for (i, text) in unit_lines {
        match class[i + 1 - start as usize] {
            LineClass::Kept => {
                if omitted {
                    push_line(&mut out, omission_marker);
                    omitted = false;
                }
                for h in held.drain(..) {
                    push_line(&mut out, h);
                }
                push_line(&mut out, text);
            }
            LineClass::Dropped => {
                omitted = true;
                held.clear();
            }
            LineClass::Bare if omitted => held.push(text),
            LineClass::Bare => push_line(&mut out, text),
        }
    }
    if omitted {
        push_line(&mut out, omission_marker);
    }
    for h in held {
        push_line(&mut out, h);
    }
    out
}

fn push_line(out: &mut String, line: &str) {
    out.push_str(line);
    out.push('\n');
}

/// Stale-source fallback: the stored tree's tokens joined by single spaces. A dropped
/// leaf prints as `...`; a sub-tree whose leaves are all dropped prints as one `...`
/// (the reference removes such sub-trees, `prune_ast`); consecutive `...` collapse into
/// one. Keywords of kept structure and of leafless sub-trees print as they are; empty
/// keywords print nothing.
pub fn render_tokens(spt: &Spt, kept: &[bool]) -> String {
    let mut out: Vec<&str> = Vec::new();
    if (spt.root as usize) < spt.nodes.len() {
        emit_tokens(spt, spt.root, kept, &mut out);
    }
    out.join(" ")
}

/// (has any leaf, has a kept leaf) for the sub-tree under `e`.
fn leaf_presence(spt: &Spt, e: &Elem, kept: &[bool]) -> (bool, bool) {
    match *e {
        Elem::Keyword(_) => (false, false),
        Elem::Leaf(l) => (true, is_kept(kept, l as usize)),
        Elem::Tree(n) => spt.nodes.get(n as usize).map_or((false, false), |node| {
            node.children.iter().fold((false, false), |acc, c| {
                let (any, k) = leaf_presence(spt, c, kept);
                (acc.0 || any, acc.1 || k)
            })
        }),
    }
}

fn push_token<'a>(out: &mut Vec<&'a str>, tok: &'a str) {
    if tok.is_empty() || (tok == "..." && out.last() == Some(&"...")) {
        return;
    }
    out.push(tok);
}

fn emit_tokens<'a>(spt: &'a Spt, n: NodeId, kept: &[bool], out: &mut Vec<&'a str>) {
    let Some(node) = spt.nodes.get(n as usize) else {
        return;
    };
    for c in &node.children {
        match *c {
            Elem::Keyword(ref k) => push_token(out, k),
            Elem::Leaf(l) => match spt.leaves.get(l as usize) {
                Some(leaf) if is_kept(kept, l as usize) => push_token(out, &leaf.text),
                Some(_) => push_token(out, "..."),
                None => {}
            },
            Elem::Tree(t) => match leaf_presence(spt, c, kept) {
                (true, false) => push_token(out, "..."),
                _ => emit_tokens(spt, t, kept, out),
            },
        }
    }
}

/// Byte spans of the kept leaves, in source order; leaves with an unknown (empty) span
/// are skipped.
pub fn kept_spans(spt: &Spt, kept: &[bool]) -> Vec<Range<u32>> {
    spt.leaves
        .iter()
        .enumerate()
        .filter(|&(i, l)| is_kept(kept, i) && l.bytes.start < l.bytes.end)
        .map(|(_, l)| l.bytes.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spt::{Leaf, Raw};

    fn leaf(s: &str, line: u32, start: u32) -> Raw {
        Raw::Leaf(Leaf {
            text: s.into(),
            is_var: false,
            binding: None,
            bytes: start..start + s.len() as u32,
            line,
        })
    }

    const SRC: &str = "void m() {\n  a();\n  b();\n  c();\n}\n";

    fn spt() -> Spt {
        // leaves a (line 2), b (line 3), c (line 4)
        Spt::build(Raw::List(vec![
            Raw::Keyword("{".into()),
            Raw::List(vec![leaf("a", 2, 13), Raw::Keyword("();".into())]),
            Raw::List(vec![leaf("b", 3, 20), Raw::Keyword("();".into())]),
            Raw::List(vec![leaf("c", 4, 27), Raw::Keyword("();".into())]),
            Raw::Keyword("}".into()),
        ]))
        .unwrap()
    }

    #[test]
    fn dropped_line_runs_become_one_placeholder() {
        let out = render_full_lines(SRC, 2..5, &spt(), &[true, false, true], OMITTED_SLASH);
        assert_eq!(out, "  a();\n// your code ...\n  c();\n");
    }

    #[test]
    fn lines_outside_the_unit_are_ignored() {
        let out = render_full_lines(SRC, 3..4, &spt(), &[true, true, true], OMITTED_SLASH);
        assert_eq!(out, "  b();\n");
    }

    #[test]
    fn bare_lines_print_and_trailing_runs_get_a_placeholder() {
        // Whole unit 1..6: line 1 and 5 hold no leaf.
        let out = render_full_lines(SRC, 1..6, &spt(), &[true, false, false], OMITTED_SLASH);
        assert_eq!(out, "void m() {\n  a();\n// your code ...\n}\n");
    }

    #[test]
    fn short_masks_and_empty_ranges() {
        assert_eq!(
            render_full_lines(SRC, 2..5, &spt(), &[true], OMITTED_SLASH),
            "  a();\n// your code ...\n"
        );
        assert_eq!(
            render_full_lines(SRC, 3..3, &spt(), &[true; 3], OMITTED_SLASH),
            ""
        );
        assert_eq!(
            render_full_lines(SRC, 0..0, &spt(), &[true; 3], OMITTED_SLASH),
            ""
        );
    }

    #[test]
    fn tokens_and_spans() {
        let s = spt();
        assert_eq!(
            render_tokens(&s, &[true, false, true]),
            "{ a (); ... c (); }"
        );
        assert_eq!(render_tokens(&s, &[false, false, true]), "{ ... c (); }");
        assert_eq!(render_tokens(&s, &[]), "{ ... }");
        assert_eq!(kept_spans(&s, &[true, false, true]), [13..14, 27..28]);
        assert_eq!(kept_spans(&s, &[true, false]), kept_spans(&s, &[true]));
        assert_eq!(kept_spans(&s, &[false, true, true]), [20..21, 27..28]);
    }

    #[test]
    fn markup_omission_marker_is_used_for_dropped_lines() {
        let spt = Spt::build(Raw::List(vec![Raw::Leaf(Leaf {
            text: "hello".into(),
            is_var: false,
            binding: None,
            bytes: 0..5,
            line: 1,
        })]))
        .unwrap();
        assert_eq!(
            render_full_lines("hello\n", 1..2, &spt, &[false], OMITTED_MARKUP),
            "<!-- your code ... -->\n"
        );
    }
}
