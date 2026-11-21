//! Rendering (`ast_to_code_with_full_lines` adapted to byte spans), the stale-source
//! token fallback, kept spans and `explain`, on hand-made trees whose leaves carry real
//! byte spans and lines into hand-written sources.

use aroma_core::render::{kept_spans, render_full_lines, render_tokens, OMITTED_SLASH};
use aroma_core::spt::{Leaf, Raw, Spt};

/// Builds `Raw` tokens in source order, locating each one in `src` after the previous.
struct Doc<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Doc<'a> {
    fn new(src: &'a str) -> Doc<'a> {
        Doc { src, pos: 0 }
    }

    fn find(&mut self, text: &str) -> (u32, u32) {
        let at = self.pos + self.src[self.pos..].find(text).expect("token in source");
        self.pos = at + text.len();
        let line = 1 + self.src[..at].matches('\n').count() as u32;
        (at as u32, line)
    }

    fn leaf(&mut self, text: &str) -> Raw {
        let (at, line) = self.find(text);
        Raw::Leaf(Leaf {
            text: text.into(),
            is_var: false,
            binding: None,
            bytes: at..at + text.len() as u32,
            line,
        })
    }

    fn var(&mut self, text: &str) -> Raw {
        match self.leaf(text) {
            Raw::Leaf(mut l) => {
                l.is_var = true;
                Raw::Leaf(l)
            }
            other => other,
        }
    }

    fn kw(&mut self, text: &str) -> Raw {
        self.find(text);
        Raw::Keyword(text.into())
    }
}

fn list(items: Vec<Raw>) -> Raw {
    Raw::List(items)
}

/// Mask keeping exactly the leaves on the given 1-based lines.
fn mask(spt: &Spt, keep_lines: &[u32]) -> Vec<bool> {
    spt.leaves
        .iter()
        .map(|l| keep_lines.contains(&l.line))
        .collect()
}

// ---------------------------------------------------------------------------------
// Case 1: a copy loop; the query matched the while loop only.

const COPY: &str = "\
class Io {
  void copy(InputStream in, OutputStream out) throws IOException {
    byte[] buf = new byte[8192];
    int n;

    while ((n = in.read(buf)) > 0) {
      out.write(buf, 0, n);
    }
    in.close();
    out.close();
  }
}
";

fn copy_spt() -> Spt {
    let mut d = Doc::new(COPY);
    let header = list(vec![
        d.kw("void"),
        d.leaf("copy"),
        d.kw("("),
        d.leaf("InputStream"),
        d.var("in"),
        d.kw(","),
        d.leaf("OutputStream"),
        d.var("out"),
        d.kw(")"),
        d.kw("throws"),
        d.leaf("IOException"),
    ]);
    let open = d.kw("{");
    let decl_buf = list(vec![
        d.kw("byte[]"),
        d.var("buf"),
        d.kw("="),
        d.kw("new"),
        d.kw("byte["),
        d.leaf("8192"),
        d.kw("];"),
    ]);
    let decl_n = list(vec![d.kw("int"), d.var("n"), d.kw(";")]);
    let cond = list(vec![
        d.kw("while"),
        d.kw("(("),
        d.var("n"),
        d.kw("="),
        d.var("in"),
        d.kw("."),
        d.leaf("read"),
        d.kw("("),
        d.var("buf"),
        d.kw("))"),
        d.kw(">"),
        d.leaf("0"),
        d.kw(")"),
    ]);
    let body = list(vec![
        d.kw("{"),
        d.var("out"),
        d.kw("."),
        d.leaf("write"),
        d.kw("("),
        d.var("buf"),
        d.kw(","),
        d.leaf("0"),
        d.kw(","),
        d.var("n"),
        d.kw(");"),
        d.kw("}"),
    ]);
    let close_in = list(vec![d.var("in"), d.kw("."), d.leaf("close"), d.kw("();")]);
    let close_out = list(vec![d.var("out"), d.kw("."), d.leaf("close"), d.kw("();")]);
    let end = d.kw("}");
    Spt::build(list(vec![
        header,
        list(vec![
            open,
            decl_buf,
            decl_n,
            list(vec![cond, body]),
            close_in,
            close_out,
            end,
        ]),
    ]))
    .unwrap()
}

#[test]
fn copy_loop_full_lines() {
    let spt = copy_spt();
    let kept = mask(&spt, &[6, 7]);
    // Unit = lines 2..=11 (the method); line 1 and 12 belong to the class.
    let out = render_full_lines(COPY, 2..12, &spt, &kept, OMITTED_SLASH);
    insta::assert_snapshot!(out, @r"
    // your code ...

        while ((n = in.read(buf)) > 0) {
          out.write(buf, 0, n);
        }
    // your code ...
      }
    ");
}

