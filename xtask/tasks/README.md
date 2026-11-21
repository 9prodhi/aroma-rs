# Salesforce idiom suite (D14, tightened by D17)

Forty query → expected-idiom tasks drawn from three public Apex repositories at pinned
commits. `cargo run -p xtask -- idiom` (task T18) runs them; this directory holds only the
task data (task T19). The files double as Apex adapter regression fixtures.

## Third-party snippets

Task `query` snippets are short excerpts from pinned public repositories, not
original aroma-rs implementation code. Each task records its repository, commit
and source path (`source_file`/`path`, including component part paths where
applicable), so the source and applicable license can be traced. This directory
also contains LWC and metadata suites in `lwc/` and `sfmeta/`.

| Source repository | License / attribution |
|---|---|
| `trailheadapps/apex-recipes` | CC0-1.0 |
| `trailheadapps/lwc-recipes` | CC0-1.0 |
| `SalesforceFoundation/NPSP` | BSD-3-Clause — Copyright (c) Salesforce.org |
| `jongpie/NebulaLogger` | MIT — Copyright (c) Jonathan Gillespie |

Full NPSP BSD-3-Clause and NebulaLogger MIT notices are in
[THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md). CC0 sources require no notice.

Licenses for the additional pinned Salesforce corpus repositories are recorded in
[`xtask/corpora/salesforce.toml`](../corpora/salesforce.toml); consult each pinned
repository's license for its full terms and notices. The snippets retain their
source licensing; aroma-rs's Apache-2.0 license does not relicense third-party
excerpts. Evaluation checkouts are fetched separately and never vendored. Do not
execute corpus source files as part of evaluation.

## How a task is used (implemented by T18, `xtask/src/idiom.rs`)

1. Index the task's repository at `commit`, holding out the method at `source_file`,
   `source_lines` and every unit whose normalized body equals it (clone dedup by
   `body_hash`, D22). Tasks with `include_tests = true` hold out a test method and are run
   with tests indexed.
2. Parse `query` as an Apex snippet (T07 wrapping) and run it.
3. **Similar-code hit (release gate, D29):** one of the top-5 similar methods whose full
   source lines contain every `required` fragment; gate: hit@5 >= 60% of all tasks. The
   pruned-text hit@5 is reported, not gated. **Recommendation win:** a recommendation
   (`--experimental-recommend`) contains every fragment the query lacks while no top-5
   similar method hits; the win-rate is reported, not gated. T18 owns the exact scoring.
4. Fragments are matched with whitespace removed and case folded (Apex is
   case-insensitive).
5. v1.0 gate (D27): similar-code hit@5 ≥ 60% over the 40 tasks. The recommendation
   win-rate is reported, not gated; recommendations become default only at ≥ 60% wins.

## Pinned repositories

| Clone under `ref/pilot/` | Repository | Commit |
|---|---|---|
| `apex-recipes` | https://github.com/trailheadapps/apex-recipes | `eda6167c5126a34daf66d9e1371ff6957ae223bd` |
| `NebulaLogger` | https://github.com/jongpie/NebulaLogger | `b792ac8d8b4eb31b14a164a24951c3cb4125a616` |
| `NPSP` | https://github.com/SalesforceFoundation/NPSP | `f5072b44781e7f31cb396c6919cfa7d0ead0ec6a` |

`ref/pilot/` is git-ignored. To create the clones:

```bash
git clone https://github.com/trailheadapps/apex-recipes ref/pilot/apex-recipes
git -C ref/pilot/apex-recipes checkout --detach eda6167c5126a34daf66d9e1371ff6957ae223bd
git clone https://github.com/jongpie/NebulaLogger ref/pilot/NebulaLogger
git -C ref/pilot/NebulaLogger checkout --detach b792ac8d8b4eb31b14a164a24951c3cb4125a616
git clone https://github.com/SalesforceFoundation/NPSP ref/pilot/NPSP
git -C ref/pilot/NPSP checkout --detach f5072b44781e7f31cb396c6919cfa7d0ead0ec6a
```

## Task file schema (`apex/<id>.toml`, one task per file)

