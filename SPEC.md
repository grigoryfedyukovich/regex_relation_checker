# Regex Relation Checker — Functional Specification

**Repository:** `regex-relation-checker`  
**Binary:** `regexrel`  
**Primary language:** Rust 2021  
**Specification revision:** v0.1.1  
**Category:** exact regular-language relation checking with constructive witnesses

## 1. Purpose

`regexrel` decides formal-language relations for a documented regular-expression subset. It supports full-string emptiness, overlap, left-to-right inclusion, and equivalence queries. Whenever a constructive counterexample or overlap witness exists, the tool returns a shortest deterministic string.

The implementation is intentionally small and inspectable. It is not a compatibility layer for PCRE, Rust `regex`, JavaScript, or any other full regex engine.

## 2. Product goals

- Parse a practical regular subset into a language-neutral AST.
- Compile the AST to a Thompson epsilon-NFA.
- Decide emptiness, overlap, inclusion, and equivalence exactly when the reachable state space is exhausted.
- Generate shortest witnesses by Unicode scalar/codepoint count.
- Replay every emitted witness independently before returning it.
- Distinguish unsupported syntax, incomplete analysis, invalid input, and internal failures.
- Provide deterministic text output and versioned JSON output suitable for CI and coding agents.
- Remain local-only and require no network or external solver during normal analysis.

## 3. Explicit non-goals

The v0.1 line does not implement:

- backreferences, recursion, conditionals, or capture-history semantics;
- arbitrary lookaround, word boundaries, or Unicode property escapes;
- lazy or possessive quantifier behavior;
- substring/search matching;
- engine-specific backtracking order or catastrophic-backtracking analysis;
- locale-sensitive matching, case folding, or Unicode-complete shorthand classes;
- persistent caching or a database.

Unsupported constructs must never be silently approximated.

## 4. Primary users

- Developers validating small regex changes.
- Maintainers checking routing, identifier, filename, or policy patterns.
- Researchers and students studying automata-based language relations.
- CI systems consuming stable verdicts, witnesses, diagnostics, and JSON.
- Coding agents that need reproducible counterexamples rather than similarity scores.

## 5. Query model

Let `L(R)` denote the full-string language of regex `R` under the selected alphabet and configuration.

| Query | Formal question | `YES` means | Constructive result |
|---|---|---|---|
| `empty R` | `L(R) = ∅` | the language is empty | `NO` carries a string in `L(R)` |
| `overlap A B` | `L(A) ∩ L(B) ≠ ∅` | the languages overlap | `YES` carries a string in both |
| `includes A B` | `L(A) ⊆ L(B)` | every left string is accepted on the right | `NO` carries a left-only string |
| `equivalent A B` | `L(A) = L(B)` | both languages are equal | `NO` carries a left-only or right-only string |

`includes` is directional. `regexrel includes A B` does not also prove `L(B) ⊆ L(A)`.

## 6. Matching and alphabet semantics

### 6.1 Full-string matching

Every regex describes complete strings. Pattern `a` accepts exactly `"a"`, not any string containing `a`.

Outer `^` and `$` are accepted as documentary anchors and compile to epsilon transitions. Anchors in positions incompatible with full-string semantics are rejected.

### 6.2 Alphabet modes

- `ascii`: `U+0000..U+007F`.
- `unicode`: Unicode scalar values `U+0000..U+D7FF` and `U+E000..U+10FFFF`; surrogate code points are excluded.

Literals outside the configured alphabet produce `UNSUPPORTED` rather than being truncated or reinterpreted.

### 6.3 Shorthand classes

In v0.1.1, `\d`, `\w`, and `\s`, plus their complements, are ASCII-defined in both alphabet modes. The report records this assumption.

### 6.4 Dot

`.` matches every scalar in the configured alphabet except newline by default. `dot_matches_newline = true` includes newline.

## 7. Supported syntax

The installed syntax contract is printed by:

```bash
regexrel syntax
```

Supported forms:

- literals;
- concatenation;
- alternation `a|b`;
- plain groups `( ... )`;
- wildcard `.`;
- character classes `[abc]`, ranges `[a-z]`, and negation `[^0-9]`;
- escapes `\n`, `\r`, `\t`, `\0`, escaped metacharacters, and `\d \D \w \W \s \S`;
- quantifiers `*`, `+`, `?`, `{m}`, `{m,}`, and `{m,n}`;
- optional outer `^` and `$`.

Counted bounds are parsed as `usize`. Numeric overflow is a syntax error with a dedicated diagnostic. Explicit bounds larger than `max_repeat` produce `UNSUPPORTED` because the current Thompson builder expands counted repetition structurally.

## 8. Verdict taxonomy

The report verdict is one of:

- `YES`: the queried proposition was proved for the supported semantics.
- `NO`: the proposition was disproved; a witness is included when constructive evidence exists.
- `UNKNOWN`: analysis stopped at a configured resource limit before a proof or counterexample was found.
- `UNSUPPORTED`: at least one input requires semantics outside the supported subset.

