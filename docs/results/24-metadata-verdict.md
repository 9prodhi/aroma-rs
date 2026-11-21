# 24 — Metadata spike evaluation and D42 verdict (2026-11-20)

**D42 verdict: structural wins.** Structural scores 7/15 = 46.7% versus lexical 3/15 = 20.0% (**+26.6667 percentage points**). All three structural D39 recall gates and all corpus floors pass. Proceed to T29 on this evidence; this evaluation change leaves the runtime default lexical.

## Protocol

The population is the 47 repositories tagged `v1.2` in
`xtask/corpora/salesforce.toml`, plus the three M-1 pilots at their existing pins:
`trailheadapps/apex-recipes`, `jongpie/NebulaLogger`, and
`SalesforceFoundation/NPSP`. Both harnesses validate/reuse those exact checkouts.
The existing `target/corpus/sf/` copy was reused. The missing pilots were copied
from the existing local pinned checkouts; no repository was fetched over the network.
Excluded/unlicensed scout repositories and repositories without stage `v1.2`
are absent from the evaluation.

Commands, using the default `target/` directory:

```sh
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_BUILD_JOBS=3
cargo run -q --release -p xtask -- recall --suite v1.2 --sfmeta-mode lexical
cargo run -q --release -p xtask -- recall --suite v1.2 --sfmeta-mode structural
cargo run -q --release -p xtask -- idiom --suite sfmeta --sfmeta-mode lexical
cargo run -q --release -p xtask -- idiom --suite sfmeta --sfmeta-mode structural
```

Recall builds **each repository separately**, then pools individual query outcomes
by `(kind, offset)`, weighting queries rather than repositories. Sampling uses
seed 119, at most 1,000 units **per kind per repository**, minimum four unit lines
and the existing markup policy's three nonblank interior lines. A query contains
`min(5, interior lines)` lines at the start, middle, or end; random-line queries
are also reported. The source remains indexed; its displayed result rank, or
that of an equal `body_hash` clone, determines recall. Only the pooled **start**
R@100 for `flow_element`, `validation_rule`, and `field` is gated at 0.95, and a
kind with fewer than 100 queries is reported without gating. Middle/end slices
and the other kinds are reported, not gated. The `field` kind includes ordinary
fields as well as formula fields, per D35; formula-field coverage is counted
separately below.

Idiom uses **one pooled metadata index of all 50 repositories**, with a repository
prefix on every path. Its initial population follows the indexer's routing and
ignore rules. Per task, the existing harness's smallest-covering-unit selection,
overlap fallback, and equal-body-hash clone holdout run against the intact pool,
including clones in other repositories. The same pool is incrementally rebuilt
with that task's sources blanked, scored, and restored before the next task.
File replacement prevents writes through corpus hard links. A hit is one of the
top five similar results whose **full source lines** contain every required
fragment, using the existing whitespace/case normalization. Pruned snippets,
recommendations, and task witnesses do not decide hits. The original 15 TOML
tasks are unchanged. The Apex/LWC absolute 60% idiom gate does not apply here.

D42 is applied as an **absolute percentage-point** difference:
`100 * (structural_hits - lexical_hits) / 15 >= 5`, together with passing D39
recall. On this suite one additional hit is 6.6667 points. No mode, task, gate,
or threshold was tuned in response to the results.

## Extraction coverage and corpus floors

Both modes index **16,962 units** from **8,155 routed files**, with zero failed
files, large-file skips, or damaged-unit skips. A read-only XML/source-span audit
finds every expected base unit in the routed population; there are no extraction
misses in either mode. It reconstructs the intact population from the last two
pooled generations, whose final field/CMDT holdouts are disjoint, and checks that
the union equals the initial 16,962-unit build.

| Kind / measure | Note 21 text count (internal research note, not published) | Lexical extracted | Structural extracted |
|---|---:|---:|---:|
| Flow files | 902 | 895 | 895 |
| Flows with extracted elements | — | 869 | 869 |
| `flow_element` | 6,863 | 6,835 | 6,835 |
| `validation_rule` | 150 | 150 | 150 |
| Formula fields (subset of `field`) | 510 | 519 | 519 |
| All `field` units | — | 6,959 | 6,959 |
| `metadata_record` | — | 840 | 840 |
| D44 `markup_block` split units | — | 2,178 | 2,178 |