#[test]
fn copy_loop_tokens_and_spans() {
    let spt = copy_spt();
    let kept = mask(&spt, &[6, 7]);
    assert_eq!(
        render_tokens(&spt, &kept),
        "... { ... while (( n = in . read ( buf )) > 0 ) { out . write ( buf , 0 , n ); } ... }"
    );
    let spans = kept_spans(&spt, &kept);
    let texts: Vec<&str> = spans
        .iter()
        .map(|r| &COPY[r.start as usize..r.end as usize])
        .collect();
    assert_eq!(
        texts,
        ["n", "in", "read", "buf", "0", "out", "write", "buf", "0", "n"]
    );
}

// ---------------------------------------------------------------------------------
// Case 2: a leaf spanning three lines (Java text block / multi-line string literal).

const QUERY: &str = "  void run() {
    String q = \"\"\"
        SELECT Id
        FROM Account\"\"\";
    db.query(q);
  }
";

fn query_spt() -> Spt {
    let mut d = Doc::new(QUERY);
    let open = d.kw("{");
    let decl = list(vec![
        d.leaf("String"),
        d.var("q"),
        d.kw("="),
        d.leaf("\"\"\"\n        SELECT Id\n        FROM Account\"\"\""),
        d.kw(";"),
    ]);
    let call = list(vec![
        d.var("db"),
        d.kw("."),
        d.leaf("query"),
        d.kw("("),
        d.var("q"),
        d.kw(");"),
    ]);
    let close = d.kw("}");
    Spt::build(list(vec![open, decl, call, close])).unwrap()
}

#[test]
fn multi_line_leaf_keeps_every_line_it_spans() {
    let spt = query_spt();
    assert_eq!(spt.leaves.len(), 6);
    assert_eq!(spt.leaves[2].line, 2);
    // Keep the declaration (String, q, literal), drop the call (db, query, q).
    let kept = [true, true, true, false, false, false];
    let out = render_full_lines(QUERY, 1..7, &spt, &kept, OMITTED_SLASH);
    insta::assert_snapshot!(out, @r#"
      void run() {
        String q = """
            SELECT Id
            FROM Account""";
    // your code ...
      }
    "#);
}

// ---------------------------------------------------------------------------------
// Case 3: tokens of nested structure with a leafless `{}` sub-tree.

fn if_else_spt() -> Spt {
    let src = "if (ready) { start(job); } else { }";
    let mut d = Doc::new(src);
    Spt::build(list(vec![
        d.kw("if"),
        list(vec![d.kw("("), d.var("ready"), d.kw(")")]),
        list(vec![
            d.kw("{"),
            list(vec![d.leaf("start"), d.kw("("), d.var("job"), d.kw(");")]),
            d.kw("}"),
        ]),
        d.kw("else"),
        list(vec![d.kw("{"), d.kw("}")]),
    ]))
    .unwrap()
}

#[test]
fn tokens_collapse_dropped_subtrees_and_keep_leafless_ones() {
    let spt = if_else_spt();
    assert_eq!(spt.leaves.len(), 3);
    let cases = [
        [true, true, true],
        [true, false, false],
        [false, true, false],
        [false, false, false],
    ];
    let out: Vec<String> = cases
        .iter()
        .map(|k| format!("{k:?} -> {}", render_tokens(&spt, k)))
        .collect();
    insta::assert_snapshot!(out.join("\n"), @r"
    [true, true, true] -> if ( ready ) { start ( job ); } else { }
    [true, false, false] -> if ( ready ) ... else { }
    [false, true, false] -> if ... { start ( ... ); } else { }
    [false, false, false] -> if ... else { }
    ");
}

// ---------------------------------------------------------------------------------
// Case 4: placeholders at both ends, bare lines and CRLF sources.

const CRLF: &str =
    "void f() {\r\n  int a = 1;\r\n\r\n  int b = 2;\r\n  // note\r\n  int c = 3;\r\n}\r\n";

/// `int v = n;`
fn int_decl(d: &mut Doc, v: &str, n: &str) -> Raw {
    list(vec![d.kw("int"), d.var(v), d.kw("="), d.leaf(n), d.kw(";")])
}

fn crlf_spt() -> Spt {
    let mut d = Doc::new(CRLF);
    let header = list(vec![d.kw("void"), d.leaf("f"), d.kw("()")]);
    let open = d.kw("{");
    let a = int_decl(&mut d, "a", "1");
    let b = int_decl(&mut d, "b", "2");
    let c = int_decl(&mut d, "c", "3");
    let close = d.kw("}");
    Spt::build(list(vec![header, list(vec![open, a, b, c, close])])).unwrap()
}

#[test]
fn placeholders_bare_lines_and_crlf() {
    let spt = crlf_spt();
    let only_b = mask(&spt, &[4]);
    let only_a_c = mask(&spt, &[2, 6]);
    let all = vec![true; spt.leaves.len()];
    let none = vec![false; spt.leaves.len()];
    let out = [
        (
            "only b",
            render_full_lines(CRLF, 1..8, &spt, &only_b, OMITTED_SLASH),
        ),
        (
            "a and c",
            render_full_lines(CRLF, 1..8, &spt, &only_a_c, OMITTED_SLASH),
        ),
        (
            "all",
            render_full_lines(CRLF, 1..8, &spt, &all, OMITTED_SLASH),
        ),
        (
            "none",
            render_full_lines(CRLF, 1..8, &spt, &none, OMITTED_SLASH),
        ),
    ]
    .iter()
    .map(|(name, text)| format!("== {name}\n{text}"))
    .collect::<String>();
    insta::assert_snapshot!(out, @r"
    == only b
    // your code ...

      int b = 2;
      // note
    // your code ...
    }
    == a and c
    // your code ...
      int a = 1;

    // your code ...
      // note
      int c = 3;
    }
    == all
    void f() {
      int a = 1;

      int b = 2;
      // note
      int c = 3;
    }
    == none
    // your code ...
    }
    ");
}

