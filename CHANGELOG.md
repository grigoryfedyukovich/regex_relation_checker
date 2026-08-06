# Changelog

All notable changes are documented here.

## Unreleased

- Fixed: `ExprKind::Alt(vec![])` (an alternation with zero branches, constructible through the public AST types though never emitted by the parser) denoted `{ε}` in `Nfa::from_expr` but `∅` in the derivative backend's `from_expr`. `Nfa::from_expr` now leaves the fragment's start/end states unconnected for this case, matching the empty-`CharSet` precedent already used in the same builder and the standard convention that an empty union is the additive identity `∅`. Regression tests added in `nfa.rs` and `derivative.rs`.
- Added dead-branch pruning to the derivative backend's binary product search: once a product state's relevant residual(s) are permanently `∅` for the query being decided (see `derivative::is_dead_end`), the state is no longer expanded. This is a pure search-space reduction with no effect on any verdict; it mainly helps `overlap`/`includes` queries where one side is a large bounded structure that dies quickly against an unrelated pattern.
- Added a third `RelationBackend`: `DerivativeBackend` (`--backend derivatives`), which decides relations via Brzozowski derivatives on normalized residual expressions and a product BFS over residual pairs. Counted repetition is expanded before derivation; residual equality uses structural normalization (empty/identity elimination, sorted alternations). Cross-checked against the automata and minimized backends.

- Added a second `RelationBackend`: `MinimizedBackend` (`--backend minimized`), which determinizes and minimizes each pattern's automaton, decides `equivalent` via canonical-form isomorphism where possible, and falls back to a product search over the minimized DFAs for `overlap`/`includes` and for `equivalent` when isomorphism fails. `--backend automata` (the previous, still-default behavior) is unchanged.

## 0.1.1

- Fixed timing totals to include witness extraction without double-counting it inside backend time.
- Removed duplicate text rendering and JSON serialization; timing fields are finalized in place.
- Added a dedicated repetition-bound overflow diagnostic.
- Applied CLI overrides before effective configuration validation while preserving a validated standalone `Config::load` API.
- Centralized witness relation classification and labels.
- Added `max_repeat` and `dot_matches_newline` to report assumptions.
- Replaced tree-based NFA visitation with dense visitation tables.
- Added binary-search character-set membership and linear normalized union.
- Added regression tests for audit findings and updated the functional specification.
- Added a comprehensive tutorial covering semantics, witnesses, Unicode, configuration, JSON, CI, library use, and troubleshooting.
- Expanded parser, character-set, NFA, relation-law, backend-boundary, CLI, configuration, golden, and corpus tests.
- Added structural JSON assertions, stable text-output fixtures, and an executable tutorial script.

## 0.1.0

- Added hand-written parser with byte-span diagnostics.
- Added interval-based ASCII and Unicode-scalar alphabets.
- Added Thompson epsilon-NFA construction.
- Added exact on-the-fly BFS checks for emptiness, overlap, inclusion, and equivalence.
- Added shortest deterministic witnesses with NFA replay validation.
- Added first-class `UNKNOWN` and `UNSUPPORTED` reports.
- Added TOML configuration, JSON schema v1, CI exit policies, tests, and Linux/macOS CI.
