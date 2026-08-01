# Integration tests

- `tests/cli.rs` exercises end-to-end commands, configuration, JSON, diagnostics, and exit policy.
- `tests/golden.rs` checks stable human-readable output against `tests/golden/*.stdout`.
- `tests/corpus.rs` covers representative route and identifier policies through the library API.
