# aroma usage

`aroma` indexes one repository of Java, Apex, Lightning Web Components (JavaScript
and HTML), Salesforce XML metadata, Aura (markup and JavaScript), and Visualforce,
and searches it for code structurally similar to a snippet or component. Results go to standard output; with `--json` standard output carries
exactly one JSON document. Logs, warnings and error messages (prefixed `aroma:`) go to
standard error. `aroma --version` prints the version.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | failure (unreadable file, no supported source files to index, a file with no unit or snippet to dump, I/O error) |
| 2 | usage error (unknown command or option, missing `--file`/`--stdin`/`--component`, unknown language, language not inferable from the file extension) |
| 3 | index missing, incompatible with this binary, or corrupt: run `aroma index` |
| 4 | another `aroma index` holds the writer lock past `--lock-timeout` |

## Environment

| Variable | Effect |
|---|---|
| `RUST_LOG` | log filter for standard error, e.g. `RUST_LOG=info` or `RUST_LOG=aroma_lang=debug` (default `warn`). Source text and literals are never logged. |

### Development

These settings concern test/reference tooling, not normal indexing or querying:

| Name | Effect |
|---|---|
| `AROMA_REFERENCE_DIR` | Environment override for reference artifacts in tests and xtask; default `ref/aroma-paper-artifacts`. Fetch with `cargo run -p xtask -- fetch-reference`. Some reference-dependent tests return early when artifacts are absent. |
| `AROMA_PYTHON` | Python executable used by `xtask gen-goldens` to create its development virtual environment; default `python3`. An existing virtual environment is reused. |
| `AROMA_DIR` | Not an environment override in this workspace. It is the Rust constant naming the `.aroma` index directory. Use `--root DIR` to select the indexed repository; setting an environment variable of this name does not relocate the index. |

### Languages and routing

`index --lang` accepts comma-separated language IDs; omitted means all adapters.
Single-snippet `query --lang` accepts one ID or a content-dispatch alias:

| ID | Files indexed | Query language |
|---|---|---|
| `java` | `*.java` | Java |
| `apex` | `*.cls`, `*.trigger`, `*.apex` | Apex, including inline SOQL/SOSL |
| `lwcjs` | `…/lwc/<component>/*.js` and direct `__tests__/*.js` children | LWC JavaScript |
| `lwchtml` | `…/lwc/<component>/*.html`, excluding `__tests__` | LWC HTML templates |
| `lwc` (alias) | expands to `lwcjs,lwchtml` | `lwchtml` if the first nonblank character is `<`, otherwise `lwcjs` |
| `sfmeta` | `*.flow-meta.xml`, `*.validationRule-meta.xml`, `*.field-meta.xml`, `*.md-meta.xml`, MDAPI `*.object` and `*.flow` | Salesforce XML metadata; query mode follows the index |
| `aura` | expands to Aura markup (`…/aura/<bundle>/*.{cmp,app,evt,intf,design,tokens}`) and `aurajs` | Aura markup ID and query alias: markup if the first nonblank character is `<`, otherwise `aurajs` |
| `aurajs` | `…/aura/<bundle>/*Controller.js`, `*Helper.js`, `*Renderer.js` | Aura JavaScript |
| `vf` | `*.page`, `*.component` (not `-meta.xml`) | Visualforce |

