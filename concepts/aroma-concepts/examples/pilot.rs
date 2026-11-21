//! M-1 pilot (plan D16): does Aroma produce useful clustered recommendations on a single
//! Salesforce repository?
//!
//! ```text
//! cargo run --release --example pilot -- <repo> [n_queries=30] [seed=119] [samples.md]
//! ```
//!
//! Protocol (mirrors paper section 5 / D17 held-out rules):
//!   1. index every Apex method/constructor/trigger/accessor body in the repo;
//!   2. sample non-test units with >= 12 lines and >= 3 body statements (seeded);
//!   3. query = the body's first statements spanning ~5 lines, pasted into a wrapper method
//!      (no declarations of parameters/fields: a realistic partial snippet);
//!   4. the source unit and its exact feature clones are excluded from the index;
//!   5. record: similar candidates (baseline), whether >= 1 cluster forms, members,
//!      and how much code the top recommendation adds beyond the query.

use aroma_concepts::features::FeatureConfig;
use aroma_concepts::pipeline::{Corpus, Params};
use aroma_concepts::render::render_full_lines;
use aroma_concepts::spt::Spt;
use aroma_concepts::ts_adapter::{to_raw, Apex, LanguageRules};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tree_sitter::{Node, Parser, Tree};

struct File {
    path: PathBuf,
    src: String,
}

struct UnitMeta {
    file: usize,
    lines: Range<u32>, // 1-based, end exclusive
    stmts: Vec<(Range<usize>, u32)>, // byte range + start line of each body statement
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if name != ".git" && name != "node_modules" {
                walk(&p, out);
            }
        } else if name.ends_with(".cls") || name.ends_with(".trigger") {
            out.push(p);
        }
    }
}

fn apex_parser() -> Parser {
    let mut p = Parser::new();
    p.set_language(&tree_sitter_sfapex::apex::LANGUAGE.into()).expect("apex grammar ABI");
    p
}

fn unit_nodes<'t>(tree: &'t Tree, rules: &Apex) -> Vec<Node<'t>> {
    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        if rules.is_unit(n) {
            out.push(n);
        }
        let mut c = n.walk();
        stack.extend(n.children(&mut c));
    }
    out.sort_by_key(|n| n.start_byte());
    out
}

fn build(body: Node<'_>, src: &[u8], rules: &Apex) -> Option<Spt> {
    to_raw(body, src, rules).and_then(Spt::build)
}

