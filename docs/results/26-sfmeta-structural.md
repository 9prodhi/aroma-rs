# 26 — sfmeta structural ships (T29, 2026-11-20)

**D42 verdict on the validated tasks: structural wins.** Structural hit@5 is
7/15 (46.7%) versus lexical 3/15 (20.0%), a **26.6667 percentage-point** gain.
All three default-structural D39 recall gates pass with pooled start R@100 **1.0000**.
T29 ships the structural default; lexical remains an explicit experimental option.

## Default and compatibility

New builds select `SfmetaMode::BUILD_DEFAULT = Structural` explicitly. This applies
to plain `BuildOptions`, `expected_manifest`, CLI `aroma index`, recall, idiom, and
`spec_for(SfMeta)`. Queries of an existing index follow its manifest. Explicit
`--sfmeta-mode lexical` retains the BM25-only path and
`experimental_lexical_only: true` diagnostics.

`SfmetaMode::default()` and the manifest's `#[serde(default)]` remain **Lexical**:
a missing field describes an index built by the old lexical default. A regression
test removes the field from a real lexical index, opens and queries it, then checks
that a default build rebuilds it as structural.

**INDEX_SCHEMA_VERSION remains 3.** No serialized layout or existing mode's
interpretation changes. The recorded mode and adapter version already distinguish
lexical and structural indexes and trigger a rebuild when the requested build mode
changes. Bumping the schema would unnecessarily invalidate compatible indexes.

## Validation protocol

The sfmeta checker now shares the real runner's pinned v1.2 population (47 repos
plus the three pilots), metadata routing/ignore walk, source-unit selection,
exact-body-clone holdout, and incremental blank/rebuild/restore path. All holdouts
are computed against the intact baseline. It enumerates full post-holdout indexed
units without querying; witnesses can name any repository in the pool. Held-out,
absent, ignored and nonanswer witnesses still fail, as do required fragments in a
query under the existing whitespace/case normalization. Apex and LWC retain their
single-repository check semantics.

The task brief's original own-repository checker result was 0/15 (not rerun).
The first corrected pooled check exited 1: **5/15 passed**, with all **15/15**
already meeting the answer-count floor, **13 invalid witnesses** across ten tasks,
and **zero query leaks**. No idiom search or rank inspection is used for task repair. Passing
tasks may change only their declared witnesses; task kinds remain seven flow,
four validation rule, three formula field, and one custom metadata record.

## Fixtures and gates

The adapter now has **52 independently registered tests**, exposing the former
fixture table as individually named structural tests (37 construct cases), checking unit kind, structural evidence, source spans, labels and leaves.
Coverage includes every flow element type, formula/picklist and ordinary fields,
validation rules, CMDT, MDAPI object children, CDATA, namespaces, attributes,
comments and opaque entity text. Additional tests cover default-mode selection,
legacy manifest opening/rebuilding, exact XML error-byte accounting on both unit
and query paths, and D34 preservation of prior feature multiplicities under child
reordering and optional-child insertion. The existing broader D34 property suite
continues to run.

Unit tests cover pooled cross-repository answerability in both modes, cross-repo
clone removal, ignored files, invalid witnesses, query leaks and restoration on
error. Existing Apex/LWC checker regressions remain in place.

## Check results and witness corrections

All counts below are from enumeration, before any idiom search. No task had fewer
than two answers, so D48 required **no source replacement or idiom replacement**.
All sources, line ranges, queries, required fragments, notes, categories and pins
stay byte-identical. Five already-passing task files are untouched. Only witness
arrays in the ten failing task files change.

