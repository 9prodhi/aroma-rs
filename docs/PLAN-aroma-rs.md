# PLAN: aroma-rs — Aroma structural code recommendation in Rust + tree-sitter

Status: DRAFT for `/plan-eng-review` · Date: 2026-11-19 · Owner: (you)

## 0. One-paragraph summary

Build a Rust library + CLI that indexes a single repository (< 200k methods/units) and, given a
partial code snippet, returns **structurally similar code with the matching region highlighted**
(Aroma Phase I + prune/rerank). Aroma's clustered *recommended extensions* ship behind
`--experimental-recommend` until they beat the similar-code baseline (pilot D26/D27, note 17),
following Aroma (Luan et al., OOPSLA 2019,
arXiv 1812.01158). v1 languages: **Java, Salesforce Apex (+ inline SOQL/SOSL), LWC (JavaScript
modules + HTML templates), Salesforce XML metadata**. The algorithm core is language-agnostic; each
language contributes a tree-sitter (or XML) adapter that produces Aroma's *simplified parse tree*.

## 1. Decisions already made (by you, 2026-11-19)

| # | Decision | Answer |
|---|---|---|
| U1 | v1 languages | Java, Apex (+SOQL/SOSL), LWC (JS + HTML), Salesforce metadata (mostly XML) |
| U2 | Product surface | Library crate + CLI (`index`, `query`, JSON output). LSP/HTTP later |
| U3 | Scale | Single repo, < 200k units; in-memory index persisted to disk |
| U5 | Product after pilot (D26, D27) | v1.0 = similar-code search; clustered recommendations experimental (`--experimental-recommend`), promoted to default only when they beat the top-5-similar baseline in ≥ 60% of shown cases |
| U4 | Release staging (D5) | v1.0 = core + Java + Apex/SOQL/SOSL; v1.1 = LWC JS + HTML; v1.2 = Salesforce metadata. Each stage ships only after its own self-retrieval benchmark passes |
| U6 | v1.1–v1.3 plan (2026-11-20) | Reviewed in `docs/PLAN-aroma-rs-v1.1-v1.2.md` (D31–D46): all new languages are tree-sitter LangSpec tables; v1.1 LWC + component join, v1.2 metadata (evidence first), v1.3 Aura + Visualforce |

## 2. Source material

The numbered research notes cited by this historical plan are internal research
notes, not published, except for the release records preserved in
[results](results/README.md). The decision tables below preserve the design choices.
`concepts/aroma-concepts/` contains the early reference-pipeline prototype.

Key facts that shape the plan:

1. **The paper and the shipped reference code disagree** (notes 04 §5, 08 §1). Reference: Phase I
   cap 100 + `overlap > 0.4·|q|`; prune maximises weighted **Jaccard** (paper: intersection size);
   features of the pruned tree are **re-extracted**; `s(τ)` denominator is the **pruned** first
   member; clusters grow by max **unpruned intersection** (paper: max `l`); **no singleton**
   clusters; intersect = sequential re-prune. Shipped `example_features.txt` was produced by a
   *different* config (N_SIBLINGS=2, no var-usage, broader wrapper skip) — reproduced exactly by
   note 04's scratch re-implementation.
2. **No Rust (or any non-Python) Aroma implementation exists** (notes 05, 11–13; `gh` spot-checked).
   No crate is worth depending on for the core; borrow ideas only.
3. **Apex grammar:** `tree-sitter-sfapex` 3.0.2 (aheber, active) gives apex/soql/sosl; inline SOQL is a
   `query_expression` subtree in the Apex tree. Keywords case-insensitive in the lexer; identifiers
   keep source case → we must case-fold.
4. **LWC templates:** `tree-sitter-html` treats `lwc:if`, `for:each` and `{expr}` as plain attributes/text.
5. **Toolchain:** local rustc is 1.89; `tree-sitter-language` 0.1.9 needs 1.90; tree-sitter 0.27 needs
   newer still. Must pin a toolchain.

## 3. Architecture

### 3.1 Workspace layout (3 crates + 1 dev tool)

```text
aroma-rs/
├── rust-toolchain.toml            # pinned stable (>= 1.90)
├── crates/
│   ├── aroma-core/                # NO tree-sitter dependency. Pure algorithm + index.
│   │   ├── spt.rs                 # arena SPT, collapse rules A/B/C, labels
│   │   ├── features.rs            # featurizer (FeatureConfig), typed FeatureKey, interner
│   │   ├── multiset.rs            # sorted run-length Bag, one visitor merge, Jaccard
│   │   ├── index.rs               # inverted index, touched-list accumulator, top-k
│   │   ├── prune.rs               # greedy selection (Jaccard | Intersection; lazy only for the latter)
│   │   ├── cluster.rs             # tuple growth, dedup, intersect
│   │   ├── render.rs              # line-granular render + kept byte spans
│   │   ├── profile.rs             # Profile::{Reference, Paper} = FeatureConfig + Params
│   │   ├── store.rs               # persisted index generation (postcard), manifest, content hashes
│   │   └── explain.rs             # matched/unmatched features, prune trace
│   ├── aroma-lang/                # adapters behind cargo features: java, apex, lwc, sfmeta
│   │   ├── table.rs               # TableAdapter: ONE generic CST→Raw walker + unit extraction + ERROR policy
│   │   ├── spec/                  # LangSpec const tables (java.rs, apex.rs, lwc_js.rs), validated vs node-types.json
│   │   ├── scope.rs               # scope pass → SymbolId per declared local (shared by all table languages)
│   │   ├── normalize/             # Normalizer trait: java.rs (virtual-ANTLR regrouping), apex.rs (SOQL sort, DML, binds)
│   │   ├── lwc_html.rs  sfmeta.rs # custom adapters (non-table) for v1.1 / v1.2
│   │   └── snippet.rs             # partial-query wrapping strategies, ERROR/MISSING policy
│   └── aroma-cli/                 # clap: index, query, dump-spt, dump-features, feature-diff, inspect-index
└── xtask/ (aroma-eval)            # recall benchmarks, corpus fetch at pinned SHAs, perf gates
```

