# TODOS

## Salesforce metadata (v1.2)

### Completed: XML metadata parser and order-insensitive features

Shipped structural metadata search after the T28 spike and T29 implementation
(D32, D34, D39, D42). Parser: `tree-sitter-xml` 0.7. Structural retrieval beat the
BM25-only baseline; see [the release record](docs/results/26-sfmeta-structural.md).

## LWC (v1.1 follow-up)

### LWC JS Recall@1 ties between near-duplicate members

**What:** v1.1 shipped with LWC JS start-slice R@1 at 0.9489 / 0.9477 (gate 0.95, [release record](docs/results/27-v1.1-release-gate.md)). Most misses are tied maxima between near-identical handlers whose distinguishing evidence is in the unqueried suffix (internal query audit, not published).

**Why:** A first-rank miss among clones is low-harm (the source is in the top 100 at 0.999), but it is the one v1.1 gate not met.

**Context:** Only evidence-based ideas qualify: e.g. treating exact-body clones across components as one result group, or using decorator/header evidence present in the query. No ranking rule may consult benchmark sources or slice positions. Validate on seeds 119 and 4242 plus the v1.0 Java/Apex suites (Java R@1 has zero margin).

**Effort:** M
**Priority:** P2
**Depends on:** nothing

## Harness

### Non-destructive Java fixture selection

**What:** `xtask` `java_corpus(n)` deletes every Java repository beyond the first `n` from the shared corpus directory `target/corpus/java`. A `recall` run (default 20) or a small perf smoke silently shrinks the 200k-unit perf fixture.

**Why:** The v1.2/v1.3 perf run first measured 124k / 105k units instead of 233k ([release record](docs/results/29-v1.2-v1.3-release-gates.md)) and needed re-clones.

**Context:** Keep all fetched repositories; index a selection (e.g. a per-`n` directory of symlinks, or an ignore list) instead of deleting. Also make `--java-repos` smaller than what is on disk a no-op for other suites.

**Effort:** S
**Priority:** P2
**Depends on:** nothing

## Recommendations (experimental, D27)

### Experiment: IDF-weighted features in cluster tests

**What:** Weight features by inverse document frequency (within the repo) in the cs/csq cluster tests and Jaccard, so generic structure (`new List<X>()`, early `return`, `for` loops) counts less than API- and domain-specific features; re-run `concepts/aroma-concepts/examples/pilot.rs` with `AROMA_GATED=1`.

**Why:** Pilot round 2 showed recommendations win only ≈ 28% vs the top-5-similar baseline because generic shapes pass the cluster tests while members are unrelated ([pilot record](docs/results/17-pilot-results.md)).

**Context:** Promotion bar for `--experimental-recommend` → default: ≥ 60% wins on judged samples and on the D17 idiom suite. Pilot sample dumps are internal research material, not published. Deviates from both paper and reference; ship as a separate profile knob.

**Effort:** S
**Priority:** P2
**Depends on:** None (concept crate + pilot harness exist)

### Experiment: multi-repo density for clustering

**What:** Index the target repo together with a curated set of public Salesforce repos (apex-recipes, NPSP, NebulaLogger, fflib, …) and measure whether cluster formation and win-rate cross the bar.

**Why:** Cluster formation grew with repo size in the pilot (7% at 732 units → 19% at 12.9k); the paper used 2.4M methods.

**Context:** Conflicts with the single-repo scope (U3); recommendations would come from other codebases' conventions, so results must show provenance. Use the pilot harness with a multi-root walk.

**Effort:** S
**Priority:** P3
**Depends on:** IDF experiment result

## Rendering

### Named-hole (anti-unification) recommendation renderer

**What:** Alternative to line-granular `// your code ...` output: align cluster members and render the shared shape with named holes `⟨h1⟩` where members differ, following ctxloom/reprise `src/au.rs` (BSD-3).

**Why:** Line-granular output shows unselected tokens and anonymous gaps; templates with holes are clearer for humans and directly fillable by agents.

**Context:** Needleman-Wunsch child alignment and lgg-style consistent holes are possible approaches. Output would no longer match the reference renderer, so ship it as `--render holes` alongside the reference `--render lines`. Plan §8.1 and §11.

**Effort:** M
**Priority:** P3
**Depends on:** v1.0 render shipped; M-1 pilot shows clusters are useful

## Release follow-ups (from the [v1.0 gate run](docs/results/19-v1-release-gates.md))

### Run CI on a real remote

**What:** Push to a GitHub remote and get `.github/workflows/ci.yml` green on ubuntu and macOS; tag a pre-release to exercise `release.yml`.

**Why:** CI has never run (no git remote); macOS and the release matrix are untested.

**Effort:** S
**Priority:** P1
**Depends on:** Release-owner remote setup and authorization. The project license is now Apache-2.0; licensing is no longer a blocker.

### Index-build memory at 200k+ units

**What:** Reduce `aroma index` peak RSS (2.48 GiB at 233k Java units; only query RSS is gated).

**Why:** Large repos on small CI runners or laptops may run out of memory while indexing.

**Context:** Likely streaming postings/lexical CSR construction instead of holding all bags; measure with `xtask perf`.

**Effort:** M
**Priority:** P2
**Depends on:** None

### Query latency headroom

**What:** p50 is 89 ms against the 100 ms gate at 233k units on 4 cores. Candidates: lazily load `files.json` (3 MB JSON parse per one-shot query), skip SPT decode for lexical hits that do not survive fusion, cache unit bags in the index instead of re-featurizing candidates.

**Why:** Thin margin; a busier machine can fail the gate.

**Effort:** M
**Priority:** P2
**Depends on:** None

## Completed

### BM25 identifier channel fused with structural search (shipped in v1.0)

A lexical BM25 index over identifiers (split terms plus full identifiers) supplies
a second candidate source, fused with structural search by reciprocal rank fusion
`Σ 1/(60 + rank)` and structurally reranked (D30). `--explain` reports channel
ranks; `--no-lexical` disables it and `--compat` keeps structural-only behaviour.
See [the v1.0 release record](docs/results/19-v1-release-gates.md).

### v1.0 gate: Java strict Recall@1 (resolved 2026-11-20)

The exact-prefix display rule lifts strict R@1 to 0.950 / 0.954 (fresh seed);
[record 19](docs/results/19-v1-release-gates.md) preserves the thin-margin caveat.


### Make `parse_timeout_halts_the_parser` load-independent

**What:** `crates/aroma-lang/tests/table.rs::parse_timeout_halts_the_parser` asserts a 20,000-statement file parses within the default 2 s wall-clock budget. Under heavy load (load avg 23–39 on 4 CPUs during Wave 1) it took 8–13 s wall time for 0.88 s CPU and failed.

**Why:** Flaky on loaded machines and slow CI runners; a red gate that isn't a real regression.

**Context:** Found by the T06 implementer (2026-11-20). Options: assert on a much larger synthetic input with a tiny explicit `parse_timeout_micros` (tests the halt, not the speed), or measure thread CPU time. Do not loosen the D11 2 s production default.

**Effort:** S
**Priority:** P1
**Depends on:** None

**Completed:** 2026-11-20 — the success case now uses an explicit 120 s budget; the 1 µs timeout case and the pinned 2 s default (`limits_default_values`) are unchanged.
