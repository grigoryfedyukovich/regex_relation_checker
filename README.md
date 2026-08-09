# regex-relation-checker

`regexrel` decides **exact** full-string language relations for a documented
regular-expression subset: emptiness, overlap, inclusion, and equivalence. When
a relation fails or overlap succeeds, it emits a **shortest** constructive
witness.

Three interchangeable analysis engines implement the same contract:

| `--backend` | Technique |
|-------------|-----------|
| `automata` (default) | On-the-fly NFA subset product BFS |
| `minimized` | Determinize → minimize → isomorphism or DFA product |
| `derivatives` | Brzozowski residuals + residual-pair product |

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
regexrel syntax
regexrel --json equivalent '^a+$' 'aa*'
regexrel --stats --backend minimized --max-states 50000 overlap 'a.*z' 'ab+z'
regexrel --backend derivatives equivalent 'a*a*a*' 'a*'
```

## Analysis engines

### `automata` (default)

Thompson ε-NFAs and a single on-the-fly product BFS over subset pairs. No
upfront determinization cost. Strong default for small and medium patterns.

### `minimized`

Fully determinizes and minimizes each side. **Equivalence** can finish via
minimal-DFA isomorphism without a product search; other queries (and
non-isomorphic equivalence) product-search the minimized DFAs.

### `derivatives`

Symbolic Brzozowski residuals with normalization. Often compact on star /
optional / repeated-atom chains; still exponential on suffix-tracking languages
such as `(a|b)*a(a|b){n}`.

Compare engines on the same input:

```bash
for b in automata minimized derivatives; do
  echo "== $b =="
  ./target/release/regexrel --backend "$b" --stats equivalent 'a+a+a+' 'a{3,}'
done
```

Full write-up: **[docs/backends.md](docs/backends.md)**.

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
targets that may hit `UNKNOWN` under default limits).

Runner (from repo root):

```bash
./bench/run.sh
./bench/run.sh --keep-going
./bench/run.sh --keep-going "--backend minimized --max-states 1000000 --timeout-ms 60000"
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
symbolic transition class.

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
return `UNKNOWN` until stronger algorithms are implemented.

See [docs/limitations.md](docs/limitations.md).