| Task | Pooled post-holdout answers | Held out | Invalid witnesses before |
|---|---:|---:|---:|
| meta01-flow-decision-priorvalue | 4 | 1 | 2 |
| meta02-flow-lookup-first-record | 441 | 1 | 0 |
| meta03-flow-update-input-reference | 294 | 2 | 1 |
| meta04-flow-loop-assignment | 137 | 1 | 0 |
| meta05-flow-screen-required-input | 506 | 1 | 1 |
| meta06-flow-subflow-inputs | 103 | 1 | 2 |
| meta07-flow-fault-connector | 128 | 1 | 2 |
| meta08-vr-ischanged-priorvalue | 2 | 1 | 1 |
| meta09-vr-ispickval-isblank | 27 | 1 | 0 |
| meta10-vr-profile-permission-bypass | 3 | 1 | 1 |
| meta11-vr-regex-format | 6 | 1 | 1 |
| meta12-formula-if-case | 8 | 1 | 1 |
| meta13-formula-date-today | 20 | 1 | 0 |
| meta14-formula-crossobject-hyperlink | 36 | 2 | 1 |
| meta15-cmdt-record-values | 899 | 1 | 0 |

Witness changes (the final TOML files carry exact repository/path/line locations):

- `meta01-flow-decision-priorvalue`: `navikt/crm-arbeidsgiver-base:force-app/main/default/flows/TAG_Event.flow-meta.xml:205` → `SalesforceLabs/Imhotep-App-Builder:force-app/main/default/flows/Imhotep_Story_AfterUpdated.flow:5`. Reason: not a post-holdout answer.
- `meta01-flow-decision-priorvalue`: `navikt/crm-ips:force-app/ipsPlatformEvent/flows/IPS_platformEventUserRole.flow-meta.xml:282` → `navikt/crm-arbeidsgiver-base:force-app/main/default/flows/TAG_CreateOpportunityfromRekrutteringshjelp.flow-meta.xml:5`. Reason: not a post-holdout answer.
- `meta03-flow-update-input-reference`: `SalesforceLabs/lwc-ui-orchestrator:force-app/main/default/flows/UIJobTaskFlow_Demo.flow-meta.xml:338` → `SalesforceLabs/AnimalShelterStarter:force-app/main/default/flows/Adopter_Match_clean_up_Subflow.flow-meta.xml:392`. Reason: held out (source or exact-body clone).
- `meta05-flow-screen-required-input`: `navikt/crm-ips:force-app/main/default/flows/IPS_User_And_Profile_Information.flow-meta.xml:49` → `navikt/crm-ips:force-app/main/default/flows/IPS_User_And_Profile_Information.flow-meta.xml:161`. Reason: not a post-holdout answer.
- `meta06-flow-subflow-inputs`: `navikt/crm-ips:force-app/main/default/flows/IPS_worktrailSetParticipantPassive.flow-meta.xml:1106` → `SalesforceLabs/AnimalShelterStarter:force-app/main/default/flows/Animal_Action_Vaccination_Process.flow-meta.xml:1057`. Reason: not a post-holdout answer.
- `meta06-flow-subflow-inputs`: `navikt/crm-ips:force-app/main/default/flows/IPS_Update_worktrail_status.flow-meta.xml:196` → `SalesforceLabs/AnimalShelterStarter:force-app/main/default/flows/Animal_Adoption_Update_Automation.flow-meta.xml:412`. Reason: not a post-holdout answer.
- `meta07-flow-fault-connector`: `navikt/crm-ips:force-app/main/default/flows/IPS_Archive_worktrail.flow-meta.xml:2046` → `navikt/crm-ips:force-app/main/default/flows/IPS_Archive_worktrail.flow-meta.xml:2024`. Reason: not a post-holdout answer.
- `meta07-flow-fault-connector`: `navikt/crm-ips:force-app/main/default/flows/IPS_Recruitment_Program.flow-meta.xml:352` → `SalesforceLabs/AnimalShelterStarter:force-app/main/default/flows/Adopter_Match_create_auto_match_records_Subflow.flow-meta.xml:352`. Reason: not a post-holdout answer.
- `meta08-vr-ischanged-priorvalue`: `navikt/crm-hot-tolk:force-app/main/default/objects/WorkOrder/validationRules/Account_cannot_be_edited.validationRule-meta.xml:5` → removed; the two valid witnesses remain. Reason: not a post-holdout answer; `crm-hot-tolk` is outside the pinned v1.2 pool.
- `meta10-vr-profile-permission-bypass`: `navikt/crm-hot-tolk:force-app/main/default/objects/HOT_Request__c/validationRules/ValidPersonNumber.validationRule-meta.xml:15` → removed; the two valid witnesses remain. Reason: not a post-holdout answer; `crm-hot-tolk` is outside the pinned v1.2 pool.
- `meta11-vr-regex-format`: `navikt/crm-hot-tolk:force-app/main/default/objects/HOT_Request__c/validationRules/ValidPersonNumber.validationRule-meta.xml:6` → removed; the two valid witnesses remain. Reason: not a post-holdout answer; `crm-hot-tolk` is outside the pinned v1.2 pool.
- `meta12-formula-if-case`: `navikt/crm-hot-tolk:force-app/main/default/objects/WorkOrder/fields/HOT_ExternalWorkOrderStatus__c.field-meta.xml:5` → removed; the two valid witnesses remain. Reason: not a post-holdout answer; `crm-hot-tolk` is outside the pinned v1.2 pool.
- `meta14-formula-crossobject-hyperlink`: `afawcett/declarative-lookup-rollup-summaries:dlrs/main/objects/LookupRollupSummaryScheduleItems__c/fields/ParentRecord__c.field-meta.xml:6` → `SalesforceFoundation/NPSP:unpackaged/config/trial/objects/npe5__Affiliation__c.object:12`. Reason: held out (source or exact-body clone).

