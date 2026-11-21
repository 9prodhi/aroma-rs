#!/usr/bin/env python3
"""aroma-rs golden driver (task T14, decision D13).

Runs the reference functions of facebookresearch/aroma-paper-artifacts `similar.py` on
our own inputs and writes the final outputs that `crates/aroma-lang/tests/differential.rs`
compares against. Invoked only by `cargo run -p xtask -- gen-goldens`, with the venv
interpreter and `python -I`, from a scratch copy of `similar.py`.

similar.py runs argparse and loads a working directory at import time, so it is not
imported: its source is verified against a pinned sha256, cut just before that
module-level entry point, and executed into a fresh module object. Only reference
functions compute results; this file adds glue (record plumbing, a deterministic Phase I
tie order, mask extraction) and one documented deviation (intern_unknown_features, D7).

Inputs (JSON lines written by gen_goldens.rs):
  corpus.jsonl   {"path", "beginline", "endline", "source", "ast"} per corpus method
  queries.jsonl  {"id", "base", "mutation", "path", "beginline", "endline", "source", "ast"}
Outputs (in --out):
  meta.json      provenance, configuration, counts, skipped cases
  corpus.json    JSON array, one corpus unit per line
  cases.json     JSON array, one case per line
"""

import argparse
import contextlib
import hashlib
import io
import json
import os
import platform
import types

ENTRY_POINT = "\nlogging.basicConfig(level=logging.DEBUG)\noptions = parse_args()\n"


class Skip(Exception):
    pass


def load_similar(path, expected_sha256):
    with open(path, "rb") as f:
        data = f.read()
    digest = hashlib.sha256(data).hexdigest()
    if digest != expected_sha256:
        raise SystemExit(f"{path}: sha256 {digest}, expected {expected_sha256}")
    src = data.decode("utf-8")
    if src.count(ENTRY_POINT) != 1:
        raise SystemExit(f"{path}: module-level entry point not found exactly once")
    defs = src[: src.index(ENTRY_POINT) + 1]
    mod = types.ModuleType("similar")
    mod.__file__ = path
    exec(compile(defs, path, "exec"), mod.__dict__)
    mod.config = mod.Config()
    mod.vocab = mod.Vocab()
    mod.options = types.SimpleNamespace(working_dir=None)
    return mod


def read_jsonl(path):
    with open(path, "r", encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]


def is_leaf(t):
    return isinstance(t, dict) and "leaf" in t and bool(t["leaf"])


def count_leaves(t):
    if isinstance(t, list):
        return sum(count_leaves(c) for c in t)
    return 1 if is_leaf(t) else 0


def kept_mask(orig, pruned, out):
    """Walk the original tree and a pruned copy (same shape; pruned sub-trees are None,
    dropped leaves are ' ... ' placeholders without "leaf") in parallel."""
    if isinstance(orig, list):
        for i, child in enumerate(orig):
            kept_mask(child, pruned[i] if isinstance(pruned, list) else None, out)
    elif is_leaf(orig):
        out.append("1" if is_leaf(pruned) else "0")


def mask_string(orig, pruned):
    out = []
    kept_mask(orig, pruned, out)
    if len(out) != count_leaves(orig):
        raise AssertionError("mask length differs from leaf count")
    return "".join(out)


def strip(t):
    if isinstance(t, list):
        return [strip(c) for c in t]
    if isinstance(t, dict):
        return {k: v for k, v in t.items() if k not in ("leading", "trailing")}
    return t


def feature_strings(sim, ast):
    """Sorted feature strings of one tree: featurize with a fresh vocabulary in init
    mode, then map every id back to its word (Vocab ids start at NUM_FEATURE_MIN + 1)."""
    saved = sim.vocab
    sim.vocab = sim.Vocab()
    try:
        ids = sim.collect_features_as_list(ast, True, False)[0]
        base = 1 + sim.config.NUM_FEATURE_MIN
        return sorted(sim.vocab.words[i - base] for i in ids)
    finally:
        sim.vocab = saved


