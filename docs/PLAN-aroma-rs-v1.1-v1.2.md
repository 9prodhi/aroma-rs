# PLAN: aroma-rs v1.1 (LWC) + v1.2 (Salesforce metadata) + v1.3 (Aura, Visualforce)

Status: Historical expansion plan, 2026-11-20 · Builds on v1.0 (`main`
0f3a2a6, `docs/results/19-v1-release-gates.md`) · Parent spec: `docs/PLAN-aroma-rs.md` (U4/D5 staging,
§4 adapter table, M6/M7).

Public release records are indexed in [results](results/README.md). Other numbered
notes cited by this historical plan are internal research notes, not published.
The final release records supersede preliminary task-note assignments below.

## 0. Summary

v1.1 adds Lightning Web Components: JavaScript modules, HTML templates, and whole-component search
that joins the two. v1.2 adds Salesforce XML metadata (flow elements, custom fields, validation rules,
custom metadata records). v1.3 adds Aura bundles and Visualforce pages. Every new language is a
**tree-sitter `LangSpec` table**, not a hand-written adapter, so it inherits v1.0's error policy,
limits, snippet repair, BM25 channel, recall harness and storage.

v1.2 is evidence-first. A spike builds a BM25-only metadata baseline before any shared-core change.
Structural metadata search ships only if it beats that baseline.

Before any of this, the v1.0 R@1 scorer is fixed and the v1.0 recall gates are re-run (D40).

## 1. Decisions (numbering continues the parent spec; review question ids in brackets)

