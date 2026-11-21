# aroma-rs

`aroma` searches a repository for code structurally similar to a partial snippet. It re-implements the search pipeline of **Aroma** (Luan et al., OOPSLA 2019) in Rust and tree-sitter, with a library and a command-line interface for Java and Salesforce code, markup and metadata. Search runs locally against an index in `.aroma/`; indexing and querying do not send your source elsewhere. Results show matching regions, folded omissions and exact byte spans for editor integrations—not generated code.

## Supported languages

Routing and unit extraction are defined in `crates/aroma-core/src/lang.rs` and `crates/aroma-lang/src/spec/`. These are eight separate language identities; arbitrary JavaScript, HTML and XML files are not indexed.

| Language ID | File routing | Indexed unit kinds |
|---|---|---|
| `java` | `*.java` | `method` |
| `apex` | `*.cls`, `*.trigger`, `*.apex` | `method`, `constructor`, `trigger`, `accessor` (including inline SOQL/SOSL) |
| `lwcjs` | `…/lwc/<component>/*.js`, also `…/lwc/<component>/__tests__/*.js` | `method` (class methods and top-level functions), `accessor`, `wire_member` |
| `lwchtml` | `…/lwc/<component>/*.html` (not `__tests__`) | `template` (root and nested templates), `markup_block` (large child regions) |
| `sfmeta` | `*.flow-meta.xml`, `*.validationRule-meta.xml`, `*.field-meta.xml`, `*.md-meta.xml`, MDAPI `*.object`, `*.flow` | `flow_element`, `field`, `validation_rule`, `metadata_record`; split regions as `markup_block` |
| `aura` | `…/aura/<bundle>/*.{cmp,app,evt,intf,design,tokens}` | `component`, `markup_block` |
| `aurajs` | `…/aura/<bundle>/*Controller.js`, `*Helper.js`, `*Renderer.js` | `method` (function-valued object properties) |
| `vf` | `*.page`, `*.component` (not `-meta.xml`) | `page`, `markup_block` |

Flow units cover decisions, assignments, record lookups/creates/updates/deletes, loops, screens, subflows, action calls and formulas. MDAPI objects are split into field and validation-rule children. Markup/metadata units over 2,000 leaves are recursively split; the parent remains indexed as oversize. Test units are excluded from queries unless `--include-tests` is set. `--lang lwc` is a CLI shorthand for both LWC adapters when indexing and infers JS versus HTML from query content; `--lang aura` similarly dispatches markup versus JS queries.

## Build and install

