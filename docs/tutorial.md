# Tutorial: checking regex relations with `regexrel`

This tutorial explains the semantic model first, then shows command-line checks, witnesses, configuration, JSON automation, CI policies, and the Rust library API.

## 1. Build and run

Install a current stable Rust toolchain, then run from the repository root:

```bash
cargo build --release
cargo test --all-targets
```

The executable is `target/release/regexrel`. For the current shell:

```bash
export PATH="$PWD/target/release:$PATH"
regexrel --version
```

During development, `cargo run --` can replace `regexrel`:

```bash
cargo run -- overlap 'a+b' 'ab+'
```

## 2. Full-string semantics

Every regex denotes a language of complete strings. `regexrel` does not perform substring search.

Thus `a` denotes only the string `"a"`; it does not match `"cat"`, `"ba"`, or `"ab"`. To model arbitrary text around `a`, write `.*a.*` explicitly.

Outer anchors merely restate the same semantics:

```bash
regexrel equivalent '^ab$' 'ab'
# YES
```

Interior and nested anchors are unsupported.

## 3. Pick the correct query

| Command | Question | `YES` means | Witness |
|---|---|---|---|
| `empty R` | Is `L(R)` empty? | No string matches `R` | On `NO`, a string in `L(R)` |
| `overlap A B` | Do the languages intersect? | A string matches both | On `YES`, a string in both |
| `includes A B` | Is `L(A)` a subset of `L(B)`? | Every left string also matches right | On `NO`, a left-only string |
| `equivalent A B` | Are the languages equal? | Both accept exactly the same strings | On `NO`, a one-sided string |

Read `includes LEFT RIGHT` as: “Does the right regex accept everything accepted by the left regex?” The argument order is significant.

## 4. Basic analyses

### 4.1 Overlap

```bash
regexrel overlap 'a+b' 'ab+'
```

```text
YES
shortest witness: "ab"
```

A disjoint pair has no witness:

```bash
regexrel overlap 'admin/.*' 'public/.*'
# NO
```

### 4.2 Inclusion and compatibility

Suppose `[a-z]+` is an old validation rule and `[a-z]{2,}` is a proposed replacement:

```bash
regexrel includes '[a-z]+' '[a-z]{2,}'
```

```text
NO
witness in left only: "a"
```

The witness is a concrete backward-compatibility break. Reversing the check proves that the stricter language is contained in the old one:

```bash
regexrel includes '[a-z]{2,}' '[a-z]+'
# YES
```

### 4.3 Equivalence

```bash
regexrel equivalent 'a|b' '[ab]'
# YES
```

A failed equivalence check records the direction:

```bash
regexrel equivalent 'b' 'a|b'
# NO
# witness in right only: "a"
```

### 4.4 Emptiness

The empty regex accepts the empty string, so its language is not empty:

```bash
regexrel empty ''
# NO
# witness in language: ""
```

This class has no member because every alphabet character is in either `\d` or `\D`:

```bash
regexrel empty '[^\d\D]'
# YES
```

## 5. Why witnesses are deterministic

The search is breadth-first, so a witness has minimum Unicode-codepoint length. Equal-length choices are explored by increasing Unicode scalar value:

```bash
regexrel overlap '[ba]' '[ab]'
# YES
# shortest witness: "a"
```

The ordering is deterministic, not display-oriented. Negated classes may produce an escaped control character. JSON is often the clearest format for such witnesses.

Every emitted witness is replayed independently through both NFAs. A replay mismatch is an internal error rather than a semantic result.

## 6. Supported syntax

Print the installed contract:

```bash
regexrel syntax
```

The main forms are:

```text
abc             concatenation
ab|cd           alternation
(ab|cd)         plain grouping
.               wildcard
[abc] [a-z]     character classes and ranges
[^0-9]          negated class
\d \w \s        ASCII-defined shorthand classes
\D \W \S        complements in the selected alphabet
* + ?           standard repetition
{m} {m,} {m,n} counted repetition
^...$           optional outer anchors
```

Use single quotes in POSIX shells so metacharacters and backslashes arrive unchanged:

```bash
regexrel equivalent '\d+' '[0-9]+'
```

Backreferences, lookaround, inline flags, lazy or possessive suffixes, word boundaries, and Unicode property escapes are unsupported.

```bash
regexrel equivalent '(a)\1' 'aa'
# UNSUPPORTED
# reason: backreferences are not regular and are not supported
# input: left
```

`UNSUPPORTED` is a report verdict. Malformed supported syntax is an input error and exits with code `2`:

```bash
regexrel overlap '(' 'a'
echo $?
# 2
```

## 7. ASCII and Unicode alphabets

ASCII is the default alphabet, covering scalar values from U+0000 through U+007F. A Unicode literal therefore requires Unicode mode:

```bash
regexrel equivalent 'é+' 'éé*'
# UNSUPPORTED

regexrel --alphabet unicode equivalent 'é+' 'éé*'
# YES
```

Unicode mode covers Unicode scalar values and excludes surrogate code points. It performs neither normalization nor case folding. A precomposed accented character and a base character followed by a combining mark remain different strings.

In v0.1.1, `\d`, `\w`, and `\s` remain ASCII-defined in Unicode mode. Their uppercase complements are relative to the selected alphabet.

## 8. Dot and newline

By default, `.` excludes newline:

```bash
regexrel overlap '.' '\n'
# NO
```

Enable newline explicitly:

```bash
regexrel --dot-matches-newline true overlap '.' '\n'
# YES
# shortest witness: "\n"
```

The same option can be stored in TOML:

```toml
dot_matches_newline = true
```

## 9. Configuration

`regexrel` loads `./regexrel.toml` when present. Select another file with `--config`.

```toml
alphabet = "ascii"
max_product_states = 100000
timeout_ms = 5000
max_repeat = 1000
dot_matches_newline = false
ci_exit_code = 10
```

The file is deserialized first, CLI overrides are applied second, and the effective result is validated last. A flag such as `--ci-exit-code 17` can therefore repair an invalid `ci_exit_code = 0` in the file.

CLI flags override the file:

```bash
regexrel \
  --config examples/regexrel.toml \
  --alphabet unicode \
  --timeout-ms 10000 \
  equivalent 'é+' 'éé*'
```

Print the effective configuration without running a query:

```bash
regexrel --print-config
```

Or print it before an analysis:

```bash
regexrel --print-config overlap 'a+' 'aa*'
```

Unknown keys are errors, preventing misspelled settings from being silently ignored.

## 10. Resource limits and `UNKNOWN`

Product automata can grow exponentially. Two operational limits protect the process:

- `max_product_states`
- `timeout_ms`

If a limit is reached before exhaustive search completes, the result is `UNKNOWN`:

```bash
regexrel --max-states 1 equivalent 'a' 'b'
```

```text
UNKNOWN
reason: analysis reached the configured product-state limit
states: left=..., right=..., product=1 / 1; transitions=...; timeout=5000 ms
timing: ...
```

This is not an approximate proof. It states that the exact procedure stopped before proving a result.

A witness discovered before the limit remains exact. The initial state can reveal an empty-string overlap even with the minimum state budget:

```bash
regexrel --max-states 1 overlap 'a*' 'b*'
# YES
# shortest witness: ""
```

Inspect completed analyses with `--stats`:

```bash
regexrel --stats overlap 'a.*z' 'ab+z'
```

## 11. JSON automation

```bash
regexrel --json includes '[a-z]+' '[a-z]{2,}'
```

The report contains stable semantic fields and environment-dependent statistics:

```json
{
  "schema_version": "1",
  "query": "includes",
  "verdict": "NO",
  "witness": {
    "value": "a",
    "relation": "left_only",
    "codepoints": 1
  },
  "diagnostic": {
    "id": "RR_INCLUDE_COUNTEREXAMPLE"
  },
  "semantics": {
    "match_mode": "full_string",
    "bounded": false
  }
}
```

`semantics.bounded: false` means completed verdicts are exact. Counted repetition is part of the exact regex language; timeout and state limits instead produce `UNKNOWN` and appear under `statistics`.

Consumers should branch on `schema_version`, `verdict`, `diagnostic.id`, and `witness.relation`, not exact timing values. `statistics.timings.total_ms` is the sum of parsing, build, backend, extraction, validation, and rendering components.

With `jq`:

```bash
regexrel --json equivalent 'b' 'a|b' |
  jq '{verdict, witness: .witness.value, side: .witness.relation}'
```

Validate the schema before relying on a report shape:

```bash
schema=$(regexrel --json equivalent 'a' 'a' | jq -r .schema_version)
test "$schema" = "1"
```

## 12. CI policies and exit codes

A completed analysis exits `0` by default, including a mathematical `NO`. Use `--fail-on` to map selected verdicts to the configured CI exit code.

Reject failed equivalence:

```bash
regexrel \
  --fail-on no \
  --ci-exit-code 17 \
  equivalent "$OLD_REGEX" "$NEW_REGEX"
```

