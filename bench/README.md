# Benchmarks

- `yes/` — theoretical answer is **YES**
- `no/`  — theoretical answer is **NO**
- Runner (from repo root, after `cargo build --release`):

```bash
./bench/run.sh
./bench/run.sh --keep-going
./bench/run.sh --keep-going "--backend derivatives --max-states 1000000 --timeout-ms 60000"
```

- Cross-backend comparison, for a single query or benchmark file, showing
  visited-state counts and timing side by side (see
  [docs/backends.md](../docs/backends.md) for what this is useful for —
  in particular, the `antimirov` vs `derivatives` state-count gap on
  `mega_equivalent__antimirov-block-position-*.md`):

```bash
./bench/compare_antimirov.sh bench/yes/mega_equivalent__antimirov-block-position-150.md
./bench/compare_antimirov.sh equivalent '(a|b|c)*' '(a|b|c)*'
```

## Runner outcomes

| Tag | Meaning |
|-----|---------|
| `OK` | First line of output matches the folder (`YES` / `NO`) |
| `LIMIT` | Engine returned `UNKNOWN` (state or time limit) — theoretical answer is still the folder label; target for better algorithms |
| `FAIL` | Wrong definite verdict, or non-zero tool exit |

Exit status: `0` all OK; `3` only LIMITs; `1` any FAIL.

## Classes

| Prefix | Role |
|--------|------|
| *(none)* | Small smoke tests |
| `heavy_` | Nested quantifiers, moderate size |
| `stress_` | Larger counts |
| `mega_` | Hundreds of quantifiers **or** exponential / tracking languages |

## Hard / exponential family

Patterns like `(a|b)*a(a|b){n}` (nth-from-end is `a`) have **Θ(2ⁿ)** residual/DFA
structure. They are intentional research targets: current backends
(`automata`, `minimized`, `derivatives`) may return `UNKNOWN` under default or
raised limits. A `LIMIT` result is not a bad benchmark — it is a goalpost for
new methods (suffix transducers, antichains, symbolic windows, etc.).

Also included: multi-window constraints, ternary alphabets, concatenated
trackers, sparse double markers, wide alphabets, structural variants of the
same language.

Engine notes: [docs/backends.md](../docs/backends.md).