| Key | Type | Meaning |
|---|---|---|
| `id` | string | Equals the file stem: `<ar\|nl\|np><NN>-<slug>` (apex-recipes, NebulaLogger, NPSP). |
| `category` | string | One of the categories below. |
| `repo` | string | Repository URL, one of the three pinned above. |
| `commit` | string | The pinned 40-hex commit of `repo`. |
| `source_file` | string | Path of the held-out method's file inside the repository (may contain spaces). |
| `source_lines` | `[start, end]` | 1-based inclusive lines of the held-out method: the first line of its declaration (after annotations) through its closing brace. |
| `query` | `"""` string | The partial snippet: lines copied verbatim (indentation aside) from the method's first statements, in source order. Lines may be skipped (comments, or a clause such as `WITH SECURITY_ENFORCED` that the task expects back) but never edited. Bracket-balanced, no backslashes. |
| `required` | array of strings | Literal code fragments the expected extension must contain. Each occurs in the held-out method, not in the query, and **all of them together** occur in at least one other method of the repository (the answerability rule below), so a similar method can supply them. |
| `notes` | string | The idiom, in one sentence. |
| `include_tests` | bool, optional | `true` exactly when the held-out method is in a test class (file name contains `test`); omitted otherwise. |

Categories: `bulk-dml` (collect, then one bulk DML call), `soql-security` (`WITH USER_MODE` /
`WITH SECURITY_ENFORCED` / `stripInaccessible`), `trigger-guard` (trigger context and
recursion guards), `saveresult` (partial-success DML and `Database.SaveResult` handling),
`test-setup` (`@TestSetup` data and `Test.startTest`/`stopTest` test shape),
`error-handling` (savepoint/rollback, exception translation), `json` (serialize /
deserialize), `async` (queueable, batch, schedulable, `enqueueJob`), `other`.

## Authoring rules

- One task per file; only the TOML subset the validator parses: basic strings without
  escapes other than `\"` and `\\`, `"""` multi-line strings without backslashes,
  `true`/`false`, integer arrays and string arrays.
- Read the method in the clone at the pinned commit and copy the query lines; never type
  them by hand.
- Prefer short, idiom-level `required` fragments (`isSuccess()`, `Database.rollback(sp)`,
  `WITH USER_MODE`) over method-specific text.
- Every category keeps at least two tasks; the suite has exactly 40.
- Choose tasks by the answerability rule below, never by running `aroma` or `xtask idiom`
  (tasks must not be tuned toward the engine).

## Answerability rule (T21)

A task is answerable when at least one *unit* (method, constructor, trigger body or property
accessor) of the repository at the pinned commit contains every `required` fragment, other
than the held-out source unit and its exact clones. This is checked at method level, not file
level: sibling methods in the source file count, the source method itself does not.

- **Matching** follows `xtask/src/idiom.rs::squash`: whitespace removed, case folded. The
  validator additionally drops comments (strings are kept), so a fragment that only appears
  in a comment never makes a task answerable; this is stricter than the scorer.
- **Exact clones** are units whose text, after the same comment/whitespace/case
  normalisation, equals the source unit's (the scorer holds these out by `body_hash`).
