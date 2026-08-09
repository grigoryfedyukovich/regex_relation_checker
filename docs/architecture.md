# Architecture

## Layers

```text
CLI / library API
    ↓
parser (hand-written) → AST
    ↓
Thompson NFA build          ─┐
    ↓                        │  derivatives also consume AST
RelationBackend              │
  • automata  (default)      │
  • minimized                │
  • derivatives ←────────────┘
    ↓
Report (text or JSON) + optional shortest witness
```

## Modules

| Module | Role |
|--------|------|
| `parser` | Supported subset → `Expr` AST; syntax help via `regexrel syntax` |
| `ast` | Language-neutral expression tree |
| `charset` | Interval sets for classes, partitions, complements |
| `nfa` | Thompson ε-NFA construction and helpers |
| `analysis` | `RelationBackend` trait, default **automata** product BFS, orchestration |
| `minimize` | **minimized** backend: determinize, Moore minimize, isomorphism / DFA product |
| `derivative` | **derivatives** backend: residual algebra + residual-pair product |
| `config` | TOML + CLI overrides; validated limits |
| `report` | Verdicts, witnesses, timings, JSON schema v1 |

## Backend contract

Every backend returns a `BackendResult` with status (`Found`, `Exhausted`,
`StateLimit`, `Timeout`), optional witness, and counters. The orchestration
layer maps that into a user-facing `Verdict` (`YES` / `NO` / `UNKNOWN` /
`UNSUPPORTED`), validates witnesses by independent replay, and fills timings.

AST-aware entry points (`analyze_binary_expr` / `analyze_empty_expr`) let the
derivatives backend avoid a pure-NFA path; other backends ignore the AST and
use the NFAs.

## Determinism

- Alphabet partitions are split at every relevant interval endpoint.
- The lowest Unicode scalar in each partition is explored first.
- Witnesses are shortest by codepoint count under that policy.

## Failure modes

| Situation | Verdict / exit |
|-----------|----------------|
| Unsupported syntax | `UNSUPPORTED` / input error |
| State or time limit | `UNKNOWN` |
| Completed search | `YES` or `NO` (exit 0 unless `--fail-on`) |
| Witness replay mismatch | internal error |

Details: [backends.md](backends.md), [semantics.md](semantics.md), [../SPEC.md](../SPEC.md).
