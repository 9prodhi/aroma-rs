# 17 — M-1 pilot results (plan D16), 2026-11-19

Harness: `concepts/aroma-concepts/examples/pilot.rs` (reference-profile pipeline, tree-sitter-sfapex 3.0.2,
D18 context-free `#VAR` rule with repo-declared types + Apex namespaces + common sObjects as globals,
Apex case-folding, `uppercase_is_global = false`). Queries: non-test units with ≥ 12 lines and ≥ 3 body
statements, seed 119; query = first body statements spanning ~5 lines pasted into a wrapper method;
source unit and exact feature clones held out. Samples are internal research material, not published.

## Corpora (single repos, shallow clones)

| Repo | Commit | Apex files | Units indexed | Index time (release) |
|---|---|---|---|---|
| trailheadapps/apex-recipes | eda6167 | 142 | 732 | 0.26 s |
| jongpie/NebulaLogger | b792ac8 | 245 | 2,826 | 1.70 s |
| SalesforceFoundation/NPSP | f5072b4 | 1,070 | 12,878 | 6.03 s |

## Cluster formation (100 queries; apex-recipes has 84 eligible)

| Thresholds (min_pruned / τ2 / τ3 / Phase I ratio) | apex-recipes | NebulaLogger | NPSP |
|---|---|---|---|
| reference 0.65 / 1.5 / 0.9 / 0.4 — similar found | 33% | 39% | 43% |
| reference — **clustered** | **7%** | **17%** | **19%** |
| relaxed 0.5 / 1.2 / 0.8 / 0.4 — similar / clustered | 60% / 27% | 60% / 31% | 72% / **52%** |
| loose 0.4 / 1.1 / 0.7 / 0.3 — similar / clustered | 80% / 50% | 77% / 55% | 84% / 75% |

Median cluster members: 2–3 (relaxed), up to 9 on NPSP at reference thresholds (30-query run).
Query latency (concept code, unoptimised, single-threaded): p50 1–32 ms, p95 10–170 ms (NPSP).

## Usefulness (my judgment on 10 relaxed-threshold samples; please verify in the sample files)

| Verdict | Count | Examples |
|---|---|---|
| Better than top-1 similar (focused idiomatic extension) | 2–3 | NPSP Q1: savepoint → `try/catch` + `Database.rollback(sp)` + `ERR_Handler`; NebulaLogger Q3: `String.format` + `Database.query(String.escapeSingleQuotes(...))` |
| Same as baseline | ~3 | near-identical sibling method already shown by top-1 similar |
| Worse / noisy | ~4 | 2-member clusters carve nearly a whole method (+95..+136 leaves); production query gets a test-method carve; unrelated detail retained |

## Go / no-go against the D16 rule (clusters ≥ 40% of queries AND beat the baseline)

**NO-GO for the plan as written** (reference profile, single repo): clusters form for only 7–19% of
queries. Relaxing thresholds reaches 52% on the large repo, but quality is mixed: Aroma beats the
similar-code baseline in roughly a quarter of the sampled cases.

## What the data suggests

1. **Phase I + prune/rerank works on single repos** (similar code found for 33–84% of held-out
   partial queries depending on threshold); this is the dependable part.
2. **Clustering needs density.** It scales with repo size (7% → 19% at reference thresholds from 732
   to 12.9k units) and with lower thresholds.
3. **Observed failure modes are filterable:** (a) 2-member clusters ≈ copy of one method → require ≥ 3
   independent members; (b) runaway carves → cap extension at ~2× query leaves; (c) test/production
   mixing → match test-ness of query and members; (d) generated/near-duplicate siblings → D22 clone
   dedup.

## Caveats

- 10 judged samples is small; judgments are mine, not a blind human rating.
- Concept-crate fidelity: no Java-style virtual-ANTLR regrouping for Apex, Apex rules approximate
  (globals list hand-written), no SymbolId shadowing, no clone dedup beyond exact feature clones.
- Queries are synthetic (first ~5 lines of real methods), as in paper §5, not real developer snippets.

---

## Round 2 (D26): similar-first with a confidence gate on recommendations

Gate: ≥ 3 independent (non-clone) members, extension adds code but ≤ 2× query leaves, test units
excluded for production queries. Samples are internal research material, not published.

| Thresholds | apex-recipes shown | NebulaLogger shown | NPSP shown | Similar found (NPSP) |
|---|---|---|---|---|
| relaxed (0.5 / 1.2 / 0.8) | 4% | 5% | 24% | 68% |
| loose (0.4 / 1.1 / 0.7, Phase I 0.3) | 19% | 18% | 42% | 83% |

Median added code is now bounded (4–14 leaves vs 24–136 ungated).

**Usefulness of shown recommendations (relaxed + gate, 18 judged, my judgment):** ~5 better than the
top-1 similar method (NPSP savepoint→rollback, NPSP custom-metadata creation continuation,
apex-recipes DML try/catch → custom exception, DataWeave `getValue()` next step, trigger-handler task
insert), ~1 tie, ~12 worse. **Win rate ≈ 28% vs the agreed ≥ 60% bar → recommendations do not pass.**
Loose thresholds + gate: ~1–2 wins of 10 on NPSP.

Dominant failure: structurally generic shapes (`new List<X>(); if (…) return …; for (…) { … }`) satisfy
the cs/csq cluster tests although members are semantically unrelated; the similar baseline is often
a near-exact sibling and strictly better.

## Conclusion

- Ship **similar-code search** (Phase I + prune/rerank with matched-region highlighting) as the v1.0
  product: it finds a related unit for 55–83% of held-out partial queries on single repos.
- Clustered recommendations fail the agreed bar on single repos even with gating; they need either
  more corpus density (multi-repo) or a stronger relevance signal (e.g. IDF-weighted features so
  generic structure counts less) before they can be default-on.