From a checkout of [github.com/9prodhi/aroma-rs](https://github.com/9prodhi/aroma-rs), use Rust **1.90.0** (pinned in `rust-toolchain.toml`) and a C compiler on `PATH` for the tree-sitter grammars:

```bash
cargo build --locked --release --workspace
cargo install --locked --path crates/aroma-cli
aroma --help
```

The workspace version is `1.3.0` and matches the v1.3 milestone; the earlier v1.0–v1.2 labels below describe implementation milestones, and the crates are not published to crates.io (`publish = false`). Tagged releases attach prebuilt `aroma` archives for Linux and macOS (x86_64 and aarch64). For offline builds, run `cargo vendor` with network access and follow its configuration instructions.

## Quickstart

Run from the repository you want to search:

```bash
aroma index
aroma inspect-index

# A query may be incomplete and need not compile.
printf '%s\n' 'BufferedReader br = new BufferedReader(new FileReader(name));' \
  'String l;' 'while ((l = br.readLine()) != null) {' > /tmp/snippet.java
aroma query --file /tmp/snippet.java --lang java
aroma query --file /tmp/snippet.java --lang java --json --explain

# Search a whole LWC component, joining its JS and template matches.
aroma query --component force-app/main/default/lwc/myComponent
# Aura bundles also support markup + controller + helper joins.
aroma query --component force-app/main/default/aura/myBundle

# Index only Salesforce XML metadata (structural search is the default).
aroma index --lang sfmeta
aroma query --file /tmp/example.flow-meta.xml --lang sfmeta
# Optional experimental BM25-only baseline; queries follow the stored mode.
aroma index --lang sfmeta --sfmeta-mode lexical
```

Replace component paths and the metadata query file with your own. To index all languages again after the metadata-only example, run `aroma index`.

The walker honours repository `.gitignore` and `.ignore` files and skips hidden paths. Re-indexing reuses unchanged files. Queries find the nearest ancestor index, or use `--root DIR`. Index schema 3 is versioned: incompatible indexes require rebuilding with `aroma index`. See [the CLI reference](docs/usage.md) and `aroma <command> --help` for supported languages, component queries, options and JSON output.

## How it works

1. **Parse → simplified parse tree.** Language-specific tree-sitter tables and normalizers feed a shared core. Local-variable normalization reduces sensitivity to variable naming. Metadata uses order-insensitive children and formula-expression features.
2. **Tree → features.** Token, ancestor, sibling and variable-use relationships become bags of structural features, stored in a memory-mapped inverted index with language namespaces.
3. **Retrieve.** Structural feature overlap finds candidates. A second channel uses BM25 over identifier terms (whole names plus camelCase/separator splits, not code string/number literals). Reciprocal rank fusion (RRF) combines the rankings.
4. **Prune and rerank.** Candidate trees are greedily pruned toward the query and scored by weighted Jaccard similarity. Display ranking combines structural and lexical evidence; exact query-prefix matches are promoted to settle near-ties. `--no-lexical` disables the lexical channel; `--compat` indexes retain reference-compatible structural behaviour. Scores run from 0 to 1 with no result cutoff: low scores are weak matches. Rendered results fold omitted lines as `// your code ...` or a language-appropriate marker.

Snippet repair tries statement/member/expression wrappers, closes unmatched brackets or markup tags and can trim up to three trailing lines. Repairs do not make arbitrary broken code valid; parse damage is reported, with low confidence above 10%. Component queries search each part normally, then sum per-part RRF evidence by folder, crediting exact-body clones in every folder that contains them (D45/D50).

Clustered Aroma recommendations are available via `--experimental-recommend`, off by default. They use results above 0.65, require at least three independent supporting methods and bound the extension size. The product is similar-code search; recommendations have not demonstrated superiority to that baseline.

## Evaluation and release gates

These are recorded release measurements, **not promises about your repository or hardware**. Sources: [record 19](docs/results/19-v1-release-gates.md) (including its correction and resolution), [record 27](docs/results/27-v1.1-release-gate.md), and [record 29](docs/results/29-v1.2-v1.3-release-gates.md). Runs used Linux, 4 CPUs, 11 GB RAM, Rust 1.90.0 and release builds.

Recall is **self-retrieval**: the source remains indexed; rank is its displayed position (or an exact-body clone). Idiom hit@5 holds out the source and exact clones and checks required fragments in full result source, not just rendered/pruned text. These are different tests.

| Milestone / gate | Target | Recorded result | Outcome |
|---|---|---|---|
| v1.0 Java contiguous R@1 / R@100 (1,000 queries) | ≥ 0.95 / ≥ 0.99 | 0.950 / 0.999; fresh seed 4242: 0.954 / 1.000 | Pass, thin margin |
| v1.0 Apex pooled R@100 (1,347 queries) | ≥ 0.98 | 0.997 (R@1 0.971, not gated) | Pass |
| v1.0 Salesforce idiom similar hit@5 | ≥ 60% | 25/40 = 62.5% | Pass, one-task margin |
| v1.1 LWC JS pooled start R@1 (n = 6,141) | ≥ 0.95 | 0.9489 (seed 119), 0.9477 (seed 4242) | **Short; shipped with the shortfall recorded (D49)** |
| v1.1 LWC JS pooled start R@100 | ≥ 0.99 | 0.9990 on both seeds | Pass |
| v1.1 LWC HTML pooled start R@100 (n = 4,059) | ≥ 0.98 | 0.9973 on both seeds | Pass |
| v1.1 LWC idiom similar hit@5 (20 tasks, 6 component) | ≥ 60% | 13/20 = 65.0%; later D50: 14/20 = 70.0% | Pass |
| v1.2 flow elements / flows corpus floor | ≥ 1,000 / ≥ 50 | 6,835 elements from 869 flows | Pass |
| v1.2 validation rules / formula fields corpus floor | ≥ 150 each | 150 / 519 | Pass; rules have no margin |
| v1.2 pooled start R@100: `flow_element`, `validation_rule`, `field` | ≥ 0.95 each | 1.0000 each (structural default) | Pass |
| v1.2 structural vs BM25-only metadata hit@5 | ≥ +5 points | 7/15 vs 3/15 = +26.7 | Pass; structural ships by default |
| v1.2 suite validity (D48) | 15/15 | 15/15 | Pass |
| v1.3 Aura bundles / VF `.page` corpus floor | ≥ 300 each | 402 / 444 | Pass |
| v1.3 Aura markup / Aura JS / VF pooled start R@100 | ≥ 0.98 each | 0.9823 / 1.0000 / 0.9887 | Pass |
| v1.3 Aura component join | works | D50 clone credit; real bundle `strike_tile` rank 2 | Pass |
| NPSP full index (v1.0 → all-language rerun) | ≤ 60 s | 6.29 s → 8.38 s | Pass |
| One-shot query, 233,318 Java units, 100 queries (v1.0 → rerun) | p50 ≤ 100 ms, p95 ≤ 500 ms | p50 89.3 ms, p95 150.2 ms → p50 90.4 ms, p95 194.1 ms | Pass |
| Component query (50 LWC components) | p50 ≤ 200 ms | p50 28.8 ms, p95 41.1 ms | Pass |
| Query peak RSS, fresh process (v1.0 → rerun) | ≤ 1 GiB | 258 MiB → 260 MiB | Pass |

Reference-compatibility differential tests matched **520/520** golden cases at every stage (note 19). The original Java R@1 report of 0.991 was wrong: D40 corrected displayed-rank accounting to 0.869; the subsequent exact-prefix display rule produced the final numbers above. No gate was relaxed to conceal this correction or the LWC shortfall.

### Reproduce

Run from the workspace root with network access and Git. Harnesses fetch pinned third-party repositories into `target/corpus/` (under `CARGO_TARGET_DIR` when set). Treat corpus contents as untrusted data; do not execute their build scripts or programs.

```bash
cargo run -p xtask -- fetch-reference
cargo run -p xtask -- fetch-corpus --stage v1.1
cargo run -p xtask -- fetch-corpus --stage v1.2
cargo run -p xtask -- fetch-corpus --stage v1.3

cargo run --release -p xtask -- recall --java-repos 20 --samples 1000 --seed 119
cargo run --release -p xtask -- recall --java-repos 20 --samples 1000 --seed 4242
cargo run --release -p xtask -- recall --suite v1.1
cargo run --release -p xtask -- recall --suite v1.1 --seed 4242
cargo run --release -p xtask -- recall --suite v1.2 --sfmeta-mode structural
cargo run --release -p xtask -- recall --suite v1.3

cargo run --release -p xtask -- idiom --check
cargo run --release -p xtask -- idiom
cargo run --release -p xtask -- idiom --suite lwc --check
cargo run --release -p xtask -- idiom --suite lwc
cargo run --release -p xtask -- idiom --suite sfmeta --check
cargo run --release -p xtask -- idiom --suite sfmeta --sfmeta-mode structural
cargo run --release -p xtask -- idiom --suite sfmeta --sfmeta-mode lexical

cargo run --release -p xtask -- perf --java-repos 200
```

The v1.1 recall command is expected to report the recorded R@1 failure. Note 29 records a harness limitation: selecting fewer Java repositories deletes the others from the shared corpus directory. Run the full perf fixture after smaller recall runs, and avoid concurrent harness runs sharing a corpus. Three of the first 200 Java reference repositories failed to clone in the recorded run; fixture availability can change.

## Limitations

- Single-repository search, not whole-ecosystem recommendation. Only one machine was measured; query p50 has about 10 ms of headroom. Index-build peak RSS was 2,486 MiB at 233k Java units (reported, not gated).
- Self-retrieval measures findability, not usefulness. Small developer-written idiom suites are not a user study. Some ranking choices were selected on the release corpus; a held-out seed only partly mitigates that bias. The Java prefix rule favours the first-line queries used by its gate.
- Near-duplicate methods and missing decorators remain hard to distinguish. LWC JS R@1 is below target. Repair cannot close every cut ternary, template literal or comma-separated declaration.
- Reported, not gated: LWC HTML template R@1 0.579; `markup_block` end-slice R@100 0.56; Aura / Aura JS / VF R@1 0.8470 / 0.9774 / 0.5784. Strong start-slice recall does not imply equally strong interior-fragment search.
- Files over 2 MiB, parse timeouts and badly damaged units can be skipped. Indexes must be rebuilt after incompatible format changes. CI had not run when these results were recorded; note 19 records it as open.
- Clustered recommendations remain experimental (idiom suite: 0 wins / 4 ties / 1 loss). See [TODOS.md](TODOS.md) for tracked follow-ups.

## Project layout and development

| Path | Purpose |
|---|---|
| `crates/aroma-core/` | language-agnostic trees, features, retrieval, pruning, storage and lexical ranking |
| `crates/aroma-lang/` | tree-sitter adapters, normalization, indexing and query orchestration |
| `crates/aroma-cli/` | `aroma` command-line application |
| `xtask/` | pinned corpus fetching and recall, idiom and performance harnesses |
| `docs/` | [CLI reference](docs/usage.md), [original design](docs/PLAN-aroma-rs.md), [language expansion plan](docs/PLAN-aroma-rs-v1.1-v1.2.md) |
| `docs/results/` | public evaluation records, corrections and measurement caveats |
| `docs/summary/` | [one-page project summary](docs/summary/summary.html), [its sources](docs/summary/sources.md) and a [guide](docs/summary/README.md) |

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

### AI-assisted development

This project was built with Claude orchestrating Codex, Pi and Antigravity agents. Humans review the work; AI-generated implementation, tests, documentation and evaluation claims still require scrutiny.

## Citation

This project re-implements the search pipeline from:

> S. Luan, D. Yang, C. Barnaby, K. Sen, and S. Chandra. **Aroma: Code Recommendation via Structural Code Search.** Proceedings of the ACM on Programming Languages (OOPSLA), 2019. arXiv:1812.01158.

The Rust implementation and its extensions are not the authors' reference implementation, and these repository measurements are not the paper's results.

## License and third-party materials

The repository's own code is licensed under **Apache-2.0**; see [LICENSE](LICENSE).
See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for task-snippet license texts and the Aroma paper running-example attribution.

The authors' reference implementation and datasets (`facebookresearch/aroma-paper-artifacts`) are **CC BY-NC-SA 4.0**, not Apache-2.0. `xtask fetch-reference` fetches them at a pinned commit into ignored `ref/aroma-paper-artifacts/` for tests and evaluation. Do not commit or redistribute that directory. The golden test fixtures are outputs of the Aroma reference implementation run on self-written inputs. The reference implementation is fetched for testing, never vendored, and is licensed CC BY-NC-SA 4.0; its license is distinct from this project's Apache-2.0 license. aroma-rs is an independent re-implementation. The paper PDF is not redistributed. Evaluation corpora are third-party code with their own licenses, fetched separately and not included in this repository or its releases. Dependency licenses also remain their own.
