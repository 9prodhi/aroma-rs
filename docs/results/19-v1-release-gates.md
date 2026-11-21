# 19 — v1.0 release gates, final run on main 39c1b6a (2026-11-20)

Machine: 4 CPUs, 11 GB RAM, Linux, Rust 1.90.0, release builds. Commands from the Definition of
done (internal research note, not published).

| Gate | Target | Result | |
|---|---|---|---|
| `xtask recall` Java contiguous (first 20 repos, 80,024 units, n=1000) | R@1 ≥ 0.95, R@100 ≥ 0.99 | R@1 **0.869** (was misreported as 0.991, see correction below), R@100 **0.999** | **FAIL (R@1)** |
| `xtask recall` Apex pooled contiguous (3 pilot repos, n=1347) | R@100 ≥ 0.98 | R@1 0.939 (was misreported as 0.993), R@100 **0.997** | PASS |
| `xtask idiom` similar hit@5 (40 repaired tasks, D29) | ≥ 60% | **25/40 = 62.5%** | PASS (1-task margin) |
| `xtask perf` NPSP full index | ≤ 60 s | **6.29 s** | PASS |
| `xtask perf` one-shot query, 233,318 Java units, 100 queries | p50 ≤ 100 ms, p95 ≤ 500 ms | p50 **89.3 ms**, p95 **150.2 ms** (max 526.7) | PASS |
| `xtask perf` query peak RSS (fresh process) | ≤ 1 GiB | **258 MiB** | PASS |
| fmt / clippy `-D warnings` / `cargo test --workspace` | green | 28 suites, 451 tests | PASS |
| T16 differential vs similar.py goldens (`--compat`) | exact | 520/520 cases, every stage | PASS |
| CI (ubuntu + macOS) | green | **not run** — repo has no git remote | OPEN |

Reported, not gated: non-contiguous recall (Java R@100 0.957, Apex 0.94–0.96); index-build peak
RSS 2.48 GiB at 233k units; recommendations 0 wins / 4 ties / 1 loss on the idiom suite (stays
experimental, D27).

## What changed after the 21 planned tasks to get here

- D30 (user): idiom-suite repair (T21), no similar score floor (T22), BM25 identifier channel with
  RRF fusion (T23). See note 18 (internal research note, not published).
- Snippet robustness (found via recall misses, all TDD): auto-close unmatched brackets; drop up to
  3 trailing lines when a cut-off snippet does not parse (damage still counts them); close an
  unclosed `try` as `} finally {}`.
- Harness fixes: recall samples lines of code, not comment-only lines; perf measures query RSS in
  a fresh probe process (it had counted the in-process index build's heap).
- Latency at 233k units (p50 109.9 → 89.3 ms): `Globals` as a sorted `Vec` (same JSON); O(1)
  `MmapPostings::open` with array checks moved to `verify` (now run by `aroma inspect-index`,
  which reports integrity); load + rerank on up to 8 scoped threads, bit-identical to sequential.

## Caveats

- p50 margin is ~11 ms on 4 cores; the plan's perf target assumed 8 cores. A slower or busier
  machine may exceed 100 ms.
- The idiom suite has 40 tasks; 62.5% passes by one task.
- Index format is schema 2 (T23): indexes built before T23 must be rebuilt.

## Correction 2026-11-20 (decision D40)

The R@1 values above were overstated. `xtask/src/recall.rs` `source_rank` counted a source as
rank 1 whenever its structural score was >= the score of the first *displayed* result, but since T23
the list is displayed in RRF order (structural rank fused with BM25 rank), so a source shown at
position 3 could count as rank 1. Fixed in commit dc38eff (rank = displayed position); re-run on
main 1a72d1a with the same corpus and seed:

| Check | Reported | Corrected | Gate |
|---|---|---|---|
| Java contiguous R@1 | 0.991 | **0.869** | ≥ 0.95 **FAIL** |
| Java contiguous R@100 | 0.999 | 0.999 | PASS |
| Apex pooled contiguous R@1 | 0.993 | 0.939 | not gated |
| Apex pooled contiguous R@100 | 0.997 | 0.997 | PASS |

Diagnosis (1,000 Java queries): 869 hits; 119 misses where the source shares the top structural
score with another unit (median 2 tied units; the query cannot distinguish them after pruning),
11 misses with a lower structural score, 1 absent. Ordering by structural score first (BM25 only as
tie-break) gives strict R@1 0.901 but drops the idiom suite to 57.5% (< 60%), so it was rejected.
Tie-aware R@1 (source has the best structural score) is 0.974. **User decision: keep the strict
gate; v1.0's Java R@1 gate is open; a research lane investigates tie-breaking.** All other v1.0 gates
are unaffected.

## Resolution 2026-11-20 (research note 22 (internal research note, not published), user-approved merge)

A display-order rule now shows candidates whose body starts with the query's exact token sequence
(≥ 3 leaves) first; such candidates contain the query verbatim at their start and normally already
hold the top structural score, so the rule settles the near-ties that BM25's short-unit bias used to
decide. Everything else keeps RRF order; `--compat` / `--no-lexical` are unchanged.

| Check | Before | After (tuning seed) | After (fresh seed 4242) | Gate |
|---|---|---|---|---|
| Java contiguous R@1 | 0.869 | **0.950** | **0.954** | ≥ 0.95 PASS (margin < 0.005) |
| Java non-contiguous R@1 | 0.775 | — | 0.786 (main 0.782) | reported |
| Java R@100 | 0.999 | 0.999 | 1.000 | PASS |
| Apex pooled R@1 / R@100 | 0.939 / 0.997 | 0.971 / 0.997 | — | PASS |
| Idiom similar hit@5 | 62.5% | 62.5% | — | PASS |

Caveat recorded with the user's decision: the gate's queries are always a method's first lines,
which is the situation this rule rewards; the margin is thin. D43's middle/end slices give a
harder view for the new languages.