`UNKNOWN` and `UNSUPPORTED` are never positive proofs.

A completed `YES` or `NO` is exact. The search is not depth-bounded. Counted regex repetition is part of the exact input language, not an approximation bound. The JSON field `semantics.bounded` therefore remains `false` for this backend. Operational state and timeout limits are reported separately and yield `UNKNOWN` when reached.

## 9. Witness contract

Witnesses are generated by breadth-first search over reachable subset or product states.

- Primary order: shortest Unicode scalar/codepoint length.
- Tie break: lexicographically lowest available scalar representatives.
- Relation labels:
  - `in_language`;
  - `in_both`;
  - `left_only`;
  - `right_only`.
- Every witness is replayed through the compiled NFA or NFAs before report emission.
- A replay mismatch is an internal error, never a user verdict.

## 10. Command-line interface

```bash
regexrel empty 'a|b'
regexrel overlap 'a+b' 'ab+'
regexrel includes '[a-z]+' '[a-z]{2,}'
regexrel equivalent 'a|b' '[ab]'
```

Useful global options:

```text
--config <PATH>
--alphabet <ascii|unicode>
--max-states <N>
--timeout-ms <N>
--max-repeat <N>
--dot-matches-newline <true|false>
--json
--stats
--print-config
--fail-on <never|no|unknown|non-yes>
--ci-exit-code <1..255>
--backend <automata|minimized>
```

### 10.1 Exit codes

- `0`: successful analysis invocation, regardless of semantic `YES` or `NO`, unless CI policy fires.
- `2`: invalid configuration or syntax input.
- `3`: internal invariant, witness-replay, or rendering failure.
- configured `ci_exit_code`: `--fail-on` policy triggered.

`--fail-on unknown` treats `UNSUPPORTED` like `UNKNOWN`.

### 10.2 Backends

`--backend` selects the analysis engine. Both implement the same documented
regular subset and are expected to agree on every verdict and witness;
`automata` (the default) does on-the-fly subset construction with a single
product-BFS over both patterns at once. `minimized` determinizes and
minimizes each pattern's automaton first, then for `equivalent` checks
isomorphism between the two minimized DFAs directly (no search needed when
it holds); for `overlap`/`includes`, and for `equivalent` when the
isomorphism check fails, it falls back to a product search over the
minimized DFAs. Report JSON's `backend.name` field always records which one
produced a given report (`"in_process_automata"` or `"minimized_dfa"`).

## 11. Configuration

Configuration is TOML. `regexrel` loads `./regexrel.toml` when present or the path supplied by `--config`.

```toml
alphabet = "ascii"
max_product_states = 100000
timeout_ms = 5000
max_repeat = 1000
dot_matches_newline = false
ci_exit_code = 10
```

Rules:

1. Unknown keys are errors.
2. File values are deserialized first.
3. CLI overrides are then applied.
4. The effective configuration is validated only after overrides, so a CLI flag can repair an invalid file value.
5. `--print-config` prints the effective validated configuration.

The public Rust API offers both `Config::load` for a standalone validated file and `Config::load_raw` for callers that need to apply overrides before `Config::validate`.

## 12. Architecture

```text
regex source
  → recursive-descent parser with byte spans
  → language-neutral AST
  → Thompson epsilon-NFA
  → canonical epsilon-closed subsets
  → unary or product BFS
  → shortest predecessor-path witness
  → independent NFA replay
  → text or JSON report
```

### 12.1 Frontend boundary

Parser cursor state and source-node identity do not escape the frontend. Analysis depends only on AST semantics and source spans used for diagnostics.

### 12.2 Character sets

Transition labels are normalized, sorted, disjoint inclusive scalar intervals. Membership uses binary search. Union merges already-normalized interval lists linearly. Intersection and subtraction preserve normalized order.

### 12.3 NFA subsets

Epsilon closures are canonical sorted, duplicate-free state-ID vectors. The implementation uses dense visitation tables instead of tree sets on the transition hot path. `Nfa::is_accepting` documents and debug-checks the canonical-subset invariant.

### 12.4 Product search

Binary BFS uses `(left_subset, right_subset)` keys. Local outgoing interval endpoints define symbolic classes. One lowest-scalar representative per class is sufficient because both transition functions are constant inside the class.

Timeout checks occur at BFS-state granularity to avoid clock polling for every symbolic character.

### 12.5 Backend boundary

`RelationBackend` isolates analysis from report assembly and witness replay. The default backend is in-process automata code. Tests may inject deterministic fake backends.

### 12.6 Persistence

v0.1.1 has no persistent cache. A future cache must include tool semantic version, complete configuration digest, query, pattern digest, and backend mode in its key.

## 13. Diagnostics and evidence

Every report provides:

1. a stable diagnostic identifier;
2. a concise message;
3. the affected input side when applicable;
4. source span and recovery hint for frontend failures;
5. effective semantic assumptions and resource limits;
6. a structured witness field when constructive evidence exists;
7. automata and search statistics.

The witness is the structured `evidence` for relation results; it is intentionally separate from the diagnostic object in JSON schema v1.

## 14. JSON report schema

`--json` emits `schema_version: "1"` with:

- tool and backend versions;
- query and verdict;
- optional witness and relation label;
- diagnostic ID, message, source information, and assumptions;
- NFA sizes, visited states, generated transitions, alphabet, and configured limits;
- phase timings;
- semantic mode.

Schema-v1 consumers must ignore unknown additive fields. Removing, renaming, or changing a field type requires a schema-version increment.

## 15. Timing contract

Reports distinguish:

- parsing;
- automata construction;
- backend graph search;
- witness extraction;
- witness validation;
- rendering;
- total.

`total_ms` is the sum of all six component fields. Backend time excludes separately measured witness extraction, preventing double counting.

To avoid rendering the report twice, text output is rendered once with a timing placeholder and finalized in place. JSON is serialized once and its two timing number fields are finalized in place. `rendering_ms` measures report serialization/template construction; terminal I/O and the tiny in-place timing patch are excluded.

## 16. Resource behavior

Defaults:

- `max_product_states = 100000`;
- `timeout_ms = 5000`;
- `max_repeat = 1000`;
- documented memory target below 512 MB for typical MVP inputs.

State explosion or timeout yields `UNKNOWN` with consumed-state and transition statistics. No partial traversal may be presented as a proof.

Large counted repetition can create large NFAs before product search. Reports include each NFA state count to make this visible.

## 17. Security and privacy

- Normal analysis performs no network access.
- Input remains local.
- No subprocess or shell interpolation is used by the default backend.
- Internal crash output prints a reproducible local command and states that no data was uploaded.
- Reports avoid unrelated source content.
- Both library and binary forbid unsafe Rust.

## 18. Testing strategy

- Unit tests: parser success/failure cases, overflow, interval algebra, NFA construction, timings, and rendering helpers.
- Golden tests: stable end-to-end text output.
- Differential tests: finite-language enumeration against concrete `Nfa::matches`.
- Algebraic tests: reflexivity, symmetry, mutual inclusion, and transitivity examples.
- Backend-boundary tests: deterministic fake outcomes and invalid-witness rejection.
- Regression tests: every confirmed bug receives a minimized fixture.
- Corpus tests: route and identifier patterns with expected relations and witnesses.
- Integration tests: JSON schema, timing totals, assumptions, TOML/CLI precedence, Unicode, CI exit policy, and diagnostics.
- CI: Linux and macOS; format, Clippy, and all targets.

## 19. Performance priorities

Implemented in v0.1.1:

- dense visitation tables for NFA closure/step instead of `BTreeSet`;
- binary-search charset membership;
- linear merge for normalized charset union;
- unary subset keys without a meaningless second vector;
- shared resource-limit result construction;
- state-granularity timeout polling;
- single-pass report rendering.

Deferred performance work:

- compact/dynamic bitset subset keys to eliminate per-product-state vector allocation;
- cached or globally refined symbolic alphabet partitions;
- benchmark-guided merging of unary and binary BFS implementations;
- process-level memory-budget enforcement.

These are optimization tasks. They do not weaken current completed verdicts.

## 20. Running examples

### Overlap

```text
$ regexrel overlap 'a+b' 'ab+'
YES
shortest witness: "ab"
```

### Inclusion failure

```text
$ regexrel includes '[a-z]+' '[a-z]{2,}'
NO
witness in left only: "a"
```

### Equivalence

```text
$ regexrel equivalent 'a|b' '[ab]'
YES
```

### Resource limit

```text
$ regexrel --max-states 1 equivalent a b
UNKNOWN
reason: analysis reached the configured product-state limit
```

## 21. Repository shape

```text
regex-relation-checker/
├── Cargo.toml
├── README.md
├── SPEC.md
├── CHANGELOG.md
├── LICENSE
├── src/
├── tests/
│   ├── golden/
│   ├── unit/
│   └── integration/
├── docs/
│   ├── tutorial.md
│   ├── semantics.md
│   ├── architecture.md
│   ├── limitations.md
│   ├── json-schema-v1.md
│   ├── evaluation.md
│   └── audit-resolution.md
├── examples/
└── .github/workflows/
```

## 22. Release criteria

A v0.1.x release is acceptable when:

- supported syntax and semantic boundaries are documented precisely;
- all specification examples are represented in tests;
- machine-readable output is versioned;
- `UNKNOWN` and `UNSUPPORTED` never masquerade as proofs;
- emitted witnesses are replayed;
- timing totals equal their reported components;
- TOML values and CLI overrides produce one validated effective configuration;
- Linux and macOS CI run format, Clippy, and tests;
- README, tutorial, specification, and limitations remain mutually consistent.
