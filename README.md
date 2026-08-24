# regex-relation-checker

`regexrel` decides **exact** full-string language relations for a documented
regular-expression subset: emptiness, overlap, inclusion, and equivalence. When
a relation fails or overlap succeeds, it emits a **shortest** constructive
witness.

Five interchangeable analysis engines implement the same contract:

| `--backend` | Technique |
|-------------|-----------|
| `automata` (default) | On-the-fly NFA subset product BFS |
| `minimized` | Determinize → minimize → isomorphism or DFA product |
| `derivatives` | Brzozowski residuals + residual-pair product |
| `antimirov` | Antimirov partial derivatives (linear forms) + product BFS |
| `abstraction` | Common-subexpression abstraction + CEGAR, wrapping any of the above |

Completed `YES` / `NO` answers are exact for the supported subset. Hitting a
state or time limit yields **`UNKNOWN`**, never a guessed verdict. Details:
[docs/backends.md](docs/backends.md), [SPEC.md](SPEC.md).

## Two-minute demo

```bash
cargo build --release

./target/release/regexrel overlap 'a+b' 'ab+'
# YES
# shortest witness: "ab"

./target/release/regexrel includes '[a-z]+' '[a-z]{2,}'
# NO
# witness in left only: "a"

./target/release/regexrel equivalent 'a|b' '[ab]'
# YES
```

```bash
regexrel empty 'a|b'
regexrel match '(a|b)*a(a|b){40}' 'abbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
regexrel --backend derivatives --stats match '(a|b)*a(a|b){40}' 'abbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
regexrel syntax
regexrel --json equivalent '^a+$' 'aa*'
regexrel --stats --backend minimized --max-states 50000 overlap 'a.*z' 'ab+z'
regexrel --backend derivatives equivalent 'a*a*a*' 'a*'
regexrel --backend abstraction --stats equivalent '((a|b){25}c){10}x' '((a|b){25}c){10}x'
regexrel --draw nfa 'a+b'
regexrel --draw dfa --output dfa.pdf '(a|b)*a'
regexrel --draw minimized --emit-dot '[ab]+'
```

## Analysis engines

### `automata` (default)

Thompson ε-NFAs and a single on-the-fly product BFS over subset pairs. No
upfront determinization cost. Strong default for small and medium patterns.

### `minimized`

Fully determinizes and minimizes each side. **Equivalence** can finish via
minimal-DFA isomorphism without a product search; other queries (and
non-isomorphic equivalence) product-search the minimized DFAs.

### `match` (concrete membership)

`regexrel match <regex> <string>` tests full-string membership. Derivative backends walk only the residuals the input visits (memoized); this is where Brzozowski/Antimirov laziness shows, unlike relation queries.

### `derivatives`

Symbolic Brzozowski residuals with normalization. Often compact on star /
optional / repeated-atom chains; still exponential on suffix-tracking languages
such as `(a|b)*a(a|b){n}`.

### `antimirov`

Antimirov **partial** derivatives: each step yields a finite *set* of residuals
(a linear form). Same residual language as Brzozowski, but the set form maps
more directly to NFA states and can stay smaller under alternation.

### `abstraction`

Common-subexpression abstraction with counterexample-guided refinement
(CEGAR), wrapping one of the four engines above rather than replacing them.
Subexpressions shared *verbatim* by both patterns are replaced with a fresh
marker symbol before the inner engine runs, shrinking the pattern the inner
engine actually has to search. A resulting `YES` is validated by expanding
the abstract witness back through the real subexpression language before
being trusted; a `NO` or `UNKNOWN` triggers refinement and, if that still
doesn't settle it, falls back to running the inner engine on the original,
unabstracted patterns — so `abstraction` never does meaningfully worse than
its inner engine alone, and can do much better when the two patterns share a
large block:

```bash
./target/release/regexrel --backend abstraction --stats \
  equivalent '((a|b){25}c){10}x' '((a|b){25}c){10}x'
# YES
# states: left=1544, right=1544, product=2 / 100000; ...
```