Boundary rule: `aroma-core` never sees tree-sitter. Adapters hand it a `Raw` tree (concept crate
`spt::Raw`). This keeps the algorithm testable on hand-built trees and lets the XML adapter use a
non-tree-sitter parser.

### 3.2 Index data flow

```text
repo walk (ignore crate, .gitignore aware)
   │  per file: blake3(content) ── unchanged vs manifest? ──yes──▶ reuse cached FileEntry
   ▼ no
language by extension ─▶ adapter (rayon, thread-local Parser, parse timeout)
   │  CST ─▶ units (method / trigger / LWC handler / template block / metadata element)
   │  unit ─▶ Raw ─▶ Spt::build (collapse A/B/C) ─▶ featurize(profile) ─▶ FeatureIds
   ▼
FileEntry { path, hash, units: [ UnitEntry { kind, lang, span, lines, distinct feature ids } ] }
   ▼
IndexGeneration { manifest, units.postcard, postings.bin (CSR, mmap) } ── atomic rename ──▶ .aroma/gen-<n>/
```

### 3.3 Query data flow

```text
snippet + --lang ─▶ snippet.rs: try wrappers (method body → class member → expression)
   │                 measure ERROR/MISSING damage; strip synthetic wrapper tokens
   ▼
query Spt ─▶ F(q) ─▶ Phase I: postings, touched-list accumulate, partition = same language (D22; --kind filter optional)
   │                  keep overlap > 0.4·|S(F(q)) ∩ indexed features|, top 100 (paper: 1000)
   ▼
candidates: load stored compact SPT blobs from spt.bin (D19; no parsing on the query path)
   ▼
Phase II (rayon): prune(F(q)) → re-featurize → Jaccard → keep > 0.65 → stable sort
   ▼
Phase III: cluster(τ3 0.9, τ2 1.5) → dedup (index-set Jaccard > 0.5) → top 5 → intersect
   ▼
render: full lines + "// your code ..." + kept byte spans ─▶ JSON { similar[], recommendations[], diagnostics }
```

### 3.4 Algorithm profiles

`Profile::Reference` is the default (it is the only executable ground truth and its thresholds
were tuned together). `Profile::Paper` is selectable for evaluation. Never mix knobs silently: the
profile name and its full parameter set are written into the index manifest and every JSON result.

| Knob | Reference (default) | Paper |
|---|---|---|
| Phase I | cap 100, overlap > 0.4·known | cap 1000, no ratio |
| Prune objective | weighted Jaccard, strict >, earliest leaf | intersection size, lazy greedy |
| Rerank keep | Jaccard > 0.65 | SimScore/|F(q)| > 0.65 (normalisation assumed) |
| s(τ) denominator | first **pruned** member | first pruned member (printed formula is a typo) |
| Cluster growth | max unpruned intersection | max l(τ) |
| Singletons | never | allowed |
| Intersect | sequential prune [q, member] | Prune(F(member) ⊎ F(q), prev) |

### 3.5 Feature representation

**Decided (D7): stable 64-bit hashes, no persisted vocabulary.** Typed
`FeatureKey { lang_ns, kind: Token|Parent|Sibling|VarUse, text }` (no unit kind: D22) → canonical string →
`FeatureId(u64) = xxh3_64_with_seed(canonical, SEED)` (seed and canonical format are part of the
feature-schema version in the manifest). Indexing threads need no coordination; incremental update
is add/remove postings. Feature strings exist only at query time (query + stored candidate SPTs are
re-featurized anyway), which is where `--explain` reads them. Collision odds at ~5M distinct
features ≈ 7·10⁻⁷ per index. Query-only unknown features count in `|F(q)|` for Jaccard but never
match postings. Namespacing by language means Apex and Java never share feature ids.
In-memory multiset math may still use a per-query dense remap (u64 → u32) for speed.

### 3.6 Storage and incremental indexing

- `.aroma/gen-<n>/` holds one immutable generation: `manifest.json` (schema version, profile,
  feature-hash seed, adapter and grammar fingerprints, toolchain), `units.postcard` (file entries:
  path, blake3, units with kind/lang/span/lines and their sorted distinct `FeatureId`s), and
  **`postings.bin` (Decided D15): a flat CSR file** = header (magic, version, endianness, counts) +
  sorted `[u64]` feature hashes + `[u32]` offsets + `[u32]` unit ids, 8-byte aligned, loaded with
  `memmap2` + `bytemuck` (no deserialisation; lookups binary-search the feature array). Load ≈ 1–5 ms
  warm, so one-shot CLI queries meet the p50 target. Each index run rewrites `postings.bin` from the
  unit table (≈ 12M entries, < 1 s); header and bounds are validated before use (fuzzed).
- **Decided (D19, supersedes D8): compact SPTs stored in the index.** `spt.bin` = offset table +
  one postcard blob per unit (labels, leaf tokens, token roles, `SymbolId`s, byte spans; no
  whitespace/trivia), ≈ 0.3–1 KB per unit (60–200 MB at 200k). A query mmaps the file and decodes
  only its ≤ 100 candidate blobs. Rendering reads original source lines when the file's blake3 still
  matches; otherwise it prints tokens from the stored tree and flags the result `stale_source`.
  Candidates from edited files therefore still participate.
- Incremental: content-hash per file (not git diff — works on dirty trees, branch switches, no
  libgit2). Changed files re-parsed; postings.bin rewritten from the unit table; generation published by atomic rename.