/// Deterministic xorshift for sampling.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn pct(v: &mut [f64], q: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[((v.len() - 1) as f64 * q).round() as usize]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let repo = PathBuf::from(args.get(1).expect("usage: pilot <repo> [n] [seed] [samples.md]"));
    let n_queries: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let seed: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(119);
    let samples_path = args.get(4).cloned();

    // ---- load + parse ----
    let t0 = Instant::now();
    let mut paths = Vec::new();
    walk(&repo, &mut paths);
    paths.sort();
    let mut parser = apex_parser();
    let mut files = Vec::new();
    let mut trees = Vec::new();
    let mut types = Vec::new();
    for p in paths {
        let Ok(src) = std::fs::read_to_string(&p) else { continue };
        let Some(tree) = parser.parse(&src, None) else { continue };
        types.extend(Apex::declared_types(&tree, src.as_bytes()));
        files.push(File { path: p, src });
        trees.push(tree);
    }
    let rules = Apex::new(types);
    let cfg = FeatureConfig { uppercase_is_global: false, ..FeatureConfig::REFERENCE };

    // ---- index ----
    let mut corpus = Corpus::default();
    let mut metas = Vec::new();
    for (fi, (f, tree)) in files.iter().zip(&trees).enumerate() {
        let src = f.src.as_bytes();
        for u in unit_nodes(tree, &rules) {
            let body = u.child_by_field_name("body").expect("is_unit checked body");
            let Some(spt) = build(body, src, &rules) else { continue };
            let mut stmts = Vec::new();
            if body.kind() == "block" {
                let mut c = body.walk();
                for s in body.named_children(&mut c) {
                    if !s.kind().ends_with("comment") {
                        stmts.push((s.byte_range(), s.start_position().row as u32 + 1));
                    }
                }
            }
            corpus.add(spt, &cfg);
            metas.push(UnitMeta {
                file: fi,
                lines: (u.start_position().row as u32 + 1)..(u.end_position().row as u32 + 2),
                stmts,
            });
        }
    }
    let index_secs = t0.elapsed().as_secs_f64();

    // exact feature clones (held out together with the source unit)
    let mut by_bag: HashMap<Vec<(u32, u32)>, Vec<u32>> = HashMap::new();
    for (i, u) in corpus.units.iter().enumerate() {
        by_bag.entry(u.bag.iter().collect()).or_default().push(i as u32);
    }

    // ---- sample queries ----
    let mut eligible: Vec<u32> = (0..metas.len() as u32)
        .filter(|&i| {
            let m = &metas[i as usize];
            let path = files[m.file].path.to_string_lossy().to_ascii_lowercase();
            !path.contains("test") && m.lines.end - m.lines.start >= 12 && m.stmts.len() >= 3
        })
        .collect();
    let mut rng = Rng(seed.max(1));
    for i in (1..eligible.len()).rev() {
        let j = (rng.next() % (i as u64 + 1)) as usize;
        eligible.swap(i, j);
    }
    eligible.truncate(n_queries);

    // Threshold sweep overrides (defaults = reference values).
    let env = |k: &str, d: f64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let gated = std::env::var("AROMA_GATED").is_ok();
    let test_units: HashSet<u32> = (0..metas.len() as u32)
        .filter(|&i| files[metas[i as usize].file].path.to_string_lossy().to_ascii_lowercase().contains("test"))
        .collect();
    let p = Params {
        min_similarity: env("AROMA_MIN_SIM", Params::REFERENCE.min_similarity),
        min_pruned: env("AROMA_MIN_PRUNED", Params::REFERENCE.min_pruned),
        tau3: env("AROMA_TAU3", Params::REFERENCE.tau3),
        tau2: env("AROMA_TAU2", Params::REFERENCE.tau2),
        ..Params::REFERENCE
    };
    eprintln!("params: {p:?}");
    let mut qparser = apex_parser();
    let (mut n_similar, mut n_clustered, mut n_extends) = (0usize, 0usize, 0usize);
    let (mut times, mut members, mut top1, mut ext_leaves) = (vec![], vec![], vec![], vec![]);
    let mut samples = String::new();
    let mut shown = 0;

    for &src_unit in &eligible {
        let m = &metas[src_unit as usize];
        let f = &files[m.file];
        let first_line = m.stmts[0].1;
        let mut k = m.stmts.iter().filter(|s| s.1 <= first_line + 4).count();
        k = k.max(2).min(m.stmts.len() - 1);
        let snippet = &f.src[m.stmts[0].0.start..m.stmts[k - 1].0.end];
        let wrapped = format!("public class PilotQ {{\n public void pilotQ() {{\n{snippet}\n }}\n}}\n");
        let Some(qtree) = qparser.parse(&wrapped, None) else { continue };
        let Some(qbody) = unit_nodes(&qtree, &rules).first().and_then(|u| u.child_by_field_name("body")) else { continue };
        let Some(qspt) = build(qbody, wrapped.as_bytes(), &rules) else { continue };

        let mut exclude: HashSet<u32> = HashSet::from([src_unit]);
        exclude.extend(by_bag[&corpus.units[src_unit as usize].bag.iter().collect::<Vec<_>>()].iter().copied());
        if gated {
            exclude.extend(test_units.iter().copied()); // D26: production query -> production members
        }

        let t = Instant::now();
        let out = corpus.query(&qspt, &cfg, &p, &exclude);
        times.push(t.elapsed().as_secs_f64() * 1000.0);

        if !out.similar.is_empty() {
            n_similar += 1;
            top1.push(out.similar[0].1);
        }
        let qleaves = qspt.leaves.len() as i64;
        let accept = |r: &&aroma_concepts::pipeline::Recommendation| {
            if !gated {
                return true;
            }
            // D26 confidence gate: >= 3 independent (non-clone) members, bounded extension.
            let independent: HashSet<Vec<(u32, u32)>> =
                r.members.iter().map(|&u| corpus.units[u as usize].bag.iter().collect()).collect();
            let added = r.kept.iter().filter(|&&b| b).count() as i64 - qleaves;
            independent.len() >= 3 && added > 0 && added <= 2 * qleaves
        };
        let rec_text = out.recommendations.iter().find(accept).map(|r| {
            let first = r.members[0] as usize;
            let kept = r.kept.iter().filter(|&&b| b).count() as i64;
            let added = kept - qspt.leaves.len() as i64;
            ext_leaves.push(added as f64);
            members.push(r.members.len() as f64);
            if added > 0 {
                n_extends += 1;
            }
            let fm = &metas[first];
            (render_full_lines(&files[fm.file].src, fm.lines.clone(), &corpus.units[first].spt, &r.kept), r.members.len(), added)
        });
        if rec_text.is_some() {
            n_clustered += 1;
        }

        if samples_path.is_some() && shown < 10 && (rec_text.is_some() || shown < 2) {
            shown += 1;
            let rel = |fi: usize| files[fi].path.strip_prefix(&repo).unwrap_or(&files[fi].path).display().to_string();
            let _ = writeln!(samples, "### Query {shown}: from `{}`:{}\n\n```apex\n{}\n```\n", rel(m.file), m.lines.start, snippet.trim());
            match &rec_text {
                Some((text, n, added)) => {
                    let _ = writeln!(samples, "**Aroma recommendation** ({n} cluster members, {added:+} leaves vs query):\n\n```apex\n{}```\n", text);
                }
                None => {
                    let _ = writeln!(samples, "**Aroma recommendation:** none (no valid cluster)\n");
                }
            }
            if let Some(&(u, score)) = out.similar.first() {
                let um = &metas[u as usize];
                let lines: Vec<&str> = files[um.file].src.lines().skip(um.lines.start as usize - 1).take(((um.lines.end - um.lines.start) as usize).min(25)).collect();
                let _ = writeln!(samples, "**Baseline top-1 similar** (`{}`:{}, Jaccard {score:.2}):\n\n```apex\n{}\n```\n", rel(um.file), um.lines.start, lines.join("\n"));
            }
        }
    }

    let nq = eligible.len().max(1) as f64;
    println!("repo={} files={} units={} index_s={:.2}", repo.display(), files.len(), metas.len(), index_secs);
    println!(
        "queries={} similar_found={:.0}% clustered={:.0}% rec_adds_code={:.0}% median_members={:.1} median_added_leaves={:.1} median_top1_jaccard={:.2}",
        eligible.len(),
        100.0 * n_similar as f64 / nq,
        100.0 * n_clustered as f64 / nq,
        100.0 * n_extends as f64 / nq,
        pct(&mut members, 0.5),
        pct(&mut ext_leaves, 0.5),
        pct(&mut top1, 0.5),
    );
    println!("query_ms p50={:.1} p95={:.1} max={:.1}", pct(&mut times.clone(), 0.5), pct(&mut times.clone(), 0.95), pct(&mut times, 1.0));
    if let Some(path) = samples_path {
        std::fs::write(&path, samples).expect("write samples");
        println!("samples -> {path}");
    }
}
