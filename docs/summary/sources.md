# Sources for the project summary

Each statement in the [project summary](summary.html) cites one or more of the IDs (E##) below,
and each ID gives its source in this repository. Numbers are copied exactly from the cited
records. Public release records are in [`docs/results/`](../results/README.md); internal notes
are named but not published. All measurements are from one 4-CPU / 11 GB Linux machine,
Rust 1.90, release builds, on 2026-11-19..20.

## What was built

- E01. aroma-rs re-implements Aroma (Luan et al., "Aroma: Code Recommendation via Structural Code
  Search", OOPSLA 2019, arXiv:1812.01158) in Rust on tree-sitter: a library + CLI that indexes a
  single repository and, given a partial code snippet, returns structurally similar code with the
  matching region highlighted (Aroma Phase I retrieval + pruning/reranking). [docs/PLAN-aroma-rs.md §0]
- E02. Languages: Java, Salesforce Apex (with inline SOQL/SOSL), Lightning Web Components JavaScript
  (`lwcjs`) and HTML templates (`lwchtml`), Salesforce XML metadata (`sfmeta`: flows, fields,
  validation rules, custom metadata records), Aura component markup (`aura`), Aura controller/helper
  JavaScript (`aurajs`), Visualforce pages (`vf`) — 8 language adapters. Sources: `crates/aroma-core/src/lang.rs`; [docs/results/27-v1.1-release-gate.md](../results/27-v1.1-release-gate.md), [docs/results/28-v1.3-recall.md](../results/28-v1.3-recall.md), [docs/results/29-v1.2-v1.3-release-gates.md](../results/29-v1.2-v1.3-release-gates.md).
- E03. Every language is a declarative tree-sitter "LangSpec" table plus a normalizer feeding one
  language-agnostic core (simplified parse tree → Aroma features). Pinned grammars: tree-sitter
  0.25.10, java 0.23.5, sfapex 3.0.2, javascript 0.25.0, html 0.23.2, xml 0.7.0. Visualforce uses
  the HTML grammar (368/371 pages parse clean vs 291/371 with XML); Aura markup uses XML (464/464).
  Sources: `Cargo.toml`; internal note 20 (not published); plan D32/D46.
- E04. Differential testing: in reference-compatibility mode the Rust pipeline reproduces the
  Aroma authors' reference implementation (similar.py) outputs on 520/520 golden cases at every stage.
  Source: [docs/results/19-v1-release-gates.md](../results/19-v1-release-gates.md).
- E05. Workspace: 3 crates + xtask harness; ~30.5k lines of Rust in crates; 734 automated tests
  passing at the end (fmt, clippy -D warnings, cargo test). [git; final gate runs]
- E06. Additions beyond the Aroma paper: a BM25 identifier channel fused with structural retrieval by
  reciprocal-rank fusion (RRF, Σ 1/(60+rank)); snippet repair for partial code (bracket closer for
  code; tag-stack closer for markup); a whole-component query that searches each part (e.g. LWC
  .js + .html, Aura markup + controller + helper) and sums RRF per folder (D45), crediting
  exact-body clones in every folder that holds them (D50). Sources: internal note 18 (not published); [docs/results/19-v1-release-gates.md](../results/19-v1-release-gates.md); plan D36, D45, D50.

## Pilot: Aroma's clustered recommendations on single repositories (M-1, note 17)

- E07. On three single Apex repositories (apex-recipes 732 units, NebulaLogger 2,826, NPSP 12,878),
  with the Aroma paper's reference thresholds, recommendation clusters formed for only 7%, 17% and 19%
  of 100 queries; relaxed thresholds reached 27%/31%/52% with mixed quality. Source: [docs/results/17-pilot-results.md](../results/17-pilot-results.md).
- E08. Decision (D26/D27): v1.0 ships similar-code search; clustered recommendations stay
  experimental until they beat the top-5-similar baseline in ≥ 60% of shown cases. On the 40-task
  idiom suite they recorded 0 wins / 4 ties / 1 loss. Sources: [docs/results/17-pilot-results.md](../results/17-pilot-results.md), [docs/results/19-v1-release-gates.md](../results/19-v1-release-gates.md).

## Evaluation protocol

- E09. Self-retrieval recall: sample an indexed unit, cut a 5-line query from it (start, middle or
  end slice per D43), keep the source indexed, and record the displayed rank of the source (or an
  exact-body clone). R@1 and R@100. A scorer bug that compared scores instead of displayed position
  overstated v1.0 Java R@1 as 0.991; corrected to 0.869 (D40). Source: [docs/results/19-v1-release-gates.md](../results/19-v1-release-gates.md) (correction).
- E10. Idiom suites: hand-written tasks; the source unit and its exact-body clones are held out;
  a hit = top-5 similar result whose full source contains every required fragment. Task validity
  is checked post-holdout (≥ 2 remaining answers; no required fragment may appear in the query)
  (D48). Sources: internal note 25 (not published); [docs/results/26-sfmeta-structural.md](../results/26-sfmeta-structural.md).
- E11. A held-out seed (4242) is used to check every ranking change chosen on the default seed (119).
  Sources: internal notes 22, 23 (not published).

## v1.0 — Java + Apex (notes 19, 22)

- E12. Ties dominated Java R@1 misses (119/131 misses were ties). An "exact-prefix" display rule
  (candidates whose ordered tokens match the query's opening sequence, ≥ 3 leaves, shown first)
  raised Java R@1 from 0.869 to 0.950 (default seed) and 0.954 (held-out seed), R@100 0.999;
  Apex pooled R@1 0.971, R@100 0.997; idiom 25/40 = 62.5% (gate 60%). Caveat: zero Java headroom;
  chosen on the release corpus. Sources: [docs/results/19-v1-release-gates.md](../results/19-v1-release-gates.md); internal note 22 (not published).
- E13. Java fixture 80,024 units (20 repos) for recall; 1,000 queries; Apex 1,347 queries over
  3 repos. Source: internal note 22 (not published).

## v1.1 — Lightning Web Components (notes 23, 25, 27)

- E14. LWC JS pooled start R@1 = 0.9489 (seed 119) / 0.9477 (seed 4242) vs gate 0.95 — SHORT;
  R@100 = 0.9990; LWC HTML start R@100 = 0.9973; n = 6,141 JS and 4,059 HTML queries, 45 repos.
  v1.1 shipped with the shortfall recorded (owner decision). Source: [docs/results/27-v1.1-release-gate.md](../results/27-v1.1-release-gate.md).
- E15. Diagnosis of 288 JS method misses: 194 tied maxima between near-duplicate methods,
  71 unique maxima displayed lower, 19 lower scores, 4 absent; decorators involved in 158.
  Source: internal note 23 (not published).
- E16. A fix reading a pasted class member as a member raised the member's self-score from 0.6875
  to 1.0 but lowered benchmark R@1 (0.9498 → 0.9489; 9 wins / 15 losses) — kept as a product fix.
  Source: internal note 23 (not published).
- E17. LWC idiom suite: 20 tasks (8 JS / 6 HTML / 6 whole-component); after post-holdout
  validation repaired 3 unanswerable tasks: 13/20 = 65.0%; after the clone-credit join fix (D50):
  14/20 = 70.0% (gate 60%). An implementer's task "repair" had leaked answers into 13 queries;
  the leak check (D48) now rejects that. Sources: internal note 25 (not published); [docs/results/27-v1.1-release-gate.md](../results/27-v1.1-release-gate.md).
- E18. LWC HTML template R@1 = 0.579 (reported, not gated). Source: [docs/results/27-v1.1-release-gate.md](../results/27-v1.1-release-gate.md).

## v1.2 — Salesforce metadata (notes 24, 26)

- E19. Evidence-first: a BM25-only (lexical) path and a structural prototype (order-insensitive
  children, positionless variable features) were both measured on the same pooled index of
  50 repositories (16,962 units, 8,155 files) and a 15-task idiom suite. Structural 7/15 = 46.7%
  vs lexical 3/15 = 20.0% (+26.7 points; the pre-declared bar was +5). Structural became the
  default. Sources: [docs/results/24-metadata-verdict.md](../results/24-metadata-verdict.md), [docs/results/26-sfmeta-structural.md](../results/26-sfmeta-structural.md).
- E20. Metadata recall R@100 (pooled start): flow_element 1.0000, validation_rule 1.0000,
  field 1.0000 (gate 0.95). Corpus: 6,835 flow elements from 869 flows, 150 validation rules,
  519 formula fields. Sources: [docs/results/24-metadata-verdict.md](../results/24-metadata-verdict.md), [docs/results/26-sfmeta-structural.md](../results/26-sfmeta-structural.md), [docs/results/29-v1.2-v1.3-release-gates.md](../results/29-v1.2-v1.3-release-gates.md).

## v1.3 — Aura + Visualforce (notes 28, 29)

- E21. R@100 pooled start: Aura markup 0.9823 (n=451), Aura JS 1.0000 (n=1,282), Visualforce
  0.9887 (n=529) (gate 0.98). R@1 reported: 0.8470 / 0.9774 / 0.5784. Floors: 402 Aura bundles,
  444 Visualforce pages (gate 300 each; reached only after adding 8 licensed repositories — the
  first scout had counted `.component` files as pages). Sources: [docs/results/28-v1.3-recall.md](../results/28-v1.3-recall.md), [docs/results/29-v1.2-v1.3-release-gates.md](../results/29-v1.2-v1.3-release-gates.md).
- E22. Component join on a real Aura bundle: crediting exact-body clones moved the queried bundle
  from rank 15 to rank 2. Source: [docs/results/29-v1.2-v1.3-release-gates.md](../results/29-v1.2-v1.3-release-gates.md).

## Performance (note 29; v1.0 in note 19)

- E23. One-shot CLI query on 233,318 Java units, 100 queries: p50 90.4 ms, p95 194.1 ms
  (v1.0: 89.3 / 150.2); query peak RSS 260 MiB; NPSP full index with all 7 non-Java languages
  enabled (17,739 units) 8.38 s; whole-component query p50 28.8 ms. Gates: p50 ≤ 100 ms,
  p95 ≤ 500 ms, RSS ≤ 1 GiB, index ≤ 60 s, component p50 ≤ 200 ms. Index-build peak RSS
  2,486 MiB (reported, not gated). Sources: [docs/results/19-v1-release-gates.md](../results/19-v1-release-gates.md), [docs/results/29-v1.2-v1.3-release-gates.md](../results/29-v1.2-v1.3-release-gates.md).

## Process (how the work was done)

- E24. Built over two days by one orchestrating agent (Claude) directing implementer agents
  (OpenAI Codex gpt-6.1-sol, Pi with DeepSeek-V4-Flash, Antigravity with Gemini 3.8) in isolated
  git worktrees; the orchestrator reviewed diffs, re-ran gates and merged. Release gates were
  fixed before measurement; shortfalls were recorded rather than tuned away. Source: internal process record (not published).

## Known limitations

- E25. Single-repository scale (< 200k units in the plan; 233k measured). One machine; no CI run;
  no external users or user study; idiom suites are small (40, 20, 15 tasks) and hand-written by
  the developers; self-retrieval recall measures findability of an existing unit, not usefulness.
  Some ranking choices were selected on the evaluation corpus (held-out seed only partly
  mitigates). Sources: [docs/results/19-v1-release-gates.md](../results/19-v1-release-gates.md), internal note 22 (not published), [docs/results/27-v1.1-release-gate.md](../results/27-v1.1-release-gate.md), [docs/results/29-v1.2-v1.3-release-gates.md](../results/29-v1.2-v1.3-release-gates.md).