`index --lang aura` selects both Aura markup and JavaScript, just as `lwc`
selects both LWC adapters. Generic JS, HTML and XML outside these routing rules
are not indexed. See the [project language table](../README.md#supported-languages)
for unit kinds.

## Commands

### `aroma index [PATH]`

Build or update the index of the repository at `PATH` (default `.`), stored in
`PATH/.aroma/`.

| Option | Default | Meaning |
|---|---|---|
| `--lang L1,L2` | all | comma-separated language IDs or `lwc` alias from the routing table above |
| `--profile reference\|paper` | `reference` | algorithm profile recorded in the index (`paper` = the paper's Phase I cap 1000 and intersection-size pruning) |
| `--sfmeta-mode structural\|lexical` | `structural` | metadata mode; lexical is experimental BM25-only |
| `--compat` | off | reference-compatible variable marking (golden comparisons only) |
| `--jobs N` | one per core | worker threads |
| `--lock-timeout SECS` | 30 | wait for another writer; `0` tries once |
| `--json` | off | print the build report as JSON |

The walk honours `.gitignore` and `.ignore` files inside the repository (not those of
parent directories) and skips hidden files and directories. Files over 2 MiB are skipped;
a parse is abandoned after 2 s; methods whose body is more than 10% unparsable are skipped;
all are counted in the report. Only files whose content hash changed are re-parsed, except
that adding, removing or renaming a type re-parses every file of that language (variable
marking depends on the set of known type names). Every run, even one where nothing changed,
writes a new generation; the previous one is kept for queries still reading it and older
ones are removed.

Metadata queries use the index's recorded mode. Manifests without `sfmeta_mode` remain
lexical for compatibility; a new build defaults to structural and rebuilds an existing
lexical index. Lexical queries report `experimental_lexical_only: true`.

Text output:

```text
indexed generation 2: 12878 units from 1070 files (3 parsed, 1067 unchanged, 0 too large, 0 failed), 4 units skipped as damaged
removed old generations: 1
```

JSON output (`--json`):

```json
{
  "generation": 2,
  "files_seen": 1070,
  "files_reparsed": 3,
  "files_unchanged": 1067,
  "files_skipped_large": 0,
  "files_failed": 0,
  "units": 12878,
  "units_skipped_damaged": 4,
  "gc_removed": [1]
}
```

### `aroma query (--file F | --stdin) --lang L`

Search the index for code similar to the snippet in `F` or on standard input.
`--lang` is required for a single file or stdin; component forms infer each part's
language from its routable path (see below).

| Option | Default | Meaning |
|---|---|---|
| `--lang L` | required for single-snippet queries | any language ID or query alias in the routing table above |
| `--root DIR` | nearest ancestor of the working directory (itself included) containing an index in `.aroma/` | repository to search |
| `--similar N` | 10 | number of similar results: the N best-scoring candidates, with no score cutoff |
| `--experimental-recommend` | off | also compute Aroma's clustered recommendations from the results scoring above 0.65; shown only with ≥ 3 independent supporting methods and an extension of at most twice the snippet |
| `--top N` | 5 | maximum recommendations printed |
| `--kind K` | any | only units of kind `method`, `constructor`, `trigger`, `accessor`, `initializer`, `snippet`, `template`, `markupblock`, `wiremember`, `flowelement`, `field`, `validationrule`, `metadatarecord`, `page`, or `component` (CLI spellings have no underscores) |
| `--include-tests` | off | also return methods from test files |
| `--budget-ms MS` | none | time budget; when exceeded, finished results are returned and `budget_exhausted` names the stage |
| `--no-lexical` | off | rank by structure only: turn off the BM25 identifier channel (it is always off on an index built with `--compat`) |
| `--explain` | off | attach matched / query-only / candidate-only features and the lexical ranks to each result |
| `--json` | off | print `QueryOutput` (schema below) |

The snippet can be a few statements, a whole method, a class member or an expression; it is
wrapped automatically and need not compile. `diagnostics.parse_damage` is the share of the
snippet's bytes that did not parse; above 10% the result is flagged `low_confidence` and a
warning is printed on standard error. If a file changed after indexing, its results show the
indexed tokens instead of the current lines and are flagged `stale_source`.

Every candidate method is pruned towards the snippet and scored: `score` is the weighted
Jaccard similarity (0 to 1) of the pruned method and the snippet. There is no score cutoff:
the score is shown with each result, and a low score means a weak match. Recommendations
still cluster only results scoring above 0.65 (the Aroma reference threshold).

Candidates come from two rankings. The structural one is Aroma's feature-overlap search. The
lexical one is BM25 (k1 1.2, b 0.75) over the snippet's identifier terms: every identifier
is lower-cased and kept whole and split at camelCase, `_`, `$` and digit boundaries
(`saveResult` gives `saveresult`, `save`, `result`; Apex names are already lower-cased, so
only `_` and digit splits apply there); terms shorter than two characters and language
keywords are dropped, and string, character and number literals are never indexed. The best
100 of each ranking are fused by reciprocal rank fusion (`sum of 1 / (60 + rank)`) into the
100 candidates that are pruned and scored; results are then ordered by the fusion of their
rank by `score` and their BM25 rank, ties broken by index order. Exact-token-prefix
matches (at least three leaves) are promoted in lexical mode to resolve near-ties. With `--no-lexical`, or on
an index built with `--compat`, only the structural ranking is used and results are ordered
by `score` alone (the reference pipeline). The same language, `--kind`, test-file, size and
clone filters apply to both rankings.

Text output, one block per result (`no similar code found` when there is none), then the
recommendations, then a summary line:

```text
#1 src/main/java/demo/FileUtil.java:10-19 method score 1.000
    public static List<String> readLines(String path) throws IOException {
// your code ...
        BufferedReader reader = new BufferedReader(new FileReader(path));
        String line;
        while ((line = reader.readLine()) != null) {
// your code ...
    }

-- generation 1, profile reference, wrapper statements, parse damage 0.00, 35/35 query features known, 0 ineligible skipped, 0 stale, 9/11 query terms known
```

With `--explain` each result also gets a line
`  explain: jaccard <x>, <m> matched, <q> query-only, <c> candidate-only` followed by up to
20 `    query-only: <feature>` lines and, when the lexical channel ran,
`  lexical: structural rank <s>, bm25 rank <l or ->, bm25 <x>, terms: <matched terms>`.
The summary line gains `, <k>/<q> query terms known` when the lexical channel ran. Stale results add
` [stale source: showing indexed tokens]` to their header. Recommendations print as
`recommendation #<i> (independent support <n>): <members>` followed by the rendered text.
The summary line ends with `, budget exhausted during <stage>` when the budget ran out and
`, no evidence` when the snippet produced no tree or no structural feature (and no identifier term) known to the index.

### Whole-component search

```bash
aroma query --component force-app/main/default/lwc/myComponent --root .
aroma query --file force-app/main/default/lwc/myComponent/myComponent.html \
  --file force-app/main/default/lwc/myComponent/myComponent.js --json
aroma query --component force-app/main/default/aura/myBundle
aroma query --file force-app/main/default/aura/myBundle/myBundle.cmp \
  --file force-app/main/default/aura/myBundle/myBundleController.js \
  --file force-app/main/default/aura/myBundle/myBundleHelper.js
```

Use `--component DIR` or repeat `--file` at least twice, not `--stdin`. A directory
query reads only directly contained routable files, skips `__tests__`, and uses the
index's enabled languages. Multi-file queries infer languages from each path;
`--lang` is not needed and does not override this inference. Paths therefore need
to retain routing context such as `lwc/<component>/` or `aura/<bundle>/`.

Each part runs a normal search (top 100); results are joined by component folder
and ranked by the sum of per-part RRF values. A missing part contributes zero.
Exact-body clones receive credit in every eligible folder that contains them
(D50); each folder keeps its best region for each part. `--similar N` limits the
number of returned components (default 10). Text output lists matched regions
per component; `--json` returns `ComponentOutput`, with `components`, `parts`
(per-part language, path and diagnostics), `generation`, and `schema_version`,
not the single-snippet `QueryOutput` schema below. Query filters such as `--kind`
and `--include-tests` still apply to the parts.

### `aroma dump-spt FILE`

Print the simplified parse tree of every method in `FILE`. Each unit starts with a header
line `== <kind> lines <first>-<last> (damage <d>)`, followed by one line per node label and
per token leaf, indented two spaces per depth. Keywords and punctuation appear inside the
node labels (`#` marks a child position, e.g. `#=#`), not as lines of their own. Local
variables are marked `#VAR`, with `s<n>` when bound to a declaration in the method. A file
without methods is treated as a snippet (header `== snippet (wrapper <w>, damage <d>)`).
Options: `--lang java|apex|sfmeta` (default: inferred from the routable path), `--compat`.
The dump and feature-diff commands currently accept only those three explicit
language overrides; omit `--lang` to infer other adapters from a routed path.

### `aroma dump-features FILE`

Print the structural features of every method in `FILE` (each unit under the same `==`
header as `dump-spt`) as `<count>  <feature>`, sorted by feature, or with `--with-origins`
the features of each token (`leaf <i> line <l> "<text>"` followed by its features, indented
two spaces). Options: `--lang`, `--with-origins`, `--compat` (reference-compatible variable
marking and feature configuration).

### `aroma feature-diff A B`

Compare the features of two files or snippets (all units of a file are summed): prints
`jaccard <x> (<shared> shared of <union>)`, then `only in <A> (<total>):` and
`only in <B> (<total>):`, each followed by the features that occur more often on that side
and by how many occurrences (`<count>  <feature>`). Options: `--lang` (default: from `A`'s
extension), `--compat`.

### `aroma inspect-index`

Print the index manifest (generation, schema, profile, compat flag, feature fingerprint,
adapter and grammar versions), unit counts by language and kind, test units, feature and
posting counts and the last build report. Options: `--root DIR` (default as for
`aroma query`), `--json` (prints `{"root": ..., "manifest": {...}, "stats": {...}}`).

```text
index generation 1 at /path/to/repo/.aroma
profile reference, compat false, schema 2, feature schema 1, fingerprint 0xfbd17e83627d6d92
adapters: apex apex-1, java java-1
grammars: apex tree-sitter-sfapex 3.0.2, java tree-sitter-java 0.23.5
units 1 (0 tests) in 1 files; 71 features, 71 postings
by language: java 1
by kind: method 1
last build: 1 files seen, 1 parsed, 0 unchanged, 0 too large, 0 failed, 0 units skipped as damaged
```

## Index layout

`.aroma/` at the repository root (add it to `.gitignore`):

| Path | Content |
|---|---|
| `lock` | writer lock (holder pid); readers never lock |
| `CURRENT` | number of the current generation |
| `gen-<n>/manifest.json` | schema and feature-schema versions, feature fingerprint, profile, compat flag, adapter and grammar versions, counts |
| `gen-<n>/postings.bin` | inverted index (memory-mapped) |
| `gen-<n>/lexical.bin` | identifier-term index for the BM25 channel: term ids, per-unit term frequencies and document lengths (memory-mapped; identifier terms only, never literals) |
| `gen-<n>/units.bin`, `files.json` | unit table (memory-mapped) and indexed file paths |
| `gen-<n>/spt.bin` | compact parse trees of every unit |
| `gen-<n>/globals.json` | known type names per language |
| `gen-<n>/inventory.postcard` | per-file hashes (incremental builds, stale-source detection at query time) and per-file features (incremental builds) |
| `gen-<n>/build.json` | the build report |

A binary refuses an index whose manifest differs from what it would write (exit code 3); run
`aroma index` again. The index schema is 3 since the adapter/language expansion; an older-schema index
from an earlier release is refused and `aroma index` rebuilds it in full.

## QueryOutput JSON schema

`aroma query --json` prints one object; `schema_version` changes only on incompatible
changes (fields may be added: `structural_rank`, `lexical_rank`, `bm25`, `matched_terms` and
the `lexical`, `query_terms`, `known_query_terms` diagnostics were added without a version
change). `lines` are 1-based and inclusive; `kept_spans` are byte ranges `[start, end)` into
the file as indexed. `explain` is `null` unless `--explain` is given. Recommendation
`members` are `"path:first-last"` and `text` renders the first member.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "aroma QueryOutput, schema_version 1",
  "type": "object",
  "additionalProperties": false,
  "required": ["schema_version", "generation", "profile", "similar", "recommendations", "diagnostics"],
  "properties": {
    "schema_version": { "type": "integer", "enum": [1] },
    "generation": { "type": "integer", "minimum": 1 },
    "profile": { "type": "string", "enum": ["reference", "paper"] },
    "similar": { "type": "array", "items": { "$ref": "#/$defs/similar" } },
    "recommendations": { "type": "array", "items": { "$ref": "#/$defs/recommendation" } },
    "diagnostics": { "$ref": "#/$defs/diagnostics" }
  },
  "$defs": {
    "span": {
      "type": "array",
      "items": { "type": "integer", "minimum": 0 },
      "minItems": 2,
      "maxItems": 2
    },
    "similar": {
      "type": "object",
      "additionalProperties": false,
      "required": [
        "path", "lines", "kind", "score", "text", "kept_spans", "stale_source", "explain",
        "structural_rank", "lexical_rank", "bm25", "matched_terms"
      ],
      "properties": {
        "path": { "type": "string", "description": "repository-relative, '/'-separated" },
        "lines": { "$ref": "#/$defs/span", "description": "first and last line, 1-based inclusive" },
        "kind": { "type": "string", "enum": ["method", "constructor", "trigger", "accessor", "initializer", "snippet", "template", "markupblock", "wiremember", "flowelement", "field", "validationrule", "metadatarecord", "page", "component"] },
        "score": { "type": "number", "description": "weighted Jaccard of the pruned method and the snippet, 0 to 1; no cutoff, low scores are weak matches (recommendations use only scores above 0.65)" },
        "text": { "type": "string", "description": "kept lines verbatim, omitted runs as '// your code ...'" },
        "kept_spans": { "type": "array", "items": { "$ref": "#/$defs/span" } },
        "stale_source": { "type": "boolean" },
        "explain": {
          "type": ["object", "null"],
          "additionalProperties": false,
          "required": ["matched", "query_only", "candidate_only", "jaccard"],
          "properties": {
            "matched": { "type": "array", "items": { "type": "string" } },
            "query_only": { "type": "array", "items": { "type": "string" } },
            "candidate_only": { "type": "array", "items": { "type": "string" } },
            "jaccard": { "type": "number" }
          }
        },
        "structural_rank": { "type": "integer", "minimum": 1, "description": "rank by score among the scored candidates" },
        "lexical_rank": { "type": ["integer", "null"], "minimum": 1, "description": "rank among the best 100 BM25 identifier matches; null when the lexical channel is off or the method is not among them" },
        "bm25": { "type": ["number", "null"], "minimum": 0, "description": "BM25 score over the snippet's identifier terms; null when the lexical channel is off" },
        "matched_terms": { "type": "array", "items": { "type": "string" }, "description": "snippet identifier terms the method contains, sorted; empty when the lexical channel is off" }
      }
    },
    "recommendation": {
      "type": "object",
      "additionalProperties": false,
      "required": ["members", "independent_support", "text", "kept_spans", "stale_source"],
      "properties": {
        "members": { "type": "array", "items": { "type": "string" } },
        "independent_support": { "type": "integer", "minimum": 3 },
        "text": { "type": "string" },
        "kept_spans": { "type": "array", "items": { "$ref": "#/$defs/span" } },
        "stale_source": { "type": "boolean" }
      }
    },
    "diagnostics": {
      "type": "object",
      "additionalProperties": false,
      "required": [
        "parse_damage", "low_confidence", "wrapper", "no_evidence", "query_features",
        "known_query_features", "ineligible_skipped", "stale_units", "budget_exhausted",
        "lexical", "query_terms", "known_query_terms"
      ],
      "properties": {
        "parse_damage": { "type": "number", "minimum": 0, "maximum": 1 },
        "low_confidence": { "type": "boolean", "description": "parse_damage above 0.10" },
        "wrapper": { "type": "string", "enum": ["statements", "member", "expression", "none"] },
        "no_evidence": { "type": "boolean", "description": "the snippet produced no tree or no structural feature (and no identifier term) known to the index" },
        "query_features": { "type": "integer", "minimum": 0 },
        "known_query_features": { "type": "integer", "minimum": 0 },
        "ineligible_skipped": { "type": "integer", "minimum": 0, "description": "candidates skipped by language, --kind, test or size filters and clone de-duplication" },
        "stale_units": { "type": "integer", "minimum": 0 },
        "budget_exhausted": {
          "type": ["string", "null"],
          "enum": ["search", "lexical", "load", "rerank", "cluster", "intersect", null]
        },
        "lexical": { "type": "boolean", "description": "the BM25 identifier channel ran" },
        "query_terms": { "type": "integer", "minimum": 0, "description": "distinct identifier terms of the snippet (0 when the channel is off)" },
        "known_query_terms": { "type": "integer", "minimum": 0, "description": "snippet terms present in the index" }
      }
    }
  }
}
```

Example (the quickstart repository, `--explain`, `kept_spans` and feature lists
abbreviated):

```json
{
  "schema_version": 1,
  "generation": 1,
  "profile": "reference",
  "similar": [
    {
      "path": "src/main/java/demo/FileUtil.java",
      "lines": [10, 19],
      "kind": "method",
      "score": 1.0,
      "text": "    public static List<String> readLines(String path) throws IOException {\n// your code ...\n        BufferedReader reader = new BufferedReader(new FileReader(path));\n        String line;\n        while ((line = reader.readLine()) != null) {\n// your code ...\n    }\n",
      "kept_spans": [[308, 322], [323, 329]],
      "stale_source": false,
      "explain": { "matched": ["#!=null1>#VAR", "##1>BufferedReader"], "query_only": [], "candidate_only": [], "jaccard": 1.0 },
      "structural_rank": 1,
      "lexical_rank": 1,
      "bm25": 3.351680555648152,
      "matched_terms": ["buffered", "bufferedreader", "file", "filereader", "line", "read", "reader", "readline", "string"]
    }
  ],
  "recommendations": [],
  "diagnostics": {
    "parse_damage": 0.0,
    "low_confidence": false,
    "wrapper": "statements",
    "no_evidence": false,
    "query_features": 35,
    "known_query_features": 35,
    "ineligible_skipped": 0,
    "stale_units": 0,
    "budget_exhausted": null,
    "lexical": true,
    "query_terms": 11,
    "known_query_terms": 9
  }
}
```

## Evaluation harnesses (developers)

Run with `--release`; they clone pinned third-party repositories into `target/corpus/`.

| Command | What it does | Gate |
|---|---|---|
| `cargo run -p xtask -- fetch-reference [DIR]` | clone the reference artifacts (CC BY-NC-SA 4.0) at the pinned commit | none |
| `cargo run --release -p xtask -- recall [--only java\|apex] [--java-repos N] [--samples N] [--seed S]` | paper §5 Recall@1/@100 with 30 bootstrap resamples; `--repo PATH --lang L` for one local checkout (reported only) | Java R@1 ≥ 0.95, R@100 ≥ 0.99; Apex R@100 ≥ 0.98 |
| `cargo run --release -p xtask -- idiom [--tasks DIR] [--only ID]` | held-out Salesforce idiom suite (`xtask/tasks/apex/*.toml`) | similar-code hit@5 ≥ 60%: a top-5 similar method whose full source lines contain every required fragment; pruned-text hit@5 and recommendation win rate reported, not gated |
| `cargo run --release -p xtask -- perf [--java-repos N] [--queries N] [--component-queries N] [--seed S] [--skip-index] [--skip-query] [--skip-component]` | NPSP full-index time (all specced languages) with units per language, one-shot `aroma query` latency, one-shot `aroma query --component` latency on `trailheadapps/lwc-recipes` LWC folders, peak RSS | NPSP ≤ 60 s; query p50 ≤ 100 ms, p95 ≤ 500 ms; component query p50 ≤ 200 ms; query-path RSS ≤ 1 GiB (index-build RSS reported only) |