| # | Decision | Answer |
|---|---|---|
| D31 [D2] | Scope | Full and sequenced: scorer fix → shared hooks → LWC JS → LWC HTML → component join → **v1.1**; metadata spike (BM25 baseline, then structural prototype) → metadata adapter → **v1.2**; Aura markup + Aura JS + Visualforce → **v1.3** |
| D32 [D3] | Adapter architecture (supersedes the "custom adapters" note in parent §3.1/§4 and D12's "LWC HTML and Salesforce XML stay custom") | Every new language is a tree-sitter `LangSpec` table: `tree-sitter-javascript` 0.25.0, `tree-sitter-html` 0.23.2, `tree-sitter-xml` 0.7.0. roxmltree is dropped because it rejects malformed XML, so it cannot parse partial snippets. Language rewrites go in the existing `Normalizer` trait. Shared-pipeline hooks: unit builder (D41), per-language feature config, term rule (D37), omission marker, repair strategy (D36) |
| D33 [D4] | Language identity and file routing | New `Lang` values: `lwcjs`, `lwchtml`, `sfmeta` (v1.1/v1.2) and `aura`, `aurajs`, `vf` (v1.3). Each has its own feature namespace. **T24 adds all six variants up front.** `LangSpec.file_extensions` becomes `matches(rel_path) -> bool`. Rules: LWC files under `…/lwc/<component>/` (Jest `__tests__/` indexed with `is_test = true`); Aura under `…/aura/<bundle>/` (`.cmp`, `.app`, `.evt`, `.intf` = `aura`; `*Controller.js`, `*Helper.js`, `*Renderer.js` = `aurajs`); Visualforce `.page` and `.component` (not `-meta.xml`). Metadata allowlist: `.flow-meta.xml`, `.validationRule-meta.xml`, `.field-meta.xml`, `.md-meta.xml`, MDAPI `.object`, `.flow`. Everything else is skipped and counted per reason in `inspect-index`. CLI `--lang lwc` = `lwcjs,lwchtml`; for `query`, `lwc` resolves to `lwchtml` when the first non-blank character is `<`, else `lwcjs` (same for `aura`/`aurajs`) |
| D34 [D5→D13] | Metadata feature model | **Revised by D42.** No shared-core change before evidence. Candidate model for the spike: per-language feature config. For `sfmeta`: labels that do not depend on child count, positionless variable-usage features, sibling features off, and children sorted by (tag, `fullName`/`name` text) with a deterministic tie-break for repeated unnamed children. A small formula lexer runs over `<formula>`, `<errorConditionFormula>` and flow `<expression>`. Property tested: "evidence present before an edit stays present" (not "features identical"). Java/Apex/LWC/Aura/VF keep positional features, bit-identical |
| D35 [D6→D15] | Unit granularity | LWC JS: class methods, getters/setters, `@wire` members, top-level functions. LWC HTML: root `<template>` + nested `<template>` (`lwc:if/elseif/else`, `for:each`, `iterator:*`) + top-level children over 200 leaves. Flows: one unit per element (`decisions`, `assignments`, `recordLookups`, `recordCreates`, `recordUpdates`, `recordDeletes`, `loops`, `screens`, `subflows`, `actionCalls`, `formulas`). Field, validation rule, CMDT record: one unit per file. MDAPI `.object`: per `<fields>` / `<validationRules>` child. Aura markup and VF: root component/page + nested `aura:if`/`aura:iteration`/`apex:repeat`/`apex:form` blocks over 200 leaves; Aura JS: each function-valued property of the controller/helper object. **Recursive split (D44):** markup and metadata units over 2,000 leaves are split into child elements until every piece is ≤ 2,000; the parent stays indexed and is marked oversize. New `UnitKind` values per kind; `--kind` filters |
| D36 [D7] | Snippet repair for markup | Repair strategy per language: bracket closer (Java, Apex, LWC JS, Aura JS) or tag-stack closer (HTML, XML, Aura, VF). The tag-stack closer skips comments, CDATA, quoted attribute values, `{expr}` and `{!expr}`; it knows HTML void elements and appends missing `</tag>` innermost first. Wrappers: JS = statements / class member / expression; HTML = `<template>…</template>`; XML = root guessed from the first tag (`Flow`, `CustomField`, `ValidationRule`, `CustomMetadata`, `CustomObject`) else a neutral root; Aura = `<aura:component>`; VF = `<apex:page>`. Trailing-line trim and damage accounting stay shared. Fragment policy: D43 |
| D37 [D8] | BM25 terms for markup | `LangSpec` term rule. Java/Apex/LWC JS/Aura JS keep the camelCase rule (asserted unchanged). HTML/XML/Aura/VF also split on `- . : { } ! $` and keep the whole token as an extra term. XML values, formula field paths and `{!v.x}` / `{!c.method}` expressions use the same rule. Feature ids unaffected |
| D38 [D9] | v1.1 release gate | Corpus: 3 pilot repos + `trailheadapps/lwc-recipes`, `ebikes-lwc`, `dreamhouse-lwc` at pinned SHAs (fetched by `xtask`, untracked under `ref/`). LWC JS contiguous recall R@1 ≥ 0.95 and R@100 ≥ 0.99; LWC HTML R@100 ≥ 0.98; 20-task LWC idiom suite (≥ 6 whole-component tasks, D46) similar hit@5 ≥ 60%; all v1.0 gates unchanged after D40's re-run |
| D39 [D10→D13] | v1.2 release gate | Corpus floor (spike pins public repos via `gh`): ≥ 1,000 flow elements from ≥ 50 flows, ≥ 150 validation rules, ≥ 150 formula fields. Per-kind recall R@100 ≥ 0.95 under D43 sampling. Structural metadata ships only if it beats the BM25-only baseline by ≥ 5 points hit@5 on a hand-judged 15-task metadata suite (D42). Otherwise v1.2 ships BM25-only metadata marked experimental. Floor not reachable: same BM25-only outcome, and the shortfall is recorded |
| D40 [D11] | R@1 scorer | `xtask/src/recall.rs:401` `source_rank` counts a source as rank 0 when its structural score ≥ the top result's, but since T23 the list is in RRF order. Fix: rank = displayed position; ties only between equal fused scores. Regression test: source at position 3 with equal structural score must give rank 2. Re-run Java and Apex recall; correct note 19. If R@1 < 0.95, fixing it becomes v1.0 work ahead of v1.1 |
| D41 [D12] | Unit builder hook | Replace "kind + `body_field`" with `fn unit(node, src) -> Option<UnitShape { kind, body: Node, leading: Vec<Node> }>`. Java/Apex use a default derived from today's tables (520 goldens unchanged). LWC/Aura JS: a member's decorators are `leading` nodes in the unit; `@wire` properties are units whose body is the decorator call. Imports are never consulted (D18 context-free); `lightning/*`, `@salesforce/*`, `c/*` module names and Aura `$A` are built-in globals |
| D42 [D13] | Metadata evidence first | T24 contains no XML feature change. Spike T28: (1) BM25-only metadata path fully specified and built: parse, extract, term generation, lexical admission without the structural eligibility filter, lexical ranking, rendering, malformed-query behaviour; (2) structural prototype per D34; (3) both measured on recall and on the hand-judged 15-task suite. The 15-task suite runs on one pooled index of all v1.2 corpus repos (task source held out), because validation rules and formulas are too sparse per repo for single-repo answerability; the comparison is relative (same tasks, same index, two modes). The LWC suite stays single-repo. The shared-core change lands (T29) only on a "structural wins" verdict |
| D43 [D14] | Recall exam for new kinds | Sampling per unit kind: minimum lines 12 for code, 4 for markup and metadata; ≥ 100 queries per gated kind, else the kind is reported, not gated; slice offsets at start, middle and end. Fragment policy: gated queries are prefix or suffix cuts (repairable by D36); interior slices that start inside an element are reported separately and never gated. Extraction coverage (units found vs elements present) printed per kind |
| D44 [D15] | Oversize units | Recursive split per D35. `inspect-index` reports oversize and split counts per kind. Test: a 5,000-leaf template and a 3,000-leaf flow screen; a snippet from the middle of each is found |
| D45 [D16/D17] | Whole-component LWC search (in v1.1) | Query-time join, no index change. `aroma query --component DIR` (or `--file x.html --file x.js`) runs the HTML part and the JS part as two normal searches (top 100 each). Hits are grouped by component folder (`lwc/<c>/`); a folder's score is the sum of its parts' RRF values (missing part = 0). Output lists components with their matched template and JS regions. Library: `Index::query_component(&ComponentQuery)` |
| D48 | Idiom task validity | Idiom tasks are validated post-holdout by `idiom --check`; < 2 answers in the evaluation population is a task bug (own repository for Apex/LWC; the pooled v1.2 repositories and pilots for sfmeta per D42). The check applies the real run's source-unit and exact-body-clone holdout before counting indexed full units or component folders, flags unavailable declared witnesses, and rejects any required fragment already present in a task or part query (whitespace-insensitive). Validate tasks before measuring the unchanged search gate. |
| D49 | v1.1 ship with recorded shortfall | User decision 2026-11-20: v1.1 ships although LWC JS start R@1 is 0.9489 (seed 119) / 0.9477 (seed 4242) against 0.95; all other D38 gates pass (note 27). The miss is mostly tied near-duplicate members (note 23); no gate, sampling or scorer change was made to close it, and the tie problem is a tracked TODO |
| D50 | Component clone join | component join expands exact-body clones to every folder that holds them. Each part still searches normally (top 100); clones of the same language that pass its kind/test filters receive the admitted clone's displayed-rank RRF. Each folder keeps only its best-ranked region per part and displays its own unit. Normal queries retain clone dedup. |
| D47 (note 20) | Pre-parse rewrite hook | `LangSpec.preprocess: fn(&str) -> Cow<str>`, length-preserving (asserted). `lwchtml` rewrites `_` → `-` inside tag names (`<name`, `</name`) because `tree-sitter-html` rejects `_` (147/1,395 LWC templates failed; 9 after the rewrite). The parser sees the rewritten text; token text, spans and rendering use the original source. XML `AttValue` is an atomic kind (its value text is not a node) |
| D46 [D18] | Aura + Visualforce (in this plan, as v1.3) | **Revised by note 20:** `aura` uses `tree-sitter-xml` (464/464 clean); `vf` uses `tree-sitter-html` (368/371 clean vs 291/371 with XML: inline scripts, bare `&`, entities). Originally: `aura` and `vf` use `tree-sitter-xml` (both are well-formed XML markup) with positional features, `{!expr}` tokenized by the shared expression lexer; `aurajs` uses `tree-sitter-javascript`. v1.3 gate: corpus floor ≥ 300 Aura bundles and ≥ 300 VF pages from pinned public repos (NPSP alone has 35 `.cmp` and 80 `.page`); recall R@100 ≥ 0.98 per language under D43 sampling; Aura component join (markup + controller + helper) reuses D45 |

Consequences, not separate decisions: new `Lang`/`UnitKind` variants and per-language feature config
change the postcard layout and manifest fingerprint, so `INDEX_SCHEMA_VERSION` becomes 3 and existing
indexes must be rebuilt (existing "re-index required" path). v1.0 performance gates are re-run at each
release with every language enabled (NPSP full index ≤ 60 s, query p50 ≤ 100 ms / p95 ≤ 500 ms,
RSS ≤ 1 GiB; component query p50 ≤ 200 ms since it runs two searches).

## 2. What already exists (reused, not rebuilt)

| Need | Existing code | Use |
|---|---|---|
| CST walk, ERROR/MISSING policy, depth/size/time caps | `aroma-lang/src/table.rs` (`parse`, `to_kraw`, `extract_units`) | Reused; `extract_units` calls the D41 unit builder |
| Language rewrites | `normalize/mod.rs` `Normalizer` trait (Java, Apex) | New: `lwc_js.rs`, `lwc_html.rs` (`{expr}`, directives), `sfmeta.rs` (sort, formula lexer), `aura.rs`/`vf.rs` (`{!expr}`) |
| `#VAR` rule, globals | `classify.rs`, `globals.rs` | Per-spec tables; `for:item`/`iterator:*`/`aura:iteration var` names and flow `{!var}` are locals |
| Snippet wrappers, closer, trim | `snippet.rs` (`wrappers`, `closers`, `parse_snippet`) | `closers` becomes the bracket strategy beside the tag-stack strategy |
| BM25 + RRF fusion | `aroma-core/src/lexical.rs`, `store/lexical.rs` | Term rule per language; RRF reused by the component join |
| Rendering | `aroma-core/src/render.rs` (`const OMITTED: &str = "// your code ..."`) | Marker per language: `// your code ...` or `<!-- your code ... -->` |
| Recall / idiom / perf harness | `xtask/src/{recall,idiom,perf}.rs`, `xtask/tasks/apex/*.toml` | Scorer fixed (D40); per-kind sampling (D43); task dirs `xtask/tasks/{lwc,sfmeta,aura,vf}/` |
| Storage, GC, lock, verify | `aroma-core/src/store/*`, `aroma-lang/src/index.rs` | Unchanged apart from schema 3 |

## 3. Architecture

```text
repo walk (ignore crate)
   │  rel path ──▶ LangSpec::matches(path) per enabled lang ──▶ none? skip + count(reason)
   ▼
tree-sitter parse (java | sfapex | javascript | html | xml)   [shared: 2 MiB, 2 s, depth 1000]
   ▼
extract_units: spec.unit(node) -> UnitShape{kind, body, leading}            ← D41
   │           markup/metadata: recursive split while n_leaves > 2000       ← D44
   ▼
to_kraw (shared ERROR policy) ─▶ spec.normalizer.normalize
   │   lwcjs/aurajs: decorators → keywords   lwchtml: {expr} → tokens   aura/vf: {!expr} → tokens
   │   sfmeta: sort children, formula lexer (structural model only on the D42 "wins" verdict)
   ▼
classify (#VAR per spec) ─▶ Spt ─▶ featurize(spec.feature_config)
   ▼
postings.bin (ids namespaced by lang) + lexical.bin (spec.term_rule terms)  ← D37

query:  --lang (lwc/aura auto-resolve) ─▶ snippet repair (bracket | tag-stack) ─▶ wrappers
        ─▶ Phase I + BM25 ─▶ RRF ─▶ prune/rerank ─▶ render (spec marker)
component query (D45):
        DIR ─▶ [html part] search ─┐
              [js part]   search ─┴▶ group hits by lwc/<c>/ ─▶ Σ RRF per folder ─▶ components[]
```

## 4. Implementation tasks (sequenced; lanes in §8)

- [ ] **T23b (P1, blocks all)**: D40 scorer fix + regression test; re-run Java/Apex recall; correct
  note 19 (and TODOS if R@1 < 0.95).
- [ ] **T24 (P1)**: hooks with no behaviour change: all six `Lang` variants (no specs yet, so
  `matches` returns false), `LangSpec.matches`, D41 unit builder, per-language `feature_config`,
  `term_rule`, `omission_marker`, `repair` strategy. Verify: 520/520 goldens, Java/Apex recall and
  Apex idiom identical to the T23b re-run, schema 3.
- [x] **T25 (P1)**: `lwcjs` spec + normalizer + path rule + JS wrappers + globals. 30+ construct
  fixtures (class methods, getters/setters, `@wire` function and property, `@api`, `@track`,
  async/await, arrows, template literals, optional chaining, imports).
- [x] **T26 (P1)**: `lwchtml` spec + normalizer (`{expr}`) + nested template units + recursive split +
  tag-stack closer + HTML term rule + `<!-- -->` marker. 30+ fixtures (`lwc:if/elseif/else`,
  `for:each`, `iterator`, slots, handlers, `lwc:ref`, `lwc:spread`, void elements).
- [x] **T27 (P1)**: D45 component join (`--component`, `--file ×2`, `Index::query_component`, JSON
  schema), CLI tests.
- [x] **T27b (P1)**: v1.1 corpus + gates: `xtask fetch` pins, D43 sampling, 20 LWC idiom tasks
  (≥ 6 component tasks), note 20. **Ship v1.1.** Shipped 2026-11-20 with LWC JS R@1 short (0.9489 / 0.9477 vs 0.95), recorded in note 27 (D49).
- [x] **T28 (P1)**: metadata spike per D42: `tree-sitter-xml` kinds and partial-XML recovery; D39
  corpus pins; BM25-only path built and measured; structural prototype measured; 15 hand-judged
  tasks; note 21 with the verdict.
- [x] **T29 (P1, structural wins)**: `sfmeta` structural: per-language feature config in aroma-core
  (D34), XML normalizer, unit builder for flow elements / MDAPI children, root wrappers. 30+
  fixtures (each flow element type, field types incl. formula and picklist, validation rule, CMDT,
  MDAPI `.object`, CDATA, namespaces, attributes, comments). Shipped: [record 26](results/26-sfmeta-structural.md).
- [x] ~~**T29b (P1, BM25 wins or floor missed)**~~ not needed (D42 verdict: structural wins): harden the spike's BM25-only path, mark results
  `experimental_lexical_only`.
- [x] **T30 (P1)**: v1.2 gates (per-kind recall, 15-task suite, perf), note 22. **Ship v1.2.** Shipped 2026-11-20 (note 29).
- [x] **T31 (P1)**: `aura` + `aurajs` + `vf` specs, normalizers (`{!expr}` lexer shared with T29's
  formula/merge-field lexer), unit builders, Aura bundle join via D45. 30+ fixtures each.
- [x] **T32 (P1)**: v1.3 corpus (≥ 300 Aura bundles, ≥ 300 VF pages) + gates, note 23. **Ship v1.3.** Shipped 2026-11-20 (note 29; VF floor met after the note 21 addendum).

## 5. Test plan and coverage

Mandatory regardless of decisions:
- **Regression.** After T24, every v1.0 gate must reproduce the T23b re-run exactly. Goldens are
  bit-exact; recall and idiom must not drop.
- **D18 invariant per new language.** For 200 indexed units, featurizing through the query path must
  equal the index features.

```text
CODE PATHS                                                   USER FLOWS
[+] recall scorer (T23b)                                     [+] aroma index (mixed SFDX repo)
  └── [PLAN ★★★] displayed rank; equal-score source at 3 → 2   ├── [PLAN ★★★] NPSP: lwc/aura/vf/allowlisted xml only;
[+] spec hooks (T24)                                           │              skip counts per reason
  ├── matches(path)                                            ├── [PLAN ★★★] --lang lwc / aura expand to both parts
  │   ├── [PLAN ★★★] lwc/<c>/x.js, __tests__ → test,           ├── [PLAN ★★★] schema-2 index → "re-index required"
  │   │              staticresources/*.js → skip               ├── [PLAN ★★★] oversize unit split; counts in inspect-index
  │   ├── [PLAN ★★★] aura/<b>/*Controller.js → aurajs          └── [PLAN ★★ ] incremental: edit one .html → only it reparsed
  │   └── [PLAN ★★★] allowlisted suffixes; .cls-meta.xml,    [+] aroma query
  │                  fieldTranslation, profile → skip          ├── [PLAN ★★★] lwc/aura auto-resolve (`<` → markup)
  ├── unit builder (D41)                                       ├── [PLAN ★★★] half-pasted template → tag closer → hit
  │   ├── [PLAN ★★★] Java/Apex default == v1.0 units           ├── [PLAN ★★★] prefix/suffix-cut flow element → hit
  │   ├── [PLAN ★★★] decorators in unit; @wire property unit   ├── [PLAN ★★ ] interior slice → reported, low_confidence ok
  │   └── [PLAN ★★★] recursive split ≤ 2000 leaves             ├── [PLAN ★★★] only `{expr}` / only text → no-evidence status
  ├── feature_config                                           ├── [PLAN ★★★] marker <!-- your code ... --> in render
  │   ├── [PLAN ★★★] positional == v1.0 bags (520 goldens)     └── [PLAN ★★★] sfmeta BM25-only flag (T29b)
  │   └── [PLAN ★★★] sfmeta: prior evidence survives edits   [+] component query (D45)
  └── term_rule                                                ├── [PLAN ★★★] both parts hit same folder → ranked first
      ├── [PLAN ★★★] camelCase rule unchanged                  ├── [PLAN ★★★] only one part matches → still listed, lower
      └── [PLAN ★★★] lightning-datatable, Account.Owner.Name,  ├── [PLAN ★★★] folder without .html (service component)
                     {!$Record.Status__c}, {!v.items}          └── [PLAN ★★★] --file with 1 or 3 files → clear usage error
[+] normalizers                                              [+] errors the user sees
  ├── lwc_js / aura_js: [PLAN ★★★] decorators, globals         ├── [PLAN ★★★] malformed XML/HTML → damage counted, >10% skipped
  ├── lwc_html: [PLAN ★★★] {expr} in attr + text, directives   ├── [PLAN ★★★] 2 MiB flow → skipped + counted, no crash
  ├── aura/vf: [PLAN ★★★] {!expr} lexer, namespaced tags       └── [PLAN ★★★] DOCTYPE / entities → text, never expanded
  └── sfmeta: [PLAN ★★★] sort + tie-break, formula lexer (prop: random formulas never panic)
[+] snippet repair
  ├── bracket: [PLAN ★★★] v1.0 tests unchanged
  └── tag-stack: [PLAN ★★★] nested open tags, void elements, self-closing, comment/CDATA/quoted
                 attr/{expr}/{!expr} containing `>` skipped, mismatched close → no repair

Harness: [PLAN ★★★] D43 per-kind sampling, offsets start/middle/end, ≥100 queries or "reported"
E2E: aroma-cli/tests/cli.rs index + query + component query per new language.
EVAL-like: recall + idiom suites (xtask), BM25 baseline vs structural (T28).
```

## 6. Failure modes

| New path | Realistic failure | Test | Handling | User sees |
|---|---|---|---|---|
| Path routing | LWC under `src/lwc/`, not `force-app/main/default` | non-default layout fixture | rule keys on `…/lwc/<c>/` | indexed normally |
| tree-sitter-xml | Slow parse of a 1.9 MiB flow | timeout test | shared 2 s parse timeout | skipped + counted in `inspect-index` |
| Tag-stack closer | `>` inside an attribute value or `{expr}` | dedicated test | strategy skips quoted values and braces | correct repair, else `low_confidence` |
| Formula / `{!expr}` lexer | Unknown function or operator | property test over random strings | unknown → token, never panic | features still produced |
| `lwc` auto-resolve | JS snippet starting with `<` | test | `--lang lwcjs` overrides; chosen language reported | `diagnostics.lang` in JSON |
| Component join | Folder holds only JS (service component) | test | missing part scores 0 | component listed with JS region only |
| Recursive split | Deep markup splits into hundreds of tiny units | test with 5,000-leaf template | split stops at element granularity; counts reported | oversize/split counts in `inspect-index` |
| Schema 3 | Query against a schema-2 index | CLI test | manifest mismatch | "re-index required" error |
| R@1 scorer | v1.0 R@1 overstated | regression test | D40 re-run, note 19 corrected | honest release record |

No critical gaps: every row has a test and visible handling.

## 7. NOT in scope

- Profiles, permission sets, layouts, flexipages, list views, translations: configuration, not logic.
- Cross-language structural units (one tree spanning HTML + JS). Component search is a query-time
  join (D45), not a composite unit.
- Links between flows and the Apex actions they call, or between templates and Apex methods.
- LSP / HTTP surface: unchanged from U2.
- CI on a real remote and the project license: still open from v1.0 (TODOS P1). These plans add
  three C grammar builds, so CI should run before tagging v1.1.

## 8. Worktree parallelization

| Step | Modules touched | Depends on |
|---|---|---|
| T23b scorer | xtask | — |
| T24 hooks | aroma-core (lang, unit, features, render, lexical), aroma-lang (spec, table, snippet, build) | T23b (baseline numbers) |
| T25 lwcjs | aroma-lang (spec, normalize) | T24 |
| T26 lwchtml | aroma-lang (spec, normalize, snippet) | T24 |
| T27 component join | aroma-lang (index), aroma-cli | T25, T26 |
| T27b v1.1 gates | xtask | T27 |
| T28 spike | scratch, xtask (corpus pins, BM25-only path prototype) | T24 |
| T29 / T29b sfmeta | aroma-core (features, only if T29), aroma-lang | T28, T26 (tag-stack) |
| T30 v1.2 gates | xtask | T29 or T29b |
| T31 aura/vf | aroma-lang (spec, normalize) | T26, T27, T29/T29b (shared `{!expr}` lexer) |
| T32 v1.3 gates | xtask | T31 |

Lanes:
- A = T23b → T24 (sequential, everything depends on them).
- Then B = T25 ∥ C = T26 ∥ D = T28 in parallel worktrees. B and C both touch `aroma-lang/src/spec/`
  and `normalize/`, but in separate new files, and T24 already added the `Lang` variants and match
  arms, so conflicts are limited to `spec/mod.rs` registration lines.
- Then T27 → T27b (v1.1).
- Then T29/T29b → T30 (v1.2).
- Then T31 → T32 (v1.3).
- `xtask` is touched by T23b, T27b, T28, T30 and T32: keep those sequential.