Require a completed inclusion proof:

```bash
regexrel --fail-on non-yes includes "$OLD_REGEX" "$NEW_REGEX"
```

Reject only inconclusive or unsupported analyses:

```bash
regexrel --fail-on unknown overlap "$LEFT_REGEX" "$RIGHT_REGEX"
```

`--fail-on unknown` covers both `UNKNOWN` and `UNSUPPORTED`. `--fail-on non-yes` covers `NO`, `UNKNOWN`, and `UNSUPPORTED`.

| Exit code | Meaning |
|---:|---|
| `0` | Analysis completed and policy did not trigger |
| `2` | Invalid syntax or configuration |
| `3` | Internal invariant failure |
| configured code | `--fail-on` policy triggered |

## 13. Common workflows

### Backward compatibility

```bash
regexrel includes "$OLD_REGEX" "$NEW_REGEX"
```

- `YES`: every old input remains accepted.
- `NO`: the witness is a concrete break.
- `UNKNOWN` or `UNSUPPORTED`: compatibility was not proved.

Check the reverse direction to detect broadening:

```bash
regexrel includes "$NEW_REGEX" "$OLD_REGEX"
```

Both directions returning `YES` is equivalent to an equivalence proof.

### Route conflicts

```bash
regexrel overlap 'admin/[a-z]+' 'public/[a-z]+'
```

A `YES` witness is an ambiguous route. `NO` proves disjointness under the documented semantics.

### Refactoring validation

```bash
regexrel equivalent 'item(s)?' 'items?'
# YES
```

Unlike sample-based tests, this proves equality over the full modeled languages.

## 14. Rust library API

```rust
use regexrel::{analyze_binary, Config, Query, Verdict};

fn main() -> Result<(), regexrel::AnalyzeError> {
    let report = analyze_binary(
        Query::Includes,
        "[a-z]+",
        "[a-z]{2,}",
        &Config::default(),
    )?;

    assert_eq!(report.verdict, Verdict::No);
    let witness = report.witness.expect("failed inclusion has a witness");
    assert_eq!(witness.value, "a");
    assert_eq!(witness.relation, "left_only");
    Ok(())
}
```

Emptiness uses a separate function:

```rust
use regexrel::{analyze_empty, Config, Verdict};

let report = analyze_empty(r"[^\d\D]", &Config::default())?;
assert_eq!(report.verdict, Verdict::Yes);
# Ok::<(), regexrel::AnalyzeError>(())
```

For experiments, implement `RelationBackend` and call `analyze_binary_with_backend` or `analyze_empty_with_backend`. Report assembly and independent witness replay remain outside the backend.

## 15. Add regression tests

Place a minimized test at the narrowest layer that detects the bug:

- parser classification and spans: `src/parser.rs`
- interval operations: `src/charset.rs`
- NFA acceptance: `src/nfa.rs`
- relation laws, bounds, and witnesses: `src/analysis.rs`
- process behavior and JSON: `tests/cli.rs`
- stable text output: `tests/golden.rs` and `tests/golden/*.stdout`
- realistic policies: `tests/corpus.rs`

Run individual integration targets while iterating:

```bash
cargo test --test cli
cargo test --test golden
cargo test --test corpus
```

Then run the full gate:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test --all-targets
```

Do not golden-test timings. Parse JSON and assert stable semantic fields instead.

## 16. Troubleshooting

### `UNSUPPORTED`

Run `regexrel syntax` and identify the unsupported construct. Do not erase a backreference or lookaround and assume the rewritten regex has the same language.

### `UNKNOWN`

Inspect `--stats`, then raise `--max-states` or `--timeout-ms` if the machine budget permits. Preserve the effective configuration in logs.

### Unexpected control-character witness

Use JSON so the character is escaped:

```bash
regexrel --json overlap '[^a]' '[^b]' | jq .witness
```

The ordering minimizes codepoints and scalar value, not visual readability.

### Shell quoting problems

Prefer single quotes on Linux and macOS:

```bash
regexrel equivalent '\d+' '[0-9]+'
```

Applications should invoke subprocesses with argument arrays rather than interpolating untrusted regexes into a shell command.

## 17. Proof boundary

A completed `YES` or `NO` is exact for the supported subset, selected alphabet, full-string mode, and configured dot behavior.

It does not prove engine behavior involving captures, backtracking order, catastrophic backtracking, backreferences, lookaround, case folding, Unicode normalization, or substring-search APIs.