The corrected checker exits **0: 15/15 tasks pass**, with the same counts as above,
zero witness errors and zero query leaks.

The nine replacement/corrected locations come from qualifying units of the same
metadata kind. Four unavailable `crm-hot-tolk` witnesses are removed. No ranking,
scoring, holdout, sampling, gate or threshold change was made.

## Evaluation commands

Run once each, only after the corrected `--check` exited 0, with the default
`target/` directory and `CARGO_PROFILE_DEV_DEBUG=0 CARGO_BUILD_JOBS=3`:

```sh
cargo run -q --release -p xtask -- idiom --suite sfmeta --sfmeta-mode structural
cargo run -q --release -p xtask -- idiom --suite sfmeta --sfmeta-mode lexical
cargo run -q --release -p xtask -- recall --suite v1.2
```

The recall command deliberately omits `--sfmeta-mode` to exercise the new default.
These runs reuse the provided hard-linked corpus, never execute corpus files, and
retain the original sampling, scoring, ranking, source/clone holdout and gates.

## Results

Both idiom commands exited 0 on the validated tasks:

| Mode | Similar hit@5 |
|---|---:|
| Structural | **7/15 = 46.7%** |
| Lexical | **3/15 = 20.0%** |

The gain is **4/15 = 26.6667 percentage points**, above D42's unchanged five-point
minimum. Sources and queries did not change, so the measured values also agree
with note 24. The validated-suite idiom portion of D42 passes. The default-structural v1.2
recall command also exited **0**, passing all three unchanged D39 start-slice gates.
Together these re-establish **D42: structural wins**. The T28 corpus-floor evidence
in note 24 remains applicable: no corpus pins, population, routing, ignore policy,
extractor or normalizer implementation changed.

Default-structural recall (query-weighted pools, identical sample counts to note 24):

