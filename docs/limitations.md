# Limitations

## Backends

All five engines (`automata`, `minimized`, `derivatives`, `antimirov`,
`abstraction`) decide the same regular subset. They may hit
`max_product_states` or `timeout_ms` on different inputs; a completed
`YES`/`NO` from any backend is exact. Suffix-tracking patterns such as
`(a|b)*a(a|b){n}` can exhaust practical limits on every current engine —
see `bench/README.md` and `docs/backends.md`.

`abstraction` is a special case: it delegates to `automata` rather than
being a fifth independent decision procedure, so it is not currently
exercised by the `tests/backend_agreement.rs` differential suite the other
four are (see `docs/backends.md`). It only helps prove `YES` when the two
patterns share a large subexpression verbatim; on a real `NO`, or on
patterns with no shared structure (the nth-from-end family above, for
instance), it falls back to `automata` with a small, bounded constant
overhead and no algorithmic benefit. It also falls back unconditionally
under `--alphabet unicode`: its marker-substitution technique needs a
scalar value outside the configured alphabet to stand in for an abstracted
subexpression, and `--alphabet unicode`'s declared range is every valid
Unicode scalar value there is, leaving none free. Abstracting anyway would
let a marker collide with a real occurrence of that character in either
pattern, breaking the soundness argument the technique depends on (see
`docs/backends.md`, `abstraction`, step 0) — so under this alphabet
`abstraction` gets no benefit at all, not even on patterns that share a
large block verbatim.


## Correctness work

These items affect the declared semantic boundary and should be addressed before broadening claims:

1. Add Unicode property escapes with a pinned Unicode data version.
2. Add differential corpus tests against a second automata implementation for the common subset.
3. Add randomized algebraic tests for complement and symbolic partitioning across Unicode boundary values.
4. Define and test behavior for extremely large UTF-8 patterns and diagnostic truncation.
5. Add a stable JSON Schema file and compatibility tests across releases.
6. Add `AbstractionBackend` to the `tests/backend_agreement.rs` differential suite (it currently only has targeted unit tests in `abstraction.rs`, not the same randomized cross-backend fuzzing the other four engines get).
7. Make `max_product_states` cumulative across an `abstraction` query's internal rounds, the way `timeout_ms` already is, so a single query can't visit somewhat more than the configured cap in total (see `docs/backends.md`, Resource limits).

## Optional feature expansion

These do not invalidate current answers:

- search/substr matching as an explicit alternate mode
- case-insensitive matching with declared simple/full fold semantics
- non-capturing groups and syntax-only inline flags
- intersection or difference as emitted automata
- DOT visualization
- reusable content-addressed cache
- IDE diagnostics and CI policy files
- catastrophic-backtracking analysis as a separate engine-specific analysis

## Deliberate exclusions

Backreferences, recursion, conditionals, arbitrary lookaround, capture histories, locale-sensitive behavior, and engine-specific backtracking order are not regular-language features in the modeled sense. They will not be accepted merely for parser compatibility without a separate semantic design.

## Performance

Counted repetition is expanded structurally, so `{1000}` creates a large NFA. `max_repeat` guards this frontend cost. Product-state growth can still be exponential in the regex size; `max_product_states` and `timeout_ms` convert this into an honest `UNKNOWN`.

The implementation does not enforce a hard memory limit. The documented 512 MB target should be evaluated with corpus benchmarks and process-level CI limits.

Dense visitation tables, binary-search charset membership, and linear charset union reduce hot-path overhead in v0.1.1. Product keys still own sorted state vectors, and symbolic representative boundaries are still recomputed per expanded BFS state. Dynamic bitsets and partition caching remain benchmark-guided optimization work.