- Manifest mismatch (grammar, adapter, schema, profile) → refuse to query, ask for re-index.
- **Decided (D10): exclusive writer lock** on `.aroma/lock` (OS advisory lock via `fd-lock`/`fs4`,
  released on process exit). A second writer waits up to `--lock-timeout` (default 30 s) then fails
  with a clear message naming the holder PID. Readers never lock; they load the newest complete
  generation and report its id in every result.

### 3.7 Contract hardening (D22–D24)

- **D22 contracts.** (a) Unit kind is metadata, not a feature namespace; queries search every unit of
  the language, `--kind` filters optionally. (b) `Raw::Leaf` gains `binding: Option<SymbolId>` and
  owns its byte span; the `Raw`/`Spt` contract is frozen only after this lands (first M1 commit).
  (c) The manifest stores the full effective feature config (profile, compat flag, case-fold,
  string bucketing, hash seed, adapter + normalizer versions, grammar fingerprints); any mismatch
  refuses queries with "re-index required". (d) Each unit gets a blake3 of its normalized token
  stream; exact clones collapse to one representative before the Phase I cap, and recommendations
  report *independent support* = distinct non-clone members.
- **D23 query path.** (a) One `Deadline` + cancellation token flows through Phase I, prune, cluster,
  intersect and render; when it fires, finished stages are returned, unfinished candidates are
  dropped, and `budget_exhausted` names the stage; a test asserts the wall clock is enforced.
  (b) Ineligible units (oversized, clones of a better-ranked unit) are filtered before the top-100
  cap and the list backfills from lower ranks. (c) Queries mmap only a compact unit table (file id,
  span, lines, kind, `spt.bin` offset); per-file feature inventories are build-only and never
  decoded by `query`; CI benchmarks real one-shot `aroma query` process time. (d) Generation GC:
  after publishing gen n, delete gens < n-1; on Windows defer deletion of still-mapped files to the
  next run.
- **D24 process.** (a) M3's end-to-end CLI tests depend on M2's Java adapter (lanes updated in §13).
  (b) M0 pins `rust-toolchain.toml` (≥ 1.90), checks for a C compiler (grammar build scripts use
  `cc`) in CI and docs, and documents `cargo vendor` for offline builds. (c) Differential goldens
  compare final outputs (features, prune masks, rerank scores, clusters, text), never intermediate
  greedy steps.

## 4. Language adapters

All adapters: kind tables are `const` arrays, unit-tested against the grammar's `node-types.json`;
comments are never children; depth cap 1000 (reference `MAX_DEPTH`) with explicit stacks;
literals are one token; strings > 128 bytes bucketed to `<str:len>` (versioned deviation).

**Decided (D11): malformed-input policy.** ERROR subtrees → one opaque `#ERR` keyword (no features,
child slot preserved); MISSING (zero-width) nodes dropped; corpus units with > 10% damaged bytes
skipped and counted; queries report `parse_damage` and set `low_confidence` above 10%; every tree
walk uses an explicit stack with depth cap 1000 (reference `MAX_DEPTH`); per-file 2 MB size cap and
2 s parse timeout. All counters appear in `inspect-index` and query diagnostics.

**Decided (D21, restates D12): three-stage adapter.** (1) shared `TableAdapter` walk driven by
`LangSpec` tables: CST→Raw, unit extraction, ERROR policy, depth cap; (2) per-language `Normalizer`
trait for real tree rewrites: Java virtual-ANTLR regrouping (method calls, `>>` splitting, for-control,
local declarations, types), Apex SOQL field sort, DML labels, binds; (3) shared D18 classification
and collapse. Each normalizer has its own fixtures; Java `--compat` goldens test stages 1–3 end to end.

*Original D12 text:* **one table-driven adapter.** `TableAdapter` owns the CST→Raw walk, unit extraction,
scope pass and ERROR policy once; Java, Apex and (v1.1) LWC JS are `LangSpec` const tables (leaf
kinds, atomic kinds, declaration kinds, unit kinds, body field, case-fold) plus small hooks. A unit
test per spec asserts every listed kind exists in the pinned grammar's `node-types.json`. LWC HTML
and Salesforce XML stay custom adapters.

| Adapter | Parser | Unit | Local-variable rule | Specials |
|---|---|---|---|---|
| Java | tree-sitter-java 0.23.5 | method body in named class | D18 context-free rule (compat: reference marking + uppercase exemption) | "virtual ANTLR" regrouping for `local_variable_declaration`, `method_invocation`, `for_statement`, generic/scoped types, `>>` splitting (note 04 §6.2) |
| Apex | tree-sitter-sfapex 3.0.2 (`apex`) | method, constructor, trigger body, initializer, accessor with body, anonymous `.apex` | D18 context-free rule after case-fold (no uppercase heuristic) | ASCII case-fold identifiers + keywords (not strings); inline SOQL subtree with SOQL keywords as keywords, fields as tokens, `:bind` var joins usage chains; SELECT field list sorted |
| LWC JS | tree-sitter-javascript 0.25 | class methods, getters/setters, wired functions, top-level functions | scope pass; imports from `lightning/*`, `@salesforce/*` are globals | decorators (`@api`, `@wire`, `@track`) kept as keywords in labels |
| LWC HTML | tree-sitter-html 0.23.2 + small `{expr}` tokenizer | template, or each top-level block / `lwc:if` / `for:each` sub-template when > 200 leaves | `for:item` / `iterator:*` names | tag + directive names are keywords; attribute names keywords; bound members tokens |
| SF metadata | `roxmltree` (not tree-sitter) | Flow elements; CustomObject fields and validation rules; custom metadata records | flow variables / `{!var}` references | element names keywords, values tokens; children sorted by (tag, name) and sibling features disabled (order-insensitive); formulas tokenized lexically; profiles, permission sets, layouts out of v1 |