| Kind | Offset | Queries | R@1 | R@100 | Gate |
|---|---|---:|---:|---:|---|
| `field` | end | 6764 | 0.4935 | 0.9230 | reported |
| `field` | middle | 6764 | 0.9043 | 0.9931 | reported |
| `field` | start | 6764 | 0.9279 | 1.0000 | ≥ 0.95 |
| `flow_element` | end | 6232 | 0.6091 | 0.9769 | reported |
| `flow_element` | middle | 6232 | 0.6101 | 0.9228 | reported |
| `flow_element` | start | 6232 | 0.8798 | 1.0000 | ≥ 0.95 |
| `markup_block` | end | 1576 | 0.9867 | 1.0000 | reported |
| `markup_block` | middle | 1576 | 0.9873 | 0.9905 | reported |
| `markup_block` | start | 1576 | 0.9918 | 1.0000 | reported |
| `metadata_record` | end | 838 | 0.3771 | 0.9976 | reported |
| `metadata_record` | middle | 838 | 0.2661 | 0.9236 | reported |
| `metadata_record` | start | 838 | 0.9463 | 0.9976 | reported |
| `validation_rule` | end | 150 | 0.9533 | 0.9933 | reported |
| `validation_rule` | middle | 150 | 0.8533 | 1.0000 | reported |
| `validation_rule` | start | 150 | 0.9933 | 1.0000 | ≥ 0.95 |

The unchanged gates concern start slices. Middle/end figures remain reported;
for example, flow-element middle R@100 is 0.9228 and CMDT-record middle R@100 is
0.9236. The verdict does not establish improvement for every fragment category.

Final command tails:

```text
idiom check: 15/15 tasks pass (>= 2 post-holdout answers in v1.2 pool; valid declared witnesses; leak-free queries)
idiom sfmeta (structural): similar hit@5 7/15 = 46.7%
idiom sfmeta (lexical): similar hit@5 3/15 = 20.0%
v1.2 recall gates passed: flow_element pooled start n=6232 R@100 1.0000 (gate 0.95); validation_rule pooled start n=150 R@100 1.0000 (gate 0.95); field pooled start n=6764 R@100 1.0000 (gate 0.95)
```

Raw logs live in `target/t29/`.

## Verification

All commands ran with `CARGO_PROFILE_DEV_DEBUG=0 CARGO_BUILD_JOBS=3`:

- `cargo fmt --all -- --check`: exit 0 (empty log).
- `cargo clippy --workspace --all-targets -- -D warnings`: exit 0;
  `Finished dev profile [unoptimized] target(s) in 3.18s`.
- `cargo test --workspace`: exit 0; **644 passed, 0 failed, 0 ignored**.
  The sfmeta adapter reports **52 passed** and its broader D34 feature suite reports
  **7 passed**. The 520-case golden differential replay and exact Java Compat tree
  checks pass. Apex, LWC and VF tests pass; their adapters and goldens are unedited.
  Final xtask tail: `56 passed; 0 failed; 0 ignored`.

The first workspace run exposed one remaining lexical-default assumption in the
generic feature-hook test. It now explicitly checks sfmeta's structural default,
retains the lexical spec's original configuration assertion, and still requires
identity configurations for every other language. The complete workspace run
above is after that correction.

Logs: `target/t29/{check-before,check-after,idiom-structural,idiom-lexical,recall-v12,
workspace-tests,clippy,fmt}.log`. No commit, branch creation or push was performed.



## Files changed

- Defaults and compatibility: `crates/aroma-core/src/store/manifest.rs`,
  `crates/aroma-lang/src/build.rs`, `crates/aroma-lang/src/spec/{mod,sfmeta}.rs`,
  `crates/aroma-cli/src/main.rs`.
- Harness: `xtask/src/{idiom,recall}.rs`.
- Tests: `crates/aroma-cli/tests/cli.rs`,
  `crates/aroma-lang/tests/{hooks,sfmeta_adapter,sfmeta_features}.rs`.
- Witness-only task edits: sfmeta `meta01`, `meta03`, `meta05`, `meta06`, `meta07`,
  `meta08`, `meta10`, `meta11`, `meta12`, `meta14` TOMLs. The other five are untouched.
- Documentation: `README.md`, `docs/usage.md`,
  `docs/PLAN-aroma-rs-v1.1-v1.2.md`, this note.

No edits touch snippet JS wrappers, Aura files, pilots, paper artifacts or paper sources.