The 7-flow / 28-element shortfall against the scout is entirely explained by
`alexed1/LightningFlowComponents/.gitignore:9` (`**mdapi`). The normal index walk
excludes those tracked MDAPI files as well as ignored untracked files. Pooling
preserves that policy. The excluded flows are the six Summer18 ShowToast,
GetRESTData, and PlaySound examples, and SendRichEmailCPE's
`Send_Rich_Email_Testflow.flow`; their element counts are 6, 2, 2, 2, 5, 3, and 8.
No ignore rule or extractor was changed to close the scout gap.

The nine additional formula fields use a multiline `<formula\n    >` opening
tag, which the scout's literal `<formula>` test missed: one in crm-nks-base,
three in coral-cloud, two each in dreamhouse-lwc and dreamhouse-sfdx, and one in
purealoe-lwc. All 510 fields counted by the scout are also extracted. Apparent
ordinary-field gaps in naive `.object` text scans are compact-layout field
references, not direct `CustomObject/fields` definitions (EDA has twelve such
references). The XML parent-aware audit excludes these references. The auxiliary
ordinary-field text scan counted 7,476 tags/files: 515 compact-layout references
and two ignored field definitions explain the difference from 6,959 indexed
fields. The ignored definitions are ActionPlansV4's
`objects/Activity/fields/TaskAPTask__c.field-meta.xml` (`**/objects/Activity`)
and the Quip MDAPI object in LightningFlowComponents (`**mdapi`); neither is a
formula field. No parser/extractor change is needed for these exclusions.

The D39 corpus floors are met in both modes: **6,835 ≥ 1,000 flow elements from
869 ≥ 50 flows; 150 ≥ 150 validation rules; 519 ≥ 150 formula fields**. Split
blocks are additional coverage and are not counted toward those base-unit floors.

Per-repository extraction (identical in both modes):