**Decided (D18, supersedes D9's default): one context-free `#VAR` rule for corpus and queries.**
An identifier is `#VAR` iff it is in expression, receiver or SOQL-bind position, is not a callee,
type, `this.`/class-qualified field, label or keyword, and is not a known global (Apex: `System`,
`Database`, `Schema`, `Trigger`, `Test`, … namespaces and sObject / `__c|__r|__mdt|__e` names in type
position; Java: imported or static-qualified names). The rule never looks at declarations outside
the unit, so a body indexed as a unit and the same body pasted as a snippet normalize identically.
Declarations inside the unit only attach a `SymbolId`, so shadowed names get separate usage chains
(compat mode keys chains on raw spelling, as the reference does). Apex: case-fold first, then
classify. `--compat` keeps the reference's syntactic marking + upper-case exemption for goldens.
**Invariant test:** for 200 indexed units, featurizing the unit body through the *query* path
(snippet wrapping) equals its *index* path features exactly.

## 5. CLI and library API

```text
aroma index [PATH] [--lang java,apex,lwc,sfmeta] [--profile reference|paper] [--jobs N]
aroma query (--file F | --stdin) --lang L [--json] [--explain] [--similar 10] [--experimental-recommend [--top 5]]
aroma dump-spt FILE --lang L          aroma dump-features FILE --lang L [--with-origins]
aroma feature-diff A B --lang L       aroma inspect-index [--json]
```

Library: `Index::open(path)`, `Index::build(root, opts)`, `index.query(&Query) -> QueryResult`.
`QueryResult { profile, similar: [{unit, path, span, score}], recommendations: [{members, text,
kept_spans, support}], diagnostics: { parse_damage, unknown_features, stale_units, budget_exhausted } }`.
JSON on stdout, logs (tracing) on stderr, versioned schema, `thiserror` errors.

## 6. Performance targets (8-core / 16 GB, 200k units)

| Metric | Target |
|---|---|
| Full index | ≤ 60 s |
| Incremental (10 changed files) | ≤ 2 s |
| Index load | ≤ 1 s |
| Query p50 / p95 / p99 (warm) | ≤ 100 ms / ≤ 500 ms / ≤ 1 s |
| RSS | ≤ 1 GiB |
| Per-candidate prune budget | 25 ms; query budget 500 ms → partial result flagged |
| Unit size cap | 2000 leaves → indexed for Phase I, excluded from prune (reported) |

## 7. Test strategy

- **Golden (insta):** `example_query.java` → SPT matches `example_query.json` (Java compat adapter);
  featurizer on `example_query.json` → `example_features.txt` (legacy config) and the 67-feature
  shipped-default bag (note 04 §4b).
- **Property (proptest), ≥ 22 invariants** from note 07 §3: token order preserved, no unary
  wrappers, alpha-renaming invariance, Apex case invariance, binary overlap ignores duplicates,
  multiset min/max laws, prune output ⊆ input + ancestors, SimScore(q, prune(q,m)) ≤ |F(q)|,
  strict-improvement termination, parallel = serial ordering, persist/load = rebuild, empty
  features → explicit no-evidence status, etc.
- **Decided (D13): differential goldens from the Python reference, generated in a virtual
  environment.** `xtask gen-goldens` creates a Python venv with pinned `numpy`/`scikit-learn`
  versions, copies `similar.py` at the pinned artifacts commit into a scratch dir (never runs it in
  place), and runs it with `python -I` on ~500 generated SPT-JSON inputs (hand-written + mutated
  Java snippets + `example_query.json`). Outputs (features, Phase I hits, prune masks, rerank scores,
  clusters, rendered text) are checked in under `tests/golden/reference/`; CI compares the Rust
  `--compat` reference profile against them and never runs Python. A venv isolates dependencies,
  not execution: the code was read in full beforehand (note 04) and only runs on our own inputs.
- **Recall harness (xtask):** paper §5 protocol (methods ≥ 12 lines; first-5-line and random-5-line
  queries; Recall@1/@100; 30 bootstrap resamples). Java: subset of `datasets/github_repositories.csv`
  at pinned SHAs (~200k methods). Salesforce: apex-recipes, lwc-recipes, dreamhouse, ebikes at
  pinned SHAs. Targets: Java Recall@1 ≥ 95%, Recall@100 ≥ 99%.
- **Decided (D14): curated Salesforce idiom suite.** ~40 query → expected-extension tasks drawn from
  public repos at pinned commits (apex-recipes, lwc-recipes, dreamhouse, ebikes, nebula-logger,
  fflib): bulk DML, SOQL with `WITH USER_MODE`/`SECURITY_ENFORCED`, trigger context guards,
  `Database.SaveResult` handling, `@TestSetup` patterns, etc. **Tightened by D17:** each task's
  source method and its normalized-body clones are excluded from the index while it runs; each task
  lists *required added elements* (e.g. `WITH USER_MODE`, `isSuccess()` check) and a hit needs a
  top-5 recommendation containing all of them that the query lacks (scored on recommendation minus
  query); every run also scores the "top-5 similar methods" baseline and v1.0 must beat it.
  **v1.0 release gate (after D27, D29): similar-code hit@5 ≥ 60%** = a top-5 *similar* method whose
  full source lines contain all required elements (pruned-region hit@5 is reported, not gated); the same suite reports recommendation win-rate vs baseline,
  and `--experimental-recommend` is promoted to default only at ≥ 60% wins. Plus one blind manual rating of 20 random queries per release. Tasks live in
  `xtask/tasks/apex/*.toml` and double as Apex adapter regression fixtures.
- **Perf gates (criterion + end-to-end):** the table in §6; fail CI on > 10% regression.
- **Adapter coverage:** 30–50 construct snippets per language (for/foreach, try/catch, switch,
  lambdas, generics, SOQL with binds, DML, triggers, LWC directives, flow elements).

### 7.1 Coverage diagram (planned tests per code path and user flow)

```text
CODE PATHS                                                   USER FLOWS
[+] aroma-core                                               [+] aroma index (first run)
  ├── spt::collapse A/B/C                                      ├── [★★★ PLAN] full build → generation published (e2e cli test)
  │   ├── [★★★ PLAN] golden example_query.json (--compat)      ├── [★★★ PLAN] file > 2 MB / parse timeout → skipped + counted
  │   └── [★★★ PLAN] prop: no unary wrappers, token order      └── [★★★ PLAN] >10% damaged unit → skipped + counted
  ├── features (profiles, compat/scope)                      [+] aroma index (incremental)
  │   ├── [★★★ PLAN] golden example_features.txt (legacy cfg)  ├── [★★★ PLAN] edit / add / delete / rename file
  │   ├── [★★★ PLAN] golden 67-feature shipped-default bag     ├── [★★★ PLAN] incremental == clean rebuild (prop)
  │   ├── [★★★ PLAN] prop: alpha-rename, Apex case invariance  ├── [★★★ PLAN] concurrent writer → waits, then lock-timeout error
  │   └── [★★★ PLAN] #.# context: rhs/out-of-range → no panic  └── [★★★ PLAN] kill -9 mid-write → old generation intact
  ├── feature hash (xxh3, seed)                              [+] aroma query
  │   └── [★★  PLAN] stability snapshot across runs/platforms  ├── [★★★ PLAN] exact method → Recall@1 (self-retrieval)
  ├── multiset (visitor merge, jaccard)                        ├── [★★★ PLAN] partial snippet (statements only) → wrapped
  │   └── [★★★ PLAN] prop: min/max/sum laws; empty/empty = 0   ├── [★★★ PLAN] heavily damaged snippet → low_confidence flag
  ├── index::search (+eligibility/backfill D23b)               ├── [★★★ PLAN] empty / comment-only snippet → no-evidence status
  │   └── [★★★ PLAN] 0.4 boundary, cap, ties, dups, backfill   ├── [★★★ PLAN] edited candidate file → rendered from stored SPT, stale_source
  ├── prune (Jaccard | Intersection, lazy only for latter)     ├── [★★★ PLAN] index missing / manifest mismatch → clear error
  │   ├── [★★★ PLAN] self-prune keeps all; ⊆ input+ancestors   ├── [★★  PLAN] --explain output schema snapshot
  │   ├── [★★★ PLAN] tie → earliest leaf; strict termination   └── [★★★ PLAN] budget exhausted → partial flag, still valid JSON
  │   ├── [★★★ PLAN] lazy == eager for Intersection (prop)
  │   └── [★★★ PLAN] 2000-leaf cap honoured; 25 ms budget
  ├── cluster / intersect
  │   ├── [★★★ PLAN] strict 0.9/1.5 boundaries; no singletons; growth tie → lowest index
  │   ├── [★★★ PLAN] dedup vs dropped tuples; cap 5
  │   └── [★★★ PLAN] intersect carved from first member only
  ├── render (full lines, placeholders, spans)
  │   └── [★★  PLAN] snapshot tests on 5 hand-made clusters
  ├── store (manifest, postcard, atomic rename, lock, spt.bin, unit table, GC)
  │   ├── [★★★ PLAN] corrupt / truncated / future-schema manifest → refuse with message
  │   ├── [★★★ PLAN] config-fingerprint mismatch (e.g. --compat index, default query) → refuse (D22c)
  │   ├── [★★★ PLAN] spt.bin round-trip == freshly built SPT (prop); bad offset → error, no UB (fuzz)
  │   └── [★★★ PLAN] GC keeps gens n, n-1; deferred delete while mapped (D23d)
  ├── dedup + support (D22d)
  │   └── [★★★ PLAN] exact clones collapse before cap; copies never raise independent support
  └── deadline (D23a)
      └── [★★★ PLAN] each stage honours cancel; wall clock ≤ budget + 10 ms; partial JSON valid
[+] aroma-lang
  ├── TableAdapter + LangSpec (Java, Apex)
  │   ├── [★★★ PLAN] every LangSpec kind exists in node-types.json (drift guard)
  │   ├── [★★★ PLAN] 30-50 construct fixtures per language (insta SPT + features)
  │   └── [★★★ PLAN] ERROR → #ERR, MISSING dropped, depth cap 1000 (deep-nesting fixture)
  ├── D18 classification + SymbolId
  │   ├── [★★★ PLAN] shadowing, params, catch, lambda, for, fields, statics, SOQL :bind
  │   └── [★★★ PLAN] query-path features == index-path features for 200 units (D18 invariant)
  ├── Normalizer (D21): Java regrouping, Apex SOQL sort/DML/binds
  │   └── [★★★ PLAN] per-construct fixtures; Java --compat SPT == example_query.json
  └── Apex hooks (case-fold, SOQL sort)
      └── [★★★ PLAN] mixed-case identifiers equal; string literals NOT folded; SELECT order-insensitive
[+] cross-implementation
  ├── [★★★ PLAN] differential vs similar.py goldens (D13: venv-generated, checked in)
  └── [★★★ PLAN] recommendation quality: Salesforce idiom suite hit@5 ≥ 60% (D14)

COVERAGE (planned): 54/54 paths have a planned test (100%) | GAPS: 0
QUALITY: ★★★ 48  ★★ 4  ★ 0
```

Test files (Rust conventions): unit tests in-module (`#[cfg(test)]`), integration tests in
`crates/*/tests/*.rs`, CLI e2e in `crates/aroma-cli/tests/cli.rs` (assert_cmd + tempdir),
property tests in `crates/aroma-core/tests/props.rs`, goldens under `tests/golden/` via `insta`,
benchmarks in `crates/aroma-core/benches/` (criterion), recall/perf harness in `xtask`.

## 8. Milestones

| M | Scope | Exit criteria |
|---|---|---|
| **M-1 ✅ done** | **Pilot (D16), result in `docs/results/17-pilot-results.md`:** clusters 7–19% (reference), gated recs win ≈ 28% → recommendations NO-GO, similar-code search GO (D26/D27). Original scope: add quick Apex rules to `concepts/aroma-concepts`; run on 3 single repos (apex-recipes, nebula-logger, one of your orgs if available), ~30 snippet queries each | Report: cluster-formation rate, median members, hand-judged usefulness of clustered extensions vs top-5 similar methods. **Proceed only if clusters form for ≥ 40% of queries and beat the baseline**; otherwise re-decide scope (multi-repo/org corpus, singletons, similar-code-first) before M0 |
| M0 | Workspace from concept crate, `rust-toolchain.toml` ≥ 1.90, C-compiler check, `cargo vendor` docs, CI (fmt, clippy -D warnings, test, cargo-deny), grammar ABI test | CI green; all grammars load |
| M1 | aroma-core: spt, features (profiles), multiset, index, prune, cluster, render on hand-built trees | goldens on `example_query.json`; property tests |
| M2 | Java adapter (compat + scope modes), snippet wrapping, ERROR/MISSING policy | `example_query.java` SPT == reference JSON; adapter coverage suite |
| M3 | store + incremental + CLI (index, query, dump-*, inspect) | persist/load = rebuild; stale detection tests |
| M4 | Eval harness, Java recall + perf gates | Recall targets met; §6 targets met |
| M5 | Apex + SOQL/SOSL → **release v1.0** | adapter suite; Apex self-retrieval Recall@1 ≥ 90%, Recall@100 ≥ 98% |
| M6 | LWC JS + HTML → **release v1.1** | adapter suite; LWC self-retrieval benchmark passes |
| M7 | Salesforce metadata (flows, objects, validation rules, CMDT) → **release v1.2** | adapter suite; large-file budget tests; flow self-retrieval benchmark |
| M8 | Release pipeline lands before v1.0 (GitHub Actions, prebuilt linux/macOS binaries); docs | release checklist |

### 8.1 Implementation notes from the code-to-code search (notes 11–13)

- `index::search`: dense `Vec<u32>` scores + a `touched` list (reset only touched slots); top-k via
  `select_nth_unstable_by` with `(score desc, unit asc)` (pattern from `postings`, note 12).
- `multiset`: one visitor merge `merge(a, b, |f, ca, cb|)` for min/max/sum/Jaccard (glaurung, note 12);
  `jaccard(∅, ∅) = 0` is deliberate (empty query must not look like a perfect match) and tested.
- `prune`: skip leaves whose `dmin == 0` under the Intersection objective; lazy greedy (CELF heap
  with `(gain, Reverse(leaf))` for deterministic ties) **only** for `Profile::Paper`; Jaccard stays
  eager (not submodular) (note 13).
- Feature document-frequency cap: **not** added. At 200k units the Phase I cost of ubiquitous
  features (`#VAR`, `#;1>…`) is ~2M increments (~2 ms); a cap would deviate from both sources.
  Revisit only if the recall harness shows ranking damage.
- Anti-unification rendering with named holes (ctxloom/reprise, BSD-3) is a post-v1 option for
  `render`; v1 keeps reference line-granular output plus byte spans.

## 9. Risks (top 8, from notes 07/09/11–13)

1. Grammar drift silently changes features → pin grammars, manifest fingerprints, feature-diff report on upgrade.
2. tree-sitter shape ≠ ANTLR shape → explicit Java regrouping; only claim exact parity in compat goldens.
3. Partial snippets parse badly → wrapper strategies, damage metric, lexical fallback flag.
4. Local/global misclassification → scope pass; fixtures for shadowing, fields, statics, binds.
5. Pathological units (5k-line Apex, giant flows) → unit caps, budgets, element-level units.
6. Small corpus → few clusters → surface "similar code" even when no cluster forms; show support counts.
7. Feature explosion from literals → string bucketing (versioned).
8. Index staleness → content hashes, stale marking, atomic generations.

## 10. What already exists

| Asset | Reuse |
|---|---|
| `concepts/aroma-concepts/` (compiling, 4 tests, clippy-clean) | Seed of `aroma-core` + Java table spec; M1 starts by moving it into the workspace |
| `ref/aroma-paper-artifacts` reference code + data | Behavioural oracle (D13 goldens), `example_query.*` goldens, datasets for recall harness |
| tree-sitter grammars (java 0.23.5, sfapex 3.0.2, javascript, html) | Parsers only; no normalisation layer exists anywhere to reuse |
| Prior-art repos (Deslop, nose, postings, glaurung, reprise, rueca) | Ideas only (licence-checked); no dependency is worth taking (notes 05, 11–13) |
| `afmt` (Rust Apex formatter on sfapex) | Reference for sfapex tree walking and tree-sitter version pairing |

## 11. NOT in scope (v1.0)

| Item | Why deferred |
|---|---|
| LWC JS + HTML adapters | v1.1 per D5 staging, gated on its own benchmark |
| Salesforce XML metadata | v1.2 per D5; parser choice (roxmltree vs tree-sitter-xml) decided then |
| LSP server / editor plugin | U2: library + CLI first; mmap index (D15) keeps a later LSP cheap |
| MCP server (`aroma mcp`) | D20: declined; agents use CLI JSON |
| HTTP service / daemon | U2; D15 removes the latency reason for a daemon |
| BM25 / embedding / LLM rerank channels | Aroma-only v1; revisit if the idiom suite misses its gate on lexical-heavy tasks |
| Cross-repo / multi-tenant / > 200k units | U3 single-repo scale |
| Visualforce, Aura, dynamic SOQL strings | Not in the requested languages; dynamic SOQL needs a second parse |
| Anti-unification renderer | Post-v1 option (note 13) |
| Default-on clustered recommendations | D27: experimental until ≥ 60% win-rate vs baseline (IDF / multi-repo experiments in TODOS.md) |
| Windows binaries | Linux + macOS first; Windows after CI matrix proves path handling |

## 12. Failure modes (one per new codepath)

| Codepath | Realistic failure | Test? | Handling? | User sees |
|---|---|---|---|---|
| repo walk | huge generated / minified file | yes (size cap) | skip + count | counter in inspect-index |
| TableAdapter walk | 5000-deep nesting | yes | explicit stack, depth cap | unit skipped + counted |
| snippet wrapping | unbalanced braces | yes | `#ERR`, damage ratio | `low_confidence` flag |
| scope pass | shadowed local reused name | yes | `SymbolId` keyed usage chains | correct features |
| feature hash | seed or format change between versions | yes (snapshot) | schema version in manifest | "re-index required" error |
| postings.bin load | truncated / wrong-endian file | yes (fuzz) | header + bounds validation | clear error, no UB |
| candidate render | file edited since index | yes | blake3 mismatch → render from stored SPT | `stale_source` flag on that result |
| prune | 2000+-leaf unit / slow candidate | yes | leaf cap + 25 ms budget | `budget_exhausted` flag |
| cluster | no cluster forms (small repo) | yes | return similar code anyway | empty `recommendations`, non-empty `similar` |
| writer | concurrent index runs | yes | OS lock + timeout | error naming holder PID |
| generation publish | kill -9 mid-write | yes | atomic rename | previous generation still served |

| query deadline | large clusters intersecting | yes | cancel token in every stage (D23a) | `budget_exhausted: <stage>` |
| unit table load | index built with other config | yes | full config fingerprint (D22c) | "re-index required" |
| clone flood | copied boilerplate fills top-100 | yes | normalized-body dedup + backfill (D22d, D23b) | independent support count |
| generation GC | old gen still mapped (Windows) | yes | deferred delete | none (disk reclaimed next run) |

No critical gaps: every failure mode has a planned test, handling, and a visible signal.

## 13. Worktree parallelization

| Step | Modules touched | Depends on |
|---|---|---|
| M0 workspace + CI | root, `.github/` | — |
| M1 core algorithm | `crates/aroma-core/{spt,features,multiset,index,prune,cluster,render,profile}` | M0 |
| M2 TableAdapter + Java spec + scope | `crates/aroma-lang/` | M0 (consumes `Raw` type from M1) |
| M3 store + CLI | `crates/aroma-core/store`, `crates/aroma-cli/` | M1 (core); M2 (CLI e2e tests) |
| M4 eval harness + goldens (D13, D14) | `xtask/`, `tests/golden/` | M1, M2 |
| M5 Apex spec + hooks | `crates/aroma-lang/spec/apex`, `hooks/` | M2 |

Lanes: **M-1 pilot runs first, alone** (go/no-go). Then **A** M0 → M1 → M3-core (store, generation
GC, unit table, `index`/`query` plumbing). **B** M2 → M5 (adapters, `aroma-lang`). **C** M4 harness
(`xtask`, venv golden generation, idiom suite). Launch A; once M1's first commit freezes `Raw`/`Spt`
(including `SymbolId`, D22b), start B and C in parallel worktrees. **Join point (D24a):** M3's CLI
end-to-end tests run only after B delivers the Java adapter (M2). Conflict flag: A and B both touch
the `Raw` contract; change it only by agreement after the freeze.

## 14. Decision log

| ID | Decision | Choice |
|---|---|---|
| D1–D4 | Review target, languages, surface, scale | Drafted plan; Java+Apex+LWC+SF metadata; library+CLI; single repo < 200k |
| D5 | Release staging | v1.0 core+Java+Apex/SOQL; v1.1 LWC; v1.2 metadata; each gated |
| D6 | Default algorithm profile | Reference (similar.py) default; `--profile paper` for evaluation |
| D7 | Feature ids | Stable xxh3 64-bit hashes; no persisted vocabulary |
| D8 | Candidate trees | ~~Re-parse at query time~~ superseded by D19 |
| D9 | Local variables | ~~Scope pass default~~ superseded by D18; `--compat` keeps reference marking |
| D10 | Concurrent writers | Exclusive OS writer lock; lock-free readers |
| D11 | Malformed input | `#ERR`, drop MISSING, damage ratio, depth cap 1000, size/time caps |
| D12 | Adapter structure | One table-driven `TableAdapter` + `LangSpec` tables + hooks |
| D13 | Reference oracle | ~500 goldens generated from similar.py in a Python venv; CI Rust-only |
| D14 | Recommendation quality | Curated Salesforce idiom suite, hit@5 ≥ 60% release gate |
| D15 | Index load latency | mmap CSR `postings.bin` (memmap2 + bytemuck) |
| D17 | Quality gate hardening | Held-out source+clones, required added elements, delta scoring, must beat similar-methods baseline |
| D18 | Normalization consistency | Context-free `#VAR` rule for corpus + query; SymbolId only for shadowing; query-path == index-path invariant |
| D19 | Candidate trees (size premise corrected) | Store compact SPT blobs in `spt.bin`; render from source when hash matches |
| D20 | MCP server | Declined: keep U2 library + CLI |
| D21 | Adapter structure restated | Shared walk → per-language Normalizer rewrite pass → shared classification/collapse |
| D22 | Contract fixes | No unit-kind partition; SymbolId in Raw; full config fingerprint; clone dedup + independent support |
| D23 | Query-path fixes | Deadline/cancel through all stages; eligibility + backfill; query-only unit table; generation GC |
| D24 | Process fixes | Lane join at M2→M3 e2e; M0 toolchain/C-compiler/vendor; goldens on final outputs |
| D26 | Re-scope after pilot round 1 | Similar-code-first; recommendations only with confidence gate (≥ 3 independent members, bounded extension, test-ness match) |
| D27 | Clustering after pilot round 2 (≈ 28% wins) | v1.0 = similar-code search; clustering behind `--experimental-recommend`; promotion bar ≥ 60% wins |
| D28 | Plan cross-review: Apex globals vs locals (amends D18) | A use that resolves to an in-unit declaration is `#VAR` even if its name is a known global (`pattern`, `request`, `type` …); globals decide only unresolved names |
| D29 | Plan cross-review: idiom hit definition | Gate on top-5 similar methods' full source lines containing every required fragment (the pruned region rarely contains fragments the query lacks); pruned-text hit@5 reported |
| D30 | v1.0 idiom gate failed (22.5%; internal research note, not published) | Repair unanswerable idiom tasks, similar list without score floor, BM25 identifier channel fused by RRF (off under `--compat`); 60% gate unchanged |
| D16 | Validate before building | Pilot milestone M-1 with go/no-go rule before M0 |
| — | Doc-frequency cap | Not added (measured cost negligible; would deviate from sources) |
| — | XML parser choice | Deferred to v1.2 |
| — | BM25 lexical channel | Not in v1 (see NOT in scope) |

## 15. Implementation Tasks

Historical implementation checklist; each task derives from a design decision above.

- [x] **T1 (P1, human: ~1 wk / CC: ~3 h)** — pilot — Run the M-1 go/no-go pilot on 3 single Apex repos (DONE: note 17; D26/D27)
  - Files: `concepts/aroma-concepts/src/ts_adapter.rs` (Apex rules), `concepts/aroma-concepts/examples/pilot.rs`, `docs/results/17-pilot-results.md`
  - Verify: report shows cluster rate, median members, usefulness vs top-5 similar baseline; decision recorded
- [ ] **T2 (P2, human: ~1 day / CC: ~20 min)** — infra — M0 workspace, toolchain pin, C-compiler check, CI, grammar ABI test
  - Files: `rust-toolchain.toml`, `Cargo.toml`, `.github/workflows/ci.yml`, `crates/aroma-lang/tests/abi.rs`
  - Verify: `cargo test --workspace` green in CI on linux + macOS
- [ ] **T3 (P1, human: ~1 day / CC: ~20 min)** — aroma-core — Freeze `Raw`/`Spt` contract with `binding: Option<SymbolId>` and byte spans
  - Files: `crates/aroma-core/src/spt.rs`
  - Verify: shadowing fixture produces separate usage chains
- [ ] **T4 (P1, human: ~3 days / CC: ~1 h)** — aroma-lang — D18 context-free `#VAR` classification + query-path == index-path invariant
  - Files: `crates/aroma-lang/src/classify.rs`, `crates/aroma-lang/tests/invariant_query_index.rs`
  - Verify: invariant holds for 200 units (Java + Apex)
- [ ] **T5 (P1, human: ~1 wk / CC: ~2 h)** — aroma-lang — Three-stage adapter: TableAdapter + LangSpec + Normalizer (Java regrouping, Apex SOQL/DML/binds)
  - Files: `crates/aroma-lang/src/{table.rs,spec/,normalize/}`
  - Verify: `--compat` Java SPT == `example_query.json`; LangSpec kinds ⊆ node-types.json
- [ ] **T6 (P1, human: ~2 days / CC: ~30 min)** — aroma-lang — Malformed-input policy (#ERR, MISSING, damage ratio, depth cap, size/timeout caps)
  - Surfaced by: Code quality D11
  - Files: `crates/aroma-lang/src/table.rs`, `crates/aroma-lang/tests/malformed.rs`
  - Verify: 5000-deep nesting fixture indexes without stack overflow; damage counters correct
- [ ] **T7 (P1, human: ~3 days / CC: ~1 h)** — store — `postings.bin` CSR mmap, `spt.bin` blobs, compact unit table, full config fingerprint
  - Files: `crates/aroma-core/src/store/{postings.rs,spt_store.rs,unit_table.rs,manifest.rs}`
  - Verify: one-shot `aroma query` p50 ≤ 100 ms on 200k-unit fixture; fuzzed headers never UB
- [ ] **T8 (P2, human: ~1 day / CC: ~20 min)** — store — Writer lock + generation GC
  - Files: `crates/aroma-core/src/store/{lock.rs,gc.rs}`
  - Verify: concurrent index test; kill -9 mid-write leaves previous generation served
- [ ] **T9 (P2, human: ~2 days / CC: ~45 min)** — query — Deadline/cancel through all stages, eligibility + backfill, clone dedup + independent support
  - Files: `crates/aroma-core/src/{query.rs,index.rs,cluster.rs}`
  - Verify: wall clock ≤ budget + 10 ms; clones never raise support; oversized units backfilled
- [ ] **T10 (P2, human: ~1 day / CC: ~20 min)** — aroma-core — Stable xxh3 feature ids (no vocabulary)
  - Surfaced by: Architecture D7
  - Files: `crates/aroma-core/src/features.rs`
  - Verify: hash snapshot stable across runs/platforms
- [ ] **T11 (P1, human: ~2 days / CC: ~1 h)** — xtask — venv golden generation from similar.py (~500 cases), CI compares final outputs only
  - Files: `xtask/src/gen_goldens.rs`, `tests/golden/reference/`
  - Verify: Rust `--compat` reference profile matches all goldens
- [ ] **T12 (P1, human: ~4 days / CC: ~2 h + your review)** — xtask — Salesforce idiom suite: 40 held-out tasks with required added elements + similar-methods baseline
  - Files: `xtask/tasks/apex/*.toml`, `xtask/src/idiom_suite.rs`
  - Verify: suite runs in CI; reports hit@5 and baseline; gate ≥ 60% for v1.0