def build_corpus(sim, corpus_path, work):
    """The reference corpus pipeline (setup with --corpus): featurize with vocabulary
    init, binary CountVectorizer, reload."""
    sim.vocab = sim.Vocab()
    features_path = os.path.join(work, sim.config.FEATURES_FILE)
    matrix_path = os.path.join(work, sim.config.TFIDF_FILE)
    sim.featurize_records_file(corpus_path, features_path)
    sim.counter_vectorize(features_path, matrix_path)
    return sim.load_all(matrix_path, features_path)


def intern_unknown_features(sim):
    """Deliberate deviation D7 (stable hashes, no vocabulary). The reference maps every
    feature missing from the corpus vocabulary to id NUM_FEATURE_MIN, so unrelated
    unknown features of a query and of a re-featurized pruned tree compare equal. Rust
    hashes keep them distinct. After the corpus is built, Vocab.get_index interns
    unseen words instead (fresh ids, never in the CountVectorizer vocabulary, so
    Phase I is unchanged); every other step runs unmodified reference code."""
    base = sim.config.NUM_FEATURE_MIN

    def get_index(self, word):
        if word not in self.vocab:
            self.words.append(word)
            self.vocab[word] = [0, len(self.vocab) + 1 + base]
        return self.vocab[word][1]

    sim.Vocab.get_index = get_index


def run_case(sim, vectorizer, counter_matrix, records, rec):
    cfg = sim.config
    query = dict(rec)
    query["features"] = sim.collect_features_as_list(query["ast"], False, False)[0]
    query["index"] = -1
    doc = sim.feature_list_to_doc(query)
    known = int(vectorizer.transform([doc]).sum())
    # Phase I with the reference function, uncapped; numpy's argsort tie order is not a
    # contract (note 08), so hits are re-sorted by (overlap desc, unit asc) and the
    # reference cap is checked explicitly instead.
    raw = sim.find_indices_similar_to_features(
        vectorizer, counter_matrix, [doc], len(records), cfg.MIN_SIMILARITY_SCORE
    )
    hits = []
    for j, s in raw:
        if float(s) != int(s):
            raise AssertionError(f"non-integral overlap {s}")
        hits.append((int(j), int(s)))
    hits.sort(key=lambda h: (-h[1], h[0]))
    if len(hits) > cfg.NUM_SIMILARS:
        raise Skip(f"{len(hits)} Phase I hits exceed NUM_SIMILARS; tie order would matter")
    # Rerank: find_similar's loop body, per hit, in the canonical hit order.
    prunes = []
    candidates = []
    for idx, overlap in hits:
        pruned = sim.prune_second_jd(query, records[idx])
        score = sim.find_similarity_score_features(query, pruned)
        prunes.append(
            {"unit": idx, "kept": mask_string(records[idx]["ast"], pruned["ast"]), "score": score}
        )
        if score > cfg.MIN_PRUNED_SCORE:
            candidates.append((records[idx], overlap, pruned, score))
    candidates = sorted(candidates, key=lambda v: v[3], reverse=True)
    clustered = sim.cluster_and_intersect(
        query, candidates, cfg.TOP_N, cfg.THRESHOLD1, cfg.THRESHOLD2
    )
    clusters = []
    for c in clustered:
        pruned_record, first_record, tuple_ = c[0], c[1], c[2:]
        clusters.append(
            {
                "members": [candidates[t][0]["index"] for t in tuple_],
                "kept": mask_string(first_record["ast"], pruned_record["ast"]),
            }
        )
    return {
        "id": rec["id"],
        "base": rec["base"],
        "mutation": rec["mutation"],
        "source": rec["source"],
        "ast": strip(rec["ast"]),
        "features": feature_strings(sim, rec["ast"]),
        "known_query_features": known,
        "hits": [[u, o] for u, o in hits],
        "prunes": prunes,
        "candidates": [[c[0]["index"], c[3]] for c in candidates],
        "clusters": clusters,
    }


