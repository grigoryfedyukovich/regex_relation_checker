# Regex Relation Checker — Functional Specification

**Binary:** `regexrel`  
**Crate:** `regex-relation-checker`  
**Edition:** Rust 2021  
**Spec revision:** v0.1.2  

## 1. Purpose

`regexrel` decides exact full-string language relations for a documented
regular-expression subset: emptiness, overlap, left-to-right inclusion, and
equivalence. When a constructive counterexample or overlap witness exists, it
returns a shortest deterministic string (by Unicode scalar count).

Analysis is local, deterministic, and exact when the reachable state space is
exhausted. Resource limits produce `UNKNOWN`, never a guessed verdict.

## 2. Queries

Let `L(R)` be the full-string language of pattern `R` under the active alphabet
and configuration.

| Query | Meaning | `YES` |
|-------|---------|-------|
| `empty R` | `L(R) = ∅` | language is empty |
| `overlap A B` | `L(A) ∩ L(B) ≠ ∅` | some shared string |
| `includes A B` | `L(A) ⊆ L(B)` | every string of `A` is in `B` |
| `equivalent A B` | `L(A) = L(B)` | same language |

`includes` is directional: left language contained in right language.

## 3. Analysis backends

Four engines implement the same contract (`RelationBackend`). Select with
`--backend`. Completed `YES`/`NO` results must agree across backends; see
[docs/backends.md](docs/backends.md).

### 3.1 `automata` (default)

On-the-fly subset construction and a product BFS over pairs of NFA subsets.
No upfront determinization. Symbolic transitions use charset partitions;
lowest-codepoint representatives yield deterministic shortest witnesses.

### 3.2 `minimized`

1. Determinize each pattern to a DFA.  
2. Minimize (Moore partition refinement).  
3. For **equivalence**, if minimal DFAs are isomorphic (canonical BFS
   relabeling), return `YES` without product search.  
4. Otherwise product-search the minimized DFAs (overlap, includes, empty, or
   non-isomorphic equivalence).

### 3.3 `derivatives`

Brzozowski residual expressions with normalization (empty/epsilon elimination,
sorted deduplicated alternations, flattened concatenations). Product BFS over
pairs of residuals; nullability decides acceptance. Counted repetition is
expanded before derivation.

### 3.4 `antimirov`

Antimirov partial derivatives: each character produces a finite set of
residuals (linear form). Product BFS over pairs of linear forms; a form
accepts when any member is nullable. Language-equivalent residual theory to
Brzozowski with a set-shaped state key.

### 3.5 Limits

`max_product_states` (`--max-states`) and `timeout_ms` (`--timeout-ms`) bound
all backends. Exhausting either yields `UNKNOWN`.

## 4. Supported syntax

- Literals and escaped metacharacters  
- Concatenation and `a|b`  
- Groups `(...)`  
- `.` (newline membership configurable)  
- Classes `[abc]`, `[a-z]`, `[^...]`  
- `\n` `\r` `\t` `\0` `\d` `\D` `\w` `\W` `\s` `\S`  
- `*` `+` `?` `{m}` `{m,}` `{m,n}`  
- Optional outer `^` and `$` only  

**Not supported** (must error as `UNSUPPORTED`, never approximated):
backreferences, lookaround, word boundaries, Unicode properties, lazy or
possessive suffixes, inline flags, conditionals, nested anchors.

Run `regexrel syntax` for the installed contract text.

## 5. Semantics summary

- Full-string matching only (not search/substring).  
- Default alphabet ASCII `U+0000..U+007F`; `--alphabet unicode` expands to
  scalar values excluding surrogates.  
- In v0.1.x, `\d` `\w` `\s` remain ASCII-defined in both alphabet modes.  
- Witnesses: BFS shortest by codepoint length; ties broken by lowest scalar in
  each symbolic partition.  
- Every returned witness is replayed on both automata before emission.

Full detail: [docs/semantics.md](docs/semantics.md).

## 6. Configuration

Load order: defaults → optional `regexrel.toml` / `--config` → CLI overrides →
validate. Unknown TOML keys are errors.

| Key | CLI | Default | Role |
|-----|-----|---------|------|
| `alphabet` | `--alphabet` | `ascii` | ASCII vs Unicode scalars |
| `max_product_states` | `--max-states` | `100000` | product / determinize cap |
| `timeout_ms` | `--timeout-ms` | `5000` | analysis time budget |
| `max_repeat` | `--max-repeat` | `1000` | max explicit `{m,n}` bound |
| `dot_matches_newline` | `--dot-matches-newline` | `false` | `.` includes newline |
| `ci_exit_code` | `--ci-exit-code` | `10` | exit when `--fail-on` trips |

## 7. CLI surface

```text
regexrel [OPTIONS] <COMMAND>
regexrel [OPTIONS] <BENCHMARK_FILE>
```

Commands: `empty`, `overlap`, `includes`, `equivalent`, `syntax`.

Global options include `--backend`, `--json`, `--stats`, `--print-config`,
`--fail-on`, and the configuration flags above.

### Benchmark files

If the **last** argument is an existing readable file and is not a reserved
subcommand name, it is a benchmark file: strip lines whose first non-blank
character is `#`, shell-tokenize the rest (single/double quotes), and treat the
tokens as CLI arguments after any preceding flags.

```bash
regexrel --backend minimized path/to/case.md
```

Suite layout and runner: [bench/README.md](bench/README.md).

## 8. Output and exit codes

Text mode prints the verdict on the first line, then optional witness and
diagnostics. `--json` emits schema v1 ([docs/json-schema-v1.md](docs/json-schema-v1.md)).

| Code | Meaning |
|------|---------|
| 0 | Completed analysis (even if verdict is `NO`) unless `--fail-on` applies |
| 2 | Invalid input / config / syntax |
| 3 | Internal invariant failure |

`--fail-on never|no|unknown|non-yes` maps selected verdicts to `--ci-exit-code`.

## 9. Library API

```rust
use regexrel::{analyze_binary, Config, Query, Verdict};

let report = analyze_binary(
    Query::Equivalent,
    "a|b",
    "[ab]",
    &Config::default(),
)?;
assert_eq!(report.verdict, Verdict::Yes);
```

Backend-specific entry points: `analyze_binary_with_backend`,
`analyze_empty_with_backend`.

## 10. Correctness boundary

- A completed `YES` or `NO` is exact for the supported subset and configuration.
- `UNKNOWN` is incomplete analysis, not a third semantic truth value.
- `UNSUPPORTED` is rejected syntax or feature.
- Backends may differ in *resource* behaviour (which limit hits first) but not
  in completed verdicts.

## 11. Non-goals

No PCRE/JS/Rust-`regex` compatibility layer; no backtracking simulation; no
network services; no locale/case-folding completeness; no silent approximation
of unsupported constructs.

## 12. Document map

| Doc | Contents |
|------|----------|
| [README.md](README.md) | Tour, demos, configuration |
| [docs/backends.md](docs/backends.md) | Engines in depth |
| [docs/semantics.md](docs/semantics.md) | Language model |
| [docs/architecture.md](docs/architecture.md) | Module map |
| [docs/limitations.md](docs/limitations.md) | Known bounds |
| [docs/json-schema-v1.md](docs/json-schema-v1.md) | JSON contract |
| [bench/README.md](bench/README.md) | Benchmark suite |