- **Eligibility** mirrors the query filter: a task without `include_tests` only counts units
  whose path does not contain `test` (case-insensitive, the Apex adapter's `is_test` rule);
  a test task counts every unit. The 2000-leaf prune limit is not modelled.
- **Units** are found by a brace-matching scan over each `.cls`/`.trigger` file (read with
  `git cat-file --batch` at the pinned commit) with comments and string literals masked.
  A `{` opens a unit when the code since the previous `;`, `{` or `}` (its header) ends
  either in `get`/`set` (an accessor) or in a balanced `( … )` preceded by a name that is not
  a control keyword (`if`, `for`, `while`, `catch`, `switch`, `when`, …); the unit runs from
  the first code of its header (annotations included) to the matching `}`, and its body is
  not scanned for further units. For loops whose header contains `;` and blocks such as
  `try {`, `else {`, class bodies and collection initialisers therefore never open a unit.
  The source unit is the smallest unit of `source_file` covering `source_lines`.

`--verbose` prints, per task, how many other units hold every fragment and the first three.
A failing task reports how many eligible units hold each fragment on its own.

## Validation

Run from the workspace root. T18 parses the same files with toml::from_str (xtask/src/idiom.rs::load_tasks, test checked_in_task_files_parse); the Python validator is run by hand and in review.

```bash
sed -n '/^```python validate-tasks$/,/^```$/p' xtask/tasks/README.md | sed '1d;$d' | python3 -I - xtask/tasks/apex ref/pilot
```

It parses every task with a subset parser (cross-checked against `tomllib` on Python ≥ 3.11),
checks the schema, ids, pinned repo/commit pairs, `include_tests`, the 40-task size and the
category spread,
and, for every task whose clone exists at the pinned commit (read with `git show`, so the
working tree does not matter): the line range is a brace-balanced method from its header
to its closing brace, the query lines are an in-order copy of method lines, every
`required` fragment occurs in the method and not in the query, and the task is answerable
(rule above). Tasks whose clone is missing get the schema checks only and print `skip`.
Append `--partial` while authoring a subset (skips the size and spread checks) and
`--verbose` to list each task's supporting units. Exit status 0 means every check passed;
the last line is the summary.

```python validate-tasks
"""Validate the D17 idiom tasks: schema, spread, and (when the pinned clones exist)
held-out method range, query provenance, required-fragment provenance and method-level
answerability (T21)."""
import bisect
import os
import re
import subprocess
import sys

ARGS = [a for a in sys.argv[1:] if not a.startswith("--")]
TASKS = ARGS[0] if len(ARGS) > 0 else "xtask/tasks/apex"
CLONES = ARGS[1] if len(ARGS) > 1 else "ref/pilot"
PARTIAL = "--partial" in sys.argv[1:]  # skip the suite-size and category-spread checks
VERBOSE = "--verbose" in sys.argv[1:]  # print the supporting units of every task
EXPECTED_TASKS = 40
REPOS = {
    "https://github.com/trailheadapps/apex-recipes": ("apex-recipes", "eda6167c5126a34daf66d9e1371ff6957ae223bd"),
    "https://github.com/jongpie/NebulaLogger": ("NebulaLogger", "b792ac8d8b4eb31b14a164a24951c3cb4125a616"),
    "https://github.com/SalesforceFoundation/NPSP": ("NPSP", "f5072b44781e7f31cb396c6919cfa7d0ead0ec6a"),
}
CATEGORIES = {"bulk-dml", "soql-security", "trigger-guard", "saveresult", "test-setup",
              "error-handling", "json", "async", "other"}
KEYS = {"id", "category", "repo", "commit", "source_file", "source_lines", "query", "required", "notes"}
OPTIONAL = {"include_tests"}  # true when the held-out method is a test (T18 then indexes tests)

BLANK = re.compile(r"(?:[ \t]*(?:#[^\n]*)?\n)*")
KEY = re.compile(r"([A-Za-z_]+)[ \t]*=[ \t]*")
EOL = re.compile(r"[ \t]*(?:#[^\n]*)?(?:\n|$)")
GAP = re.compile(r"(?:\s|#[^\n]*)*")
BASIC = re.compile(r'"((?:[^"\\\n]|\\["\\])*)"')
INT = re.compile(r"-?[0-9]+")


def parse_value(text, pos):
    if text.startswith('"""', pos):
        end = text.find('"""', pos + 3)
        if end < 0:
            raise ValueError("unterminated multi-line string")
        body = text[pos + 3:end]
        if "\\" in body:
            raise ValueError("backslash in a multi-line string (escapes are not allowed here)")
        return (body[1:] if body.startswith("\n") else body), end + 3
    if text.startswith('"', pos):
        m = BASIC.match(text, pos)
        if not m:
            raise ValueError(f"bad string at offset {pos}")
        return re.sub(r'\\(["\\])', r"\1", m.group(1)), m.end()
    if text.startswith("[", pos):
        items, pos = [], pos + 1
        while True:
            pos = GAP.match(text, pos).end()
            if text.startswith("]", pos):
                return items, pos + 1
            value, pos = parse_value(text, pos)
            items.append(value)
            pos = GAP.match(text, pos).end()
            if text.startswith(",", pos):
                pos += 1
            elif not text.startswith("]", pos):
                raise ValueError(f"expected ',' or ']' at offset {pos}")
    for word, value in (("true", True), ("false", False)):
        if text.startswith(word, pos):
            return value, pos + len(word)
    m = INT.match(text, pos)
    if m:
        return int(m.group(0)), m.end()
    raise ValueError(f"unsupported value at offset {pos}")


def parse_toml(text):
    """The TOML subset the task files use: key = string | \"\"\"multi-line\"\"\" | int | bool | array."""
    out, pos = {}, 0
    while True:
        pos = BLANK.match(text, pos).end()
        if pos >= len(text):
            return out
        m = KEY.match(text, pos)
        if not m:
            raise ValueError(f"expected 'key =' at offset {pos}")
        value, pos = parse_value(text, m.end())
        m2 = EOL.match(text, pos)
        if not m2:
            raise ValueError(f"trailing text after {m.group(1)}")
        pos = m2.end()
        if m.group(1) in out:
            raise ValueError(f"duplicate key {m.group(1)}")
        out[m.group(1)] = value


def code_and_strings(s):
    """Split Apex text into (code without comments, code with string literals blanked)."""
    code, blanked, i, n = [], [], 0, len(s)
    while i < n:
        if s[i] == "'":
            j = i + 1
            while j < n and s[j] != "'":
                j += 2 if s[j] == "\\" else 1
            code.append(s[i:j + 1])
            blanked.append("''")
            i = j + 1
        elif s.startswith("//", i):
            j = s.find("\n", i)
            i = n if j < 0 else j
        elif s.startswith("/*", i):
            j = s.find("*/", i + 2)
            i = n if j < 0 else j + 2
            code.append(" ")
            blanked.append(" ")
        else:
            code.append(s[i])
            blanked.append(s[i])
            i += 1
    return "".join(code), "".join(blanked)


def norm(s):
    return " ".join(code_and_strings(s)[0].split()).lower()


def balanced(s):
    pairs, stack = {")": "(", "]": "[", "}": "{"}, []
    for c in code_and_strings(s)[1]:
        if c in "([{":
            stack.append(c)
        elif c in pairs:
            if not stack or stack.pop() != pairs[c]:
                return False
    return not stack


def git(clone, *args):
    r = subprocess.run(["git", "-C", clone, *args], capture_output=True, text=True)
    return r.returncode, r.stdout


def check_against_clone(t, clone, errors):
    start, end = t["source_lines"]
    code, text = git(clone, "show", f"{t['commit']}:{t['source_file']}")
    if code != 0:
        errors.append(f"{t['source_file']} not found at {t['commit']}")
        return
    lines = text.split("\n")
    if end > len(lines):
        errors.append(f"source_lines end {end} beyond file length {len(lines)}")
        return
    method = "\n".join(lines[start - 1:end])
    if "(" not in lines[start - 1] or not lines[end - 1].strip().endswith("}"):
        errors.append("source_lines do not start at a method header and end at its closing brace")
    if not balanced(method):
        errors.append("held-out method text is not brace-balanced")
    body = [ln.strip() for ln in lines[start:end]]
    want = [ln.strip() for ln in t["query"].split("\n") if ln.strip()]
    k = 0
    for ln in body:
        if k < len(want) and ln == want[k]:
            k += 1
    if k != len(want):
        errors.append(f"query line {k + 1} is not an in-order copy of a method line: {want[k]!r}")
    nm, nq = norm(method), norm(t["query"])
    for frag in t["required"]:
        f = " ".join(frag.split()).lower()
        if f not in nm:
            errors.append(f"required {frag!r} not in the held-out method")
        if f in nq:
            errors.append(f"required {frag!r} already in the query")
    check_answerable(t, clone, start, end, errors)


LEXEME = re.compile(r"'(?:\\.|[^'\\\n])*'?|//[^\n]*|/\*.*?(?:\*/|\Z)", re.DOTALL)
NOT_NEWLINE = re.compile(r"[^\n]")


def mask(s):
    """`s` with comments and string-literal contents replaced by spaces (same length, newlines kept)."""
    def blank(m):
        t = m.group(0)
        if t.startswith("'"):
            return t[0] + " " * (len(t) - 2) + t[-1] if len(t) > 1 else t
        return NOT_NEWLINE.sub(" ", t)
    return LEXEME.sub(blank, s)


def squash(s):
    """Comments dropped, whitespace removed, case folded (xtask/src/idiom.rs::squash)."""
    kept = LEXEME.sub(lambda m: m.group(0) if m.group(0).startswith("'") else " ", s)
    return "".join(kept.split()).lower()


CONTROL = {"if", "for", "while", "catch", "switch", "when", "do", "try", "else", "finally", "return", "new"}
NAME_BEFORE_PARENS = re.compile(r"([A-Za-z_][A-Za-z0-9_]*)\s*$")
ACCESSOR = re.compile(r"(?:^|\W)(?:get|set)$", re.IGNORECASE)


def unit_header(header):
    """True when the code before a '{' declares a method, constructor or trigger (a name
    that is not a control keyword, then a balanced parameter list) or a property accessor."""
    h = header.strip()
    if ACCESSOR.search(h):
        return True
    if not h.endswith(")"):
        return False
    depth, k = 0, len(h) - 1
    while k >= 0:
        depth += {")": 1, "(": -1}.get(h[k], 0)
        if depth == 0:
            break
        k -= 1
    name = NAME_BEFORE_PARENS.search(h[:k]) if k > 0 else None
    return bool(name) and name.group(1).lower() not in CONTROL


def method_ranges(text):
    """1-based inclusive (first, last) lines of every unit of an Apex file, by a brace-matching
    scan over the text with comments and strings masked: a unit starts at the first code of
    its header (annotations included; the header is the code since the previous ';', '{' or
    '}') and ends at the brace matching the '{' that follows the header. Unit bodies are not
    scanned for further units (Apex has no local or anonymous classes)."""
    m = mask(text)
    newlines = [k.start() for k in re.finditer("\n", m)]
    braces = [(k.start(), k.group(0)) for k in re.finditer(r"[;{}]", m)]
    out, seg, b = [], 0, 0
    while b < len(braces):
        i, c = braces[b]
        b += 1
        if c != "{":
            seg = i + 1
            continue
        header, seg = m[seg:i], i + 1
        if not unit_header(header):
            continue
        depth, j = 1, len(m) - 1
        while b < len(braces):
            k, d = braces[b]
            b += 1
            depth += {"{": 1, "}": -1}.get(d, 0)
            if depth == 0:
                j = k
                break
        first = i - len(header.lstrip())
        out.append((bisect.bisect_left(newlines, first) + 1, bisect.bisect_left(newlines, j) + 1))
        seg = j + 1
    return out


UNITS = {}


def repo_units(clone, commit):
    """[(path, first, last, squashed text, is_test)] of every unit of every .cls/.trigger file
    at `commit` (is_test: the path contains 'test', the Apex adapter's rule)."""
    key = (clone, commit)
    if key in UNITS:
        return UNITS[key]
    _, listing = git(clone, "ls-tree", "-r", "-z", "--name-only", commit)
    paths = [p for p in listing.split("\0") if p.lower().endswith((".cls", ".trigger"))]
    r = subprocess.run(["git", "-C", clone, "cat-file", "--batch"], capture_output=True,
                       input="".join(f"{commit}:{p}\n" for p in paths).encode())
    data, pos, units = r.stdout, 0, []
    for p in paths:
        eol = data.index(b"\n", pos)
        size = int(data[pos:eol].split()[2])
        text = data[eol + 1:eol + 1 + size].decode("utf-8", "replace")
        pos = eol + 1 + size + 1
        lines = text.split("\n")
        for first, last in method_ranges(text):
            body = "\n".join(lines[first - 1:last])
            units.append((p, first, last, squash(body), "test" in p.lower()))
    UNITS[key] = units
    return units


def check_answerable(t, clone, start, end, errors):
    """Answerability (T21): every required fragment occurs together in at least one unit the
    query can return other than the held-out source and its exact clones (same text after
    dropping comments and whitespace, case folded). Non-test tasks only see non-test units."""
    units = repo_units(clone, t["commit"])
    own = [u for u in units if u[0] == t["source_file"] and u[1] <= start and u[2] >= end]
    if not own:
        errors.append("no unit found by the brace scan covers source_lines")
        return
    source = min(own, key=lambda u: u[2] - u[1])
    frags = [squash(f) for f in t["required"]]
    eligible = [u for u in units if u[3] != source[3] and (t.get("include_tests", False) or not u[4])]
    support = [u for u in eligible if all(f in u[3] for f in frags)]
    if VERBOSE:
        print(f"     {t['id']}: {len(support)} other unit(s) hold every fragment"
              + "".join(f"; {u[0]}:{u[1]}" for u in support[:3]))
    if not support:
        per = ", ".join(f"{r!r}={sum(f in u[3] for u in eligible)}" for r, f in zip(t["required"], frags))
        errors.append("unanswerable: no other unit (outside the held-out source and its exact clones) "
                      f"holds every required fragment; units holding each: {per}")


def main():
    files = sorted(f for f in os.listdir(TASKS) if f.endswith(".toml")) if os.path.isdir(TASKS) else []
    failures, checked, ids, per_category = 0, 0, set(), {}
    for f in files:
        path = os.path.join(TASKS, f)
        with open(path, encoding="utf-8") as fh:
            text = fh.read()
        errors = []
        try:
            t = parse_toml(text)
        except ValueError as e:
            print(f"FAIL {f}: {e}")
            failures += 1
            continue
        if sys.version_info >= (3, 11):
            import tomllib
            if tomllib.loads(text) != t:
                errors.append("tomllib parses the file differently from the subset parser")
        if not KEYS <= set(t) <= KEYS | OPTIONAL:
            errors.append(f"keys {sorted((KEYS - set(t)) | (set(t) - KEYS - OPTIONAL))} missing or unexpected")
            print(f"FAIL {f}: " + "; ".join(errors))
            failures += 1
            continue
        if t["id"] != f[:-5]:
            errors.append("id must equal the file stem")
        if t["id"] in ids:
            errors.append("duplicate id")
        ids.add(t["id"])
        if t["category"] not in CATEGORIES:
            errors.append(f"unknown category {t['category']!r}")
        per_category[t["category"]] = per_category.get(t["category"], 0) + 1
        if REPOS.get(t["repo"], (None, None))[1] != t["commit"]:
            errors.append("repo/commit is not one of the pinned pairs")
        sl = t["source_lines"]
        if not (isinstance(sl, list) and len(sl) == 2 and all(isinstance(x, int) for x in sl) and 0 < sl[0] < sl[1]):
            errors.append("source_lines must be [start, end] with 0 < start < end")
        if not t["query"].strip() or not balanced(t["query"]):
            errors.append("query must be non-empty and bracket-balanced")
        if not t["required"] or not all(isinstance(r, str) and r.strip() for r in t["required"]):
            errors.append("required must be a non-empty list of non-empty strings")
        if not t["notes"].strip():
            errors.append("notes must not be empty")
        is_test = "test" in os.path.basename(t["source_file"]).lower()
        if t.get("include_tests", False) is not is_test:
            errors.append("include_tests = true is required exactly when the held-out method is in a test class")
        clone_name = REPOS.get(t["repo"], (None, None))[0]
        clone = os.path.join(CLONES, clone_name) if clone_name else None
        if not errors and clone and os.path.isdir(os.path.join(clone, ".git")):
            if git(clone, "cat-file", "-e", f"{t['commit']}^{{commit}}")[0] == 0:
                check_against_clone(t, clone, errors)
                checked += 1
            else:
                print(f"skip {t['id']}: {clone} lacks commit {t['commit']}")
        elif not errors:
            print(f"skip {t['id']}: clone {clone} not present")
        if errors:
            failures += 1
            print(f"FAIL {t['id']}: " + "; ".join(errors))
    if not files:
        failures += 1
        print(f"FAIL: no task files in {TASKS}")
    elif not PARTIAL and len(files) != EXPECTED_TASKS:
        failures += 1
        print(f"FAIL: {len(files)} task files, expected {EXPECTED_TASKS}")
    thin = sorted(c for c in CATEGORIES if per_category.get(c, 0) < 2)
    if files and not PARTIAL and thin:
        failures += 1
        print(f"FAIL: categories with fewer than 2 tasks: {thin}")
    print(f"{len(files)} tasks, {checked} checked against clones, {failures} failing; per category: "
          + ", ".join(f"{c}={per_category.get(c, 0)}" for c in sorted(CATEGORIES)))
    sys.exit(1 if failures else 0)


main()
```