| Repository | Flow elements | Fields | Formula fields | Rules | CMDT records | Split blocks |
|---|---:|---:|---:|---:|---:|---:|
| SFDC-Assets/Get-Geolocation | 2 | 0 | 0 | 0 | 0 | 0 |
| SalesforceFoundation/Contacts_and_Organizations | 0 | 60 | 7 | 0 | 0 | 0 |
| SalesforceFoundation/EDA | 0 | 468 | 11 | 2 | 4 | 1160 |
| SalesforceFoundation/Households | 0 | 158 | 10 | 0 | 0 | 0 |
| SalesforceFoundation/OutboundFundsModule | 0 | 56 | 1 | 0 | 0 | 0 |
| SalesforceFoundation/PMM | 17 | 104 | 15 | 13 | 31 | 0 |
| SalesforceFoundation/Recurring_Donations | 0 | 34 | 3 | 0 | 0 | 0 |
| SalesforceFoundation/Relationships | 0 | 30 | 1 | 1 | 0 | 0 |
| SalesforceFoundation/Volunteers-for-Salesforce | 0 | 101 | 11 | 7 | 0 | 0 |
| SalesforceFoundation/k12-architecture-kit | 0 | 44 | 2 | 0 | 0 | 579 |
| SalesforceLabs/ActionPlansV4 | 4 | 65 | 5 | 8 | 7 | 0 |
| SalesforceLabs/AnimalShelterStarter | 534 | 290 | 37 | 9 | 4 | 346 |
| SalesforceLabs/ApexPulse | 8 | 21 | 1 | 0 | 0 | 0 |
| SalesforceLabs/Configurable-Self-Registration-for-Experience-Cloud | 1 | 107 | 2 | 18 | 18 | 70 |
| SalesforceLabs/Deadline-Manager-for-Orchestrator | 36 | 17 | 0 | 0 | 0 | 0 |
| SalesforceLabs/EnhancedLightningGrid | 0 | 31 | 0 | 3 | 0 | 0 |
| SalesforceLabs/IT-Manager | 6 | 14 | 1 | 0 | 0 | 0 |
| SalesforceLabs/Imhotep-App-Builder | 509 | 135 | 11 | 1 | 0 | 0 |
| SalesforceLabs/LSStarterConfig | 4 | 522 | 0 | 0 | 0 | 0 |
| SalesforceLabs/Product-Data-Management | 29 | 5 | 0 | 0 | 0 | 0 |
| SalesforceLabs/Quicker-Quotes | 158 | 10 | 0 | 0 | 0 | 0 |
| SalesforceLabs/SDGStarterPack | 13 | 28 | 0 | 0 | 0 | 0 |
| SalesforceLabs/UnsubscribeLink | 38 | 5 | 0 | 0 | 0 | 0 |
| SalesforceLabs/Work-Order-Utility-Flow | 16 | 0 | 0 | 0 | 0 | 0 |
| SalesforceLabs/lwc-ui-orchestrator | 36 | 20 | 0 | 1 | 30 | 0 |
| SalesforceLabs/revenue-cloud-foundations | 757 | 121 | 29 | 0 | 5 | 0 |
| afawcett/declarative-lookup-rollup-summaries | 0 | 136 | 3 | 2 | 6 | 0 |
| alexed1/LightningFlowComponents | 1603 | 501 | 1 | 1 | 2 | 23 |
| alexed1/approvalsOnFlow | 242 | 65 | 0 | 0 | 0 | 0 |
| certinia/alexa-salesforce-flow-skill-examples | 49 | 57 | 7 | 0 | 0 | 0 |
| navikt/crm-arbeidsforhold | 406 | 200 | 20 | 1 | 29 | 0 |
| navikt/crm-arbeidsgiver-base | 476 | 635 | 172 | 38 | 123 | 0 |
| navikt/crm-ips | 946 | 471 | 30 | 32 | 128 | 0 |
| navikt/crm-nks-base | 361 | 88 | 23 | 3 | 134 | 0 |
| navikt/crm-sto | 258 | 17 | 2 | 0 | 11 | 0 |
| trailheadapps/agent-script-recipes | 125 | 95 | 0 | 0 | 0 | 0 |
| trailheadapps/az-insurance | 1 | 6 | 0 | 0 | 0 | 0 |
| trailheadapps/coral-cloud | 34 | 63 | 19 | 0 | 0 | 0 |
| trailheadapps/dreamhouse-lwc | 18 | 32 | 5 | 0 | 0 | 0 |
| trailheadapps/dreamhouse-sfdx | 16 | 49 | 5 | 0 | 0 | 0 |
| trailheadapps/easy-spaces | 12 | 32 | 0 | 0 | 2 | 0 |
| trailheadapps/easy-spaces-lwc | 13 | 34 | 0 | 0 | 2 | 0 |
| trailheadapps/ecars | 7 | 22 | 0 | 0 | 0 | 0 |
| trailheadapps/lwc-recipes | 3 | 28 | 0 | 0 | 0 | 0 |
| trailheadapps/pronto | 11 | 52 | 4 | 0 | 0 | 0 |
| trailheadapps/purealoe | 31 | 45 | 1 | 0 | 0 | 0 |
| trailheadapps/purealoe-lwc | 31 | 45 | 1 | 0 | 0 | 0 |
| trailheadapps/apex-recipes | 1 | 30 | 0 | 0 | 11 | 0 |
| jongpie/NebulaLogger | 23 | 817 | 61 | 5 | 100 | 0 |
| SalesforceFoundation/NPSP | 0 | 993 | 18 | 5 | 193 | 0 |

## Recall results

Pooled query-weighted results (four decimals). All sample counts agree between modes.