versus 512 product states for the same query under `automata`. See
[bench/yes/mega_cegar_overlap__shared-core.md](bench/yes/mega_cegar_overlap__shared-core.md)
and the sibling `mega_cegar_*` files for the measured comparison.

`--abstraction-inner <automata|minimized|derivatives|antimirov>` picks which
engine `abstraction` runs for each CEGAR round and for the concrete
fall-back (default `automata`, matching `abstraction`'s historical
behaviour). It's ignored by every other `--backend` value:

```bash
./target/release/regexrel --backend abstraction \
  --abstraction-inner derivatives equivalent 'a(bc)*d' 'a(bc)*d'
```

Compare engines on the same input:

```bash
for b in automata minimized derivatives antimirov abstraction; do
  echo "== $b =="
  ./target/release/regexrel --backend "$b" --stats equivalent 'a+a+a+' 'a{3,}'
done
```

Full write-up: **[docs/backends.md](docs/backends.md)**.

## Drawing automata

`--draw` renders a **single** regex as a Graphviz automaton and dumps a PDF
(produced by `dot`). This is not a relation query: only one pattern is
accepted.

| Kind | Automaton | Implementation |
|------|-----------|----------------|
| `nfa` | Thompson ε-NFA | `Nfa::from_expr` |
| `dfa` | Subset-construction DFA | `minimize::determinize` |
| `minimized` | Moore-minimized DFA | `minimize::minimize` |

```bash
regexrel --draw nfa 'a+b'                      # writes nfa.pdf
regexrel --draw dfa --output dfa.pdf '(a|b)*a'
regexrel --draw minimized --emit-dot '[ab]+'   # DOT on stdout
regexrel --draw nfa --output graph.dot 'colou?r'
```

`--output` defaults to `<kind>.pdf`. A `.dot` suffix, or `--emit-dot`, skips
Graphviz so the command works without `dot` installed. `.svg` / `.png` are
handed to `dot` as `-Tsvg` / `-Tpng`. `--alphabet`, `--dot-matches-newline`,
`--max-states`, and `--timeout-ms` apply as they do for analysis.

## Benchmark files

If the last CLI argument is an existing readable file (and not a reserved
subcommand name), it is treated as a **benchmark**:

1. Drop lines whose first non-blank character is `#`.
2. Shell-tokenize the rest (`'` and `"` quoting).
3. Use those tokens as the CLI arguments after any preceding flags.

```bash
regexrel --backend derivatives bench/yes/equivalent__a-or-a_vs_a.md
```

### Suite layout (`bench/`)

| Path | Expected first line |
|------|---------------------|
| `bench/yes/*.md` | `YES` |
| `bench/no/*.md` | `NO` |

Prefixes: plain smoke tests; `heavy_` / `stress_` larger counts; `mega_`
hundreds of quantifiers or **exponential / window** languages (research
targets that may hit `UNKNOWN` under default limits). `mega_cegar_*`
specifically exercises `--backend abstraction`'s shared-subexpression
collapse — see [docs/backends.md](docs/backends.md)'s `abstraction` section.

Runner (from repo root):

```bash
./bench/run.sh
./bench/run.sh --keep-going
./bench/run.sh --keep-going "--backend minimized --max-states 1000000 --timeout-ms 60000"
./bench/run.sh --keep-going "--backend abstraction --abstraction-inner derivatives"
```

Outcomes: **OK** (verdict matches), **LIMIT** (`UNKNOWN` — resource bound),
**FAIL** (wrong definite verdict or crash). See [bench/README.md](bench/README.md).

## Semantics

Matching is **full-string** only: `a` is the language `{"a"}`, not a substring
search. Outer `^` / `$` are accepted as documentation of that boundary; anchors
elsewhere are unsupported.

Default alphabet: ASCII `U+0000..U+007F`. `--alphabet unicode` uses Unicode
scalars excluding surrogates. In v0.1.x, `\d` `\w` `\s` stay ASCII-defined in
both modes.

Witnesses are shortest by codepoint count; ties use the lowest scalar in each
symbolic transition class. This holds for `abstraction` too — a validated
witness is always re-expressed in terms of the original patterns, never left
as an abstract marker string.

