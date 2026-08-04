# Integration tests

- `tests/cli.rs` exercises end-to-end commands, configuration, JSON, diagnostics, and exit policy.
- `tests/golden.rs` checks stable human-readable output against `tests/golden/*.stdout`.
- `tests/corpus.rs` covers representative route and identifier policies through the library API.
- `tests/property.rs` differentially fuzzes `analyze_binary` against an independent, freshly-written
  interpreter of the parsed `Expr` (no shared code with `nfa.rs`/`analysis.rs`), plus checks
  reflexivity, overlap symmetry, and equivalence-vs-mutual-inclusion consistency. Runs under
  `cargo test --all-targets` with no extra setup; see the module doc comment in that file for the
  exact soundness guarantees each assertion does and doesn't provide.
- `tests/backend_agreement.rs` differentially fuzzes `AutomataBackend` against `MinimizedBackend`
  (see `src/minimize.rs`) directly against each other -- any disagreement in verdict or witness
  between the two backends is a real bug in one of them, caught automatically rather than by hand.
