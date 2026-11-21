# Project summary

This directory holds a one-page summary of aroma-rs and the source list behind it.

- [`summary.html`](summary.html) is the one-page project summary: what aroma-rs is,
  the release gates as measured, limitations and a map from each summary sentence to
  its source IDs. Open it in a browser. Its typography loads external Google Fonts,
  so viewing it may make network requests (unlike local CLI search).
- [`sources.md`](sources.md) is the numbered E01–E25 source list behind that page.
  It gives the origin and bounds of each statement; it is not a new experiment.
  Process statements and snapshot test counts are not substitutes for reproducible
  measurement records.

## Source IDs and release records

Public release records **17–29** live in [`docs/results/`](../results/README.md).
Other research notes are internal and not published; the source list marks those
as internal notes. It also cites design plans and code where appropriate. Not every
ID is a benchmark.

| IDs / context | Public records and other provenance |
|---|---|
| E01–E03: scope, languages and adapters | design plans and language/spec code; [27](../results/27-v1.1-release-gate.md), [28](../results/28-v1.3-recall.md), [29](../results/29-v1.2-v1.3-release-gates.md); grammar probes: internal note |
| E04–E06: compatibility, workspace snapshot and extensions | [19](../results/19-v1-release-gates.md), code/test snapshots and plan D36/D45/D50; idiom history: internal note |
| E07–E08: pilot and experimental recommendations | [17](../results/17-pilot-results.md), [19](../results/19-v1-release-gates.md) |
| E09–E11: recall, holdout and seed protocol | [19](../results/19-v1-release-gates.md), [26](../results/26-sfmeta-structural.md), plan D43/D48; tie-break and LWC audits: internal notes |
| E12–E13: Java/Apex recall | [19](../results/19-v1-release-gates.md); tie-break investigation: internal note |
| E14–E18: LWC recall, member parsing and task validity | [27](../results/27-v1.1-release-gate.md); per-query/task audits: internal notes |
| E19–E20: metadata comparison, recall and corpus floors | [24](../results/24-metadata-verdict.md), [26](../results/26-sfmeta-structural.md), [29](../results/29-v1.2-v1.3-release-gates.md) |
| E21–E22: Aura/Visualforce recall and component join | [28](../results/28-v1.3-recall.md), [29](../results/29-v1.2-v1.3-release-gates.md); corpus scout: internal note |
| E23: performance | [19](../results/19-v1-release-gates.md), [29](../results/29-v1.2-v1.3-release-gates.md) |
| E24: AI-assisted process | internal process record, not a release benchmark |
| E25: limitations | [19](../results/19-v1-release-gates.md), [27](../results/27-v1.1-release-gate.md), [29](../results/29-v1.2-v1.3-release-gates.md); ranking investigation: internal note |

Use record 19's **correction and resolution**, record 27's **later D50 change**,
and record 28's **corpus-addendum rerun**; initial figures are not all final release
results. Record 29 is the final v1.2/v1.3 gate and all-language performance record.

Self-retrieval recall leaves the source indexed; it measures findability, not
usefulness. The small developer-written idiom suites instead hold out the source
and exact clones. The summary records the LWC JS R@1 shortfall and the Java gate's
thin margin. It is not evidence of a user study or a multi-machine performance
comparison.

## AI assistance

The project and this summary were developed with **Claude orchestrating Codex, Pi
and Antigravity agents**, and humans review the work. The summary's numbers were
checked against the release records; check statements against the linked records,
code and reproducible harnesses before reusing them.

For build instructions, gate reproduction and licensing boundaries for third-party
reference artifacts and corpora, see the [project README](../../README.md).