See [docs/semantics.md](docs/semantics.md) and [SPEC.md](SPEC.md).

## Supported subset

- literals and escaped metacharacters  
- concatenation and `a|b`  
- groups `(...)`  
- `.` (newline configurable)  
- classes `[abc]`, `[a-z]`, `[^...]`  
- `\n` `\r` `\t` `\0` `\d` `\D` `\w` `\W` `\s` `\S`  
- `*` `+` `?` `{m}` `{m,}` `{m,n}`  
- optional outer `^` and `$`  

Not supported: backreferences, lookaround, inline flags, lazy/possessive
suffixes, Unicode properties, word boundaries, conditionals.  
`regexrel syntax` prints the installed contract.

## Configuration

`./regexrel.toml` or `--config PATH`. CLI overrides then validation.

```toml
alphabet = "ascii"
max_product_states = 100000
timeout_ms = 5000
max_repeat = 1000
dot_matches_newline = false
ci_exit_code = 10
```

```bash
regexrel --print-config
regexrel --max-states 1000000 --timeout-ms 60000 --backend derivatives ...
```

`max_product_states` and `timeout_ms` bound both the abstracted round and any
concrete fall-back round under `--backend abstraction` — they share the
overall budget rather than each getting a fresh one.

## Exit codes and CI

Completed analysis exits `0` even when the verdict is `NO`. Invalid input or
config exits `2`; internal failures exit `3`.

```bash
regexrel --fail-on no --ci-exit-code 17 equivalent 'a|b' '[ab]'
regexrel --fail-on unknown overlap "$LEFT" "$RIGHT"
regexrel --fail-on non-yes includes "$OLD" "$NEW"
```

## JSON

```bash
regexrel --json includes '[a-z]+' '[a-z]{2,}'
```

Schema v1: [docs/json-schema-v1.md](docs/json-schema-v1.md).

## Library

```rust
use regexrel::{analyze_binary, Config, Query, Verdict};

let report = analyze_binary(
    Query::Equivalent,
    "a|b",
    "[ab]",
    &Config::default(),
)?;
assert_eq!(report.verdict, Verdict::Yes);
# Ok::<(), regexrel::AnalyzeError>(())
```

`AbstractionBackend` is exported for programmatic use alongside the other
four (`AutomataBackend`, `MinimizedBackend`, `DerivativeBackend`,
`AntimirovBackend`), all implementing the shared `RelationBackend` trait:

```rust
use regexrel::{analyze_binary_with_backend, AbstractionBackend, AutomataBackend, Config, Query};

let backend = AbstractionBackend::with_inner(AutomataBackend);
let report = analyze_binary_with_backend(
    Query::Equivalent,
    "((a|b){25}c){10}x",
    "((a|b){25}c){10}x",
    &Config::default(),
    &backend,
)?;
# Ok::<(), regexrel::AnalyzeError>(())
```

`AbstractionBackend::new()` is a convenience shorthand for
`AbstractionBackend::with_inner(AutomataBackend)`.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
./check.sh          # local mirror of CI steps
./bench/run.sh --keep-going
```

## Documentation map

| Document | Topic |
|----------|--------|
| [SPEC.md](SPEC.md) | Functional contract |
| [docs/backends.md](docs/backends.md) | Engines in depth |
| [docs/architecture.md](docs/architecture.md) | Module layout |
| [docs/semantics.md](docs/semantics.md) | Language model |
| [docs/limitations.md](docs/limitations.md) | Known bounds |
| [docs/json-schema-v1.md](docs/json-schema-v1.md) | JSON schema v1 |
| [bench/README.md](bench/README.md) | Benchmark suite |

## Limitations

Models regular languages only — not engine-specific backtracking or captures.
Unicode shorthands and case-folding are intentionally incomplete. Large counted
repetitions expand under `max_repeat`. Hard suffix-tracking benchmarks may
return `UNKNOWN` until stronger algorithms are implemented. `abstraction`
only helps when both patterns literally share a subexpression (same AST
shape, same source text for that piece) — it does not detect semantically
equivalent-but-differently-written shared structure.

See [docs/limitations.md](docs/limitations.md).