| Kind | Offset | Queries per mode | Lexical R@1 | Lexical R@100 | Structural R@1 | Structural R@100 | Gate |
|---|---|---:|---:|---:|---:|---:|---|
| `flow_element` | start | 6232 | 0.8233 | 0.9998 | 0.8798 | 1.0000 | R@100 ≥ 0.95 |
| `flow_element` | middle | 6232 | 0.6115 | 0.9815 | 0.6101 | 0.9228 | reported |
| `flow_element` | end | 6232 | 0.4982 | 0.9584 | 0.6091 | 0.9769 | reported |
| `validation_rule` | start | 150 | 0.9933 | 1.0000 | 0.9933 | 1.0000 | R@100 ≥ 0.95 |
| `validation_rule` | middle | 150 | 0.9000 | 1.0000 | 0.8533 | 1.0000 | reported |
| `validation_rule` | end | 150 | 0.9867 | 1.0000 | 0.9533 | 0.9933 | reported |
| `field` | start | 6764 | 0.8989 | 0.9999 | 0.9279 | 1.0000 | R@100 ≥ 0.95 |
| `field` | middle | 6764 | 0.8634 | 0.9880 | 0.9043 | 0.9931 | reported |
| `field` | end | 6764 | 0.4264 | 0.8446 | 0.4935 | 0.9230 | reported |
| `metadata_record` | start | 838 | 0.9021 | 0.9976 | 0.9463 | 0.9976 | reported |
| `metadata_record` | middle | 838 | 0.4535 | 0.9630 | 0.2661 | 0.9236 | reported |
| `metadata_record` | end | 838 | 0.3210 | 1.0000 | 0.3771 | 0.9976 | reported |
| `markup_block` | start | 1576 | 0.9911 | 0.9994 | 0.9918 | 1.0000 | reported |
| `markup_block` | middle | 1576 | 0.9879 | 0.9987 | 0.9873 | 0.9905 | reported |
| `markup_block` | end | 1576 | 0.9860 | 0.9956 | 0.9867 | 1.0000 | reported |

The four real-run summary lines, with recall ordered lexical then structural:

```text
v1.2 recall gates passed: flow_element pooled start n=6232 R@100 0.9998 (gate 0.95); validation_rule pooled start n=150 R@100 1.0000 (gate 0.95); field pooled start n=6764 R@100 0.9999 (gate 0.95)
v1.2 recall gates passed: flow_element pooled start n=6232 R@100 1.0000 (gate 0.95); validation_rule pooled start n=150 R@100 1.0000 (gate 0.95); field pooled start n=6764 R@100 1.0000 (gate 0.95)
idiom sfmeta (lexical): similar hit@5 3/15 = 20.0%
idiom sfmeta (structural): similar hit@5 7/15 = 46.7%
```

Middle/end results describe fragment limitations separately from the start-slice gates. In particular, lexical field end-slice R@100 is 0.8446. No offset was dropped or promoted into a gate after seeing results.
Structural middle-slice R@100 falls from 0.9815 to 0.9228 for flow elements
and from 0.9630 to 0.9236 for CMDT records. These remain reported under D43.
The structural-wins verdict follows the specified start-slice and relative
idiom gates; it does not establish an improvement for every fragment category.

## Idiom results

| Task | Lexical top-5 hit/rank | Structural top-5 hit/rank | Held-out units (lexical / structural) |
|---|---|---|---|
| meta01-flow-decision-priorvalue | miss | miss | 1 / 1 |
| meta02-flow-lookup-first-record | miss | hit, rank 1 | 1 / 1 |
| meta03-flow-update-input-reference | miss | miss | 2 / 2 |
| meta04-flow-loop-assignment | miss | hit, rank 1 | 1 / 1 |
| meta05-flow-screen-required-input | hit, rank 1 | hit, rank 1 | 1 / 1 |
| meta06-flow-subflow-inputs | hit, rank 2 | hit, rank 1 | 1 / 1 |
| meta07-flow-fault-connector | miss | miss | 1 / 1 |
| meta08-vr-ischanged-priorvalue | miss | miss | 1 / 1 |
| meta09-vr-ispickval-isblank | miss | hit, rank 4 | 1 / 1 |
| meta10-vr-profile-permission-bypass | miss | hit, rank 4 | 1 / 1 |
| meta11-vr-regex-format | miss | miss | 1 / 1 |
| meta12-formula-if-case | miss | miss | 1 / 1 |
| meta13-formula-date-today | miss | miss | 1 / 1 |
| meta14-formula-crossobject-hyperlink | miss | miss | 2 / 2 |
| meta15-cmdt-record-values | hit, rank 1 | hit, rank 1 | 1 / 1 |

Lexical: **3/15 = 20.0%**. Structural: **7/15 = 46.7%**. The gain is
**4/15 = 26.6667 percentage points**, so the idiom portion of D42 passes.
Holdout counts agree in both modes for every task.
The four additional structural hits are meta02, meta04, meta09, and meta10.
Both modes miss all three formula-field tasks (meta12–meta14), an important
limit on the observed answer quality.

## Index times