def dump_lines(path, items):
    with open(path, "w", encoding="utf-8") as f:
        f.write("[\n")
        f.write(",\n".join(json.dumps(i, sort_keys=True, separators=(",", ":")) for i in items))
        f.write("\n]\n")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--similar", required=True)
    ap.add_argument("--sha256", required=True)
    ap.add_argument("--reference-commit", required=True)
    ap.add_argument("--corpus", required=True)
    ap.add_argument("--queries", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()

    sim = load_similar(args.similar, args.sha256)
    sink = io.StringIO()
    with contextlib.redirect_stdout(sink):
        vectorizer, counter_matrix, records = build_corpus(sim, args.corpus, args.work)
    intern_unknown_features(sim)
    corpus_in = read_jsonl(args.corpus)
    corpus_out = []
    for i, (rec, src) in enumerate(zip(records, corpus_in)):
        corpus_out.append(
            {
                "unit": i,
                "path": src["path"],
                "source": src["source"],
                "ast": strip(src["ast"]),
                "features": feature_strings(sim, src["ast"]),
                "n_leaves": count_leaves(src["ast"]),
            }
        )

    cases = []
    skipped = []
    for rec in read_jsonl(args.queries):
        try:
            with contextlib.redirect_stdout(sink):
                cases.append(run_case(sim, vectorizer, counter_matrix, records, rec))
        except Exception as e:  # recorded, never silently dropped
            skipped.append({"id": rec["id"], "reason": f"{type(e).__name__}: {e}"})
        sink.seek(0)
        sink.truncate()

    import numpy
    import scipy
    import sklearn

    cfg = sim.config
    meta = {
        "generator": "cargo run -p xtask -- gen-goldens",
        "reference_commit": args.reference_commit,
        "similar_py_sha256": args.sha256,
        "python": platform.python_version(),
        "packages": {
            "numpy": numpy.__version__,
            "scipy": scipy.__version__,
            "scikit-learn": sklearn.__version__,
        },
        "config": {
            "N_PARENTS": cfg.N_PARENTS,
            "N_SIBLINGS": cfg.N_SIBLINGS,
            "N_VAR_SIBLINGS": cfg.N_VAR_SIBLINGS,
            "NUM_SIMILARS": cfg.NUM_SIMILARS,
            "MIN_SIMILARITY_SCORE": cfg.MIN_SIMILARITY_SCORE,
            "MIN_PRUNED_SCORE": cfg.MIN_PRUNED_SCORE,
            "THRESHOLD1": cfg.THRESHOLD1,
            "THRESHOLD2": cfg.THRESHOLD2,
            "TOP_N": cfg.TOP_N,
        },
        "deviations": [
            "Phase I hits ordered by (overlap desc, unit asc); numpy argsort tie order is unspecified",
            "D7: features unknown to the corpus vocabulary keep distinct ids (reference: all collapse to id 10)",
        ],
        "n_corpus": len(corpus_out),
        "n_cases": len(cases),
        "n_cases_with_clusters": sum(1 for c in cases if c["clusters"]),
        "skipped": skipped,
    }
    os.makedirs(args.out, exist_ok=True)
    with open(os.path.join(args.out, "meta.json"), "w", encoding="utf-8") as f:
        json.dump(meta, f, indent=2, sort_keys=True)
        f.write("\n")
    dump_lines(os.path.join(args.out, "corpus.json"), corpus_out)
    dump_lines(os.path.join(args.out, "cases.json"), cases)
    print(
        f"goldens: {len(corpus_out)} corpus units, {len(cases)} cases "
        f"({meta['n_cases_with_clusters']} with clusters), {len(skipped)} skipped"
    )


if __name__ == "__main__":
    main()