// ---------------------------------------------------------------------------------
// explain

use aroma_core::explain::{explain, Explanation};
use aroma_core::features::{featurize, unit_bag, FeatureConfig};
use aroma_core::lang::Lang;
use aroma_core::multiset::jaccard;
use std::collections::BTreeSet;

fn feature_set(spt: &Spt, mask: Option<&[bool]>) -> BTreeSet<String> {
    featurize(spt, &FeatureConfig::DEFAULT, mask)
        .into_iter()
        .flatten()
        .collect()
}

fn two(a: &str, b: &str) -> Spt {
    Spt::build(list(vec![
        Raw::Keyword("{".into()),
        Raw::Leaf(Leaf {
            text: a.into(),
            is_var: false,
            binding: None,
            bytes: 0..0,
            line: 1,
        }),
        Raw::Keyword(";".into()),
        Raw::Leaf(Leaf {
            text: b.into(),
            is_var: false,
            binding: None,
            bytes: 0..0,
            line: 1,
        }),
        Raw::Keyword("}".into()),
    ]))
    .unwrap()
}

fn three(a: &str, b: &str, c: &str) -> Spt {
    let leaf = |t: &str| {
        Raw::Leaf(Leaf {
            text: t.into(),
            is_var: false,
            binding: None,
            bytes: 0..0,
            line: 1,
        })
    };
    Spt::build(list(vec![
        Raw::Keyword("{".into()),
        leaf(a),
        Raw::Keyword(";".into()),
        leaf(b),
        Raw::Keyword(";".into()),
        leaf(c),
        Raw::Keyword("}".into()),
    ]))
    .unwrap()
}

#[test]
fn explain_small_trees_exactly() {
    let q = two("a", "b");
    let c = three("a", "b", "c");
    let e = explain(
        Lang::Java,
        &FeatureConfig::DEFAULT,
        &q,
        &c,
        &[true, true, false],
    );
    assert_eq!(
        e,
        Explanation {
            matched: vec!["a".into(), "a>>b".into(), "b".into()],
            query_only: vec!["{#;#}2>a".into(), "{#;#}4>b".into()],
            candidate_only: vec!["{#;#;#}2>a".into(), "{#;#;#}4>b".into()],
            // Σmin = a + b + 2·(a>>b) = 4; Σmax = 4 + 4 distinct parent features = 8.
            jaccard: 0.5,
        }
    );
}

#[test]
fn explain_partitions_the_feature_sets() {
    let q = query_spt();
    let c = copy_spt();
    let kept = mask(&c, &[6, 7]);
    let e = explain(Lang::Java, &FeatureConfig::DEFAULT, &q, &c, &kept);
    let qs = feature_set(&q, None);
    let cs = feature_set(&c, Some(&kept));
    let matched: BTreeSet<String> = e.matched.iter().cloned().collect();
    let q_only: BTreeSet<String> = e.query_only.iter().cloned().collect();
    let c_only: BTreeSet<String> = e.candidate_only.iter().cloned().collect();
    // Sorted and deduplicated.
    for v in [&e.matched, &e.query_only, &e.candidate_only] {
        assert!(v.windows(2).all(|w| w[0] < w[1]));
    }
    assert_eq!(&matched | &q_only, qs);
    assert_eq!(&matched | &c_only, cs);
    assert!(matched.is_disjoint(&q_only) && matched.is_disjoint(&c_only));
    let qb = unit_bag(Lang::Java, &featurize(&q, &FeatureConfig::DEFAULT, None));
    let cb = unit_bag(
        Lang::Java,
        &featurize(&c, &FeatureConfig::DEFAULT, Some(&kept)),
    );
    assert_eq!(e.jaccard, jaccard(&qb, &cb));
}

#[test]
fn explain_identity_and_empty_masks() {
    let q = two("a", "b");
    let same = explain(Lang::Java, &FeatureConfig::DEFAULT, &q, &q, &[true, true]);
    assert_eq!(same.matched.len(), 5);
    assert!(same.query_only.is_empty() && same.candidate_only.is_empty());
    assert_eq!(same.jaccard, 1.0);
    // A short mask hides the missing leaves; nothing kept means nothing matched.
    let none = explain(Lang::Java, &FeatureConfig::DEFAULT, &q, &q, &[]);
    assert!(none.matched.is_empty() && none.candidate_only.is_empty());
    assert_eq!(none.query_only.len(), 5);
    assert_eq!(none.jaccard, 0.0);
}