These are measured wall-clock observations, not isolated performance benchmarks.
The recall and idiom pairs ran concurrently on separate index roots. Lexical
recall reused the first three indexes after the interrupted diagnostic run;
structural recall rebuilt every file when the stored mode changed. The table
shows `reparsed / unchanged` files so the incremental work is explicit. Pooled
idiom baseline builds started fresh in both modes. Times exclude compilation.

| Repository | Lexical seconds | Lexical files (reparsed / unchanged) | Structural seconds | Structural files (reparsed / unchanged) |
|---|---:|---:|---:|---:|
| SFDC-Assets/Get-Geolocation | 0.198 | 0 / 1 | 0.280 | 1 / 0 |
| SalesforceFoundation/Contacts_and_Organizations | 0.219 | 0 / 7 | 0.419 | 7 / 0 |
| SalesforceFoundation/EDA | 0.809 | 0 / 478 | 7.995 | 478 / 0 |
| SalesforceFoundation/Households | 0.451 | 8 / 0 | 0.334 | 8 / 0 |
| SalesforceFoundation/OutboundFundsModule | 0.310 | 56 / 0 | 0.251 | 56 / 0 |
| SalesforceFoundation/PMM | 0.767 | 150 / 0 | 0.356 | 150 / 0 |
| SalesforceFoundation/Recurring_Donations | 0.362 | 5 / 0 | 0.238 | 5 / 0 |
| SalesforceFoundation/Relationships | 0.282 | 6 / 0 | 0.222 | 6 / 0 |
| SalesforceFoundation/Volunteers-for-Salesforce | 1.067 | 12 / 0 | 0.605 | 12 / 0 |
| SalesforceFoundation/k12-architecture-kit | 6.836 | 88 / 0 | 2.610 | 88 / 0 |
| SalesforceLabs/ActionPlansV4 | 0.353 | 82 / 0 | 0.267 | 82 / 0 |
| SalesforceLabs/AnimalShelterStarter | 4.330 | 364 / 0 | 2.071 | 364 / 0 |
| SalesforceLabs/ApexPulse | 0.331 | 22 / 0 | 0.299 | 22 / 0 |
| SalesforceLabs/Configurable-Self-Registration-for-Experience-Cloud | 0.729 | 144 / 0 | 0.497 | 144 / 0 |
| SalesforceLabs/Deadline-Manager-for-Orchestrator | 0.619 | 11 / 0 | 0.304 | 11 / 0 |
| SalesforceLabs/EnhancedLightningGrid | 0.390 | 6 / 0 | 0.255 | 6 / 0 |
| SalesforceLabs/IT-Manager | 0.335 | 17 / 0 | 0.229 | 17 / 0 |
| SalesforceLabs/Imhotep-App-Builder | 3.367 | 58 / 0 | 1.723 | 58 / 0 |
| SalesforceLabs/LSStarterConfig | 0.818 | 524 / 0 | 0.508 | 524 / 0 |
| SalesforceLabs/Product-Data-Management | 0.333 | 12 / 0 | 0.427 | 12 / 0 |
| SalesforceLabs/Quicker-Quotes | 1.008 | 16 / 0 | 0.770 | 16 / 0 |
| SalesforceLabs/SDGStarterPack | 0.343 | 33 / 0 | 0.240 | 33 / 0 |
| SalesforceLabs/UnsubscribeLink | 0.325 | 6 / 0 | 0.272 | 6 / 0 |
| SalesforceLabs/Work-Order-Utility-Flow | 0.317 | 3 / 0 | 0.212 | 3 / 0 |
| SalesforceLabs/lwc-ui-orchestrator | 0.396 | 55 / 0 | 0.297 | 55 / 0 |
| SalesforceLabs/revenue-cloud-foundations | 2.884 | 199 / 0 | 1.913 | 199 / 0 |
| afawcett/declarative-lookup-rollup-summaries | 0.426 | 144 / 0 | 0.336 | 144 / 0 |
| alexed1/LightningFlowComponents | 6.704 | 785 / 0 | 6.937 | 785 / 0 |
| alexed1/approvalsOnFlow | 1.277 | 87 / 0 | 1.681 | 87 / 0 |
| certinia/alexa-salesforce-flow-skill-examples | 0.689 | 17 / 0 | 2.298 | 17 / 0 |
| navikt/crm-arbeidsforhold | 1.848 | 267 / 0 | 1.953 | 267 / 0 |
| navikt/crm-arbeidsgiver-base | 3.456 | 858 / 0 | 2.258 | 858 / 0 |
| navikt/crm-ips | 4.473 | 714 / 0 | 5.742 | 714 / 0 |
| navikt/crm-nks-base | 2.122 | 252 / 0 | 1.564 | 252 / 0 |
| navikt/crm-sto | 1.045 | 62 / 0 | 0.747 | 62 / 0 |
| trailheadapps/agent-script-recipes | 0.573 | 156 / 0 | 0.666 | 156 / 0 |
| trailheadapps/az-insurance | 0.267 | 7 / 0 | 0.327 | 7 / 0 |
| trailheadapps/coral-cloud | 0.489 | 73 / 0 | 0.918 | 73 / 0 |
| trailheadapps/dreamhouse-lwc | 0.337 | 33 / 0 | 1.057 | 33 / 0 |
| trailheadapps/dreamhouse-sfdx | 0.394 | 53 / 0 | 0.442 | 53 / 0 |
| trailheadapps/easy-spaces | 0.312 | 36 / 0 | 0.479 | 36 / 0 |
| trailheadapps/easy-spaces-lwc | 0.436 | 38 / 0 | 0.862 | 38 / 0 |
| trailheadapps/ecars | 0.337 | 24 / 0 | 0.557 | 24 / 0 |
| trailheadapps/lwc-recipes | 0.272 | 29 / 0 | 1.108 | 29 / 0 |
| trailheadapps/pronto | 0.541 | 57 / 0 | 1.483 | 57 / 0 |
| trailheadapps/purealoe | 0.425 | 49 / 0 | 0.697 | 49 / 0 |
| trailheadapps/purealoe-lwc | 0.593 | 49 / 0 | 1.202 | 49 / 0 |
| trailheadapps/apex-recipes | 0.306 | 42 / 0 | 0.500 | 42 / 0 |
| jongpie/NebulaLogger | 1.646 | 932 / 0 | 2.179 | 932 / 0 |
| SalesforceFoundation/NPSP | 2.470 | 1028 / 0 | 1.766 | 1028 / 0 |

