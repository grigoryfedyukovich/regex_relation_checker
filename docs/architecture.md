# Architecture

## Layers

```text
CLI / library API
    ↓
parser (hand-written) → AST
    ↓
Thompson NFA build          ─┐
    ↓                        │  derivatives also consume AST
RelationBackend               │  abstraction rewrites the AST, then
  • automata  (default)      │  delegates to the configured inner engine
  • minimized                │  (--abstraction-inner, default automata)
  • derivatives               │
  • antimirov   ←────────────┘
  • antichain
  • hkc          (overlap/empty delegate to automata / shared reachability)
  • portfolio    (separator decomposition → automata/HKC shared-budget fallback)
  • abstraction  (wraps any of the above)
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
| `draw` | Graphviz DOT/PDF rendering of NFA, DFA, or minimized DFA (`--draw`) |
| `derivative` | **derivatives** backend: residual algebra + residual-pair product |
| `antimirov` | **antimirov** backend: partial derivatives (linear forms) + product |
| `antichain` | **antichain** backend: antichain inclusion (individual `A`-states × minimal-antichain `B`-subsets) |
| `hkc` | **hkc** backend: bisimulation up to congruence over a combined NFA; `overlap`/`empty` delegate rather than implementing their own search |
| `portfolio` | **portfolio** backend: unique-separator decomposition plus shared-budget automata/HKC fallback |
| `abstraction` | **abstraction** backend: common-subexpression CEGAR reduction, delegates to a configurable inner engine (`--abstraction-inner`) |
| `config` | TOML + CLI overrides; validated limits |
| `report` | Verdicts, witnesses, timings, JSON schema v1 |

## Backend contract

`analysis::OrientedBackend` is an orthogonal execution wrapper around any of
these engines. `reversed` transforms both `Expr` and `Nfa` before delegation;
`auto` runs bounded forward/reverse probes for binary relations and continues
in the cheaper direction. The CLI applies this wrapper *after* selecting the
outer backend, so it also works around `AbstractionBackend` regardless of its
configured inner engine.

Every backend returns a `BackendResult` with status (`Found`, `Exhausted`,
`StateLimit`, `Timeout`), optional witness, and counters. The orchestration
layer maps that into a user-facing `Verdict` (`YES` / `NO` / `UNKNOWN` /
`UNSUPPORTED`), validates witnesses by independent replay, and fills timings.

AST-aware entry points (`analyze_binary_expr` / `analyze_empty_expr`) let the
derivatives backend avoid a pure-NFA path, let `portfolio` prove safe separator
decompositions from syntax, and let the abstraction backend rewrite the AST (structurally-shared subexpressions → fresh marker symbols)
before building NFAs at all; other backends ignore the AST and use the NFAs
directly.

## Determinism

- Alphabet partitions are split at every relevant interval endpoint.
- The lowest Unicode scalar in each partition is explored first.
- Witnesses are shortest by codepoint count under that policy.
- `abstraction`'s choice of which structurally-shared subexpressions to
  abstract is ranked largest-first with a deterministic tiebreak on
  structural key, independent of hash-map iteration order, so the same
  input always produces the same abstraction and the same witness.

## Failure modes

| Situation | Verdict / exit |
|-----------|----------------|
| Unsupported syntax | `UNSUPPORTED` / input error |
| State or time limit | `UNKNOWN` |
| Completed search | `YES` or `NO` (exit 0 unless `--fail-on`) |
| Witness replay mismatch | internal error |

Details: [backends.md](backends.md), [semantics.md](semantics.md), [../SPEC.md](../SPEC.md).
