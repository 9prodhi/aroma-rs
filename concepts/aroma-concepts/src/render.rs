//! Rendering (similar.py `ast_to_code_with_full_lines`): print every source line of the
//! first cluster member that holds a kept leaf; collapse each run of dropped lines into
//! one `// your code ...` placeholder. Line-granular, so retained lines may show tokens
//! that were not selected; expose the kept byte ranges separately for precise UIs.

use crate::spt::Spt;
use std::collections::BTreeSet;
use std::ops::Range;

pub fn render_full_lines(source: &str, unit_lines: Range<u32>, spt: &Spt, kept: &[bool]) -> String {
    let keep: BTreeSet<u32> = spt
        .leaves
        .iter()
        .zip(kept)
        .filter(|(_, &k)| k)
        .map(|(l, _)| l.line)
        .collect();
    let mut out = String::new();
    let mut gap = false;
    for (i, line) in source.lines().enumerate() {
        let ln = i as u32 + 1;
        if !unit_lines.contains(&ln) {
            continue;
        }
        if keep.contains(&ln) {
            if gap {
                out.push_str("// your code ...\n");
                gap = false;
            }
            out.push_str(line);
            out.push('\n');
        } else {
            gap = true;
        }
    }
    out
}

pub fn kept_spans(spt: &Spt, kept: &[bool]) -> Vec<Range<usize>> {
    spt.leaves
        .iter()
        .zip(kept)
        .filter(|(_, &k)| k)
        .map(|(l, _)| l.bytes.clone())
        .collect()
}