| Index operation | Lexical seconds | Structural seconds |
|---|---:|---:|
| Sum of 50 repository index builds | 59.614 | 61.354 |
| Fresh pooled idiom baseline | 73.665 | 57.989 |
| 15 pooled holdout rebuilds (sum) | 68.478 | 67.926 |
| Recall command, including queries | 1368.832 | 3156.802 |
| Idiom command, including queries | 148.179 | 131.687 |

Untracked reproducibility artifacts are under `target/t28b-eval/`: the four
raw command logs, per-task timings, pooled extraction audit, corpus integrity
checks and workspace test output (local artifacts, not published).

## Harness fix and verification

The existing recall sampler reparsed and re-extracted a whole source file for
each sampled unit. EDA alone yields 1,160 D44 split markup units, making that
repeated work expensive. Metadata sampling now caches body interiors per file;
Java/Apex/LWC sampling is unchanged. A regression test checks cached interiors
against explicit expected lines and the individual-unit extraction path.
The initial uncached full run was stopped before it produced a complete gate
result; the four reported runs use the final harness.

Before/after check: `recall --repo target/corpus/sf/SalesforceFoundation__EDA
--lang sfmeta --samples 1` produced **byte-identical report text**. Both runs
sampled four kinds, recalled 4/4 start queries at R@100, and recalled all 12
start/middle/end queries at R@100 (1.000 for each). Elapsed probe time was
8.52 s before and 6.17 s after; this is a diagnostic, not a performance gate.
No adapter, ranking, extraction, or holdout fix was made, and neither mode was tuned.

Offline tests cover suite/mode selection and rejection, the exact pinned
population, cross-repository clone holdout, full-source scoring, restoration
between tasks in both modes, metadata overlap fallback, per-kind sampling, and
the inclusive 0.95/100-query gate boundary.

Required gates: `cargo fmt --all -- --check` passed;
`cargo clippy --workspace --all-targets -- -D warnings` passed;
`cargo test --workspace` passed (**589 passed, 0 failed, 0 ignored**).

All 50 corpus checkouts were checked again after idiom evaluation: each remains
at its pinned SHA with clean tracked source. No branch, commit, or push was made.
