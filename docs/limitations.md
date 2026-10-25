# Limitations

## Backends

All eight engines (`automata`, `reversal`, `minimized`, `derivatives`,
`antimirov`, `antichain`, `hkc`, `abstraction`) decide the same regular subset. They may
hit `max_product_states` or `timeout_ms` on different inputs; a completed
`YES`/`NO` from any backend is exact. Suffix-tracking patterns such as
`(a|b)*a(a|b){n}` can exhaust the forward `automata` backend while becoming
small under the opt-in `reversal` backend; inputs hard in both directions can
still exhaust practical limits. See `bench/README.md` and `docs/backends.md`.

`reversal` is not an independent decision procedure: it wraps the same
subset-product search as `automata` with a bounded reverse-language probe.
The general `--orientation reversed|auto` layer can wrap any backend. Reversal
preserves all supported language relations and shortest witness length. For
`automata`, a dedicated reverse-DAG reconstruction also preserves the original
lexicographic tie-break. Other backends currently expose only their final
witness, not their equal-depth predecessor structure, so a reversed tied
witness can differ from the forward canonical same-length witness after it is
mapped back. This affects deterministic witness tie selection only, never the
verdict or witness validity. `--orientation auto` is a performance heuristic:
its 5% probes estimate continuation cost from budget pressure, transition
fanout, and start-frontier complexity, with a 20% hysteresis favoring forward.
A misprediction can waste time or states but cannot change a completed verdict;
explicit `forward`/`reversed` remain available for reproducible direction choice.

`abstraction` is another special case: it delegates to a configurable inner
engine rather than being an independent decision procedure, so it is
not currently exercised by the `tests/backend_agreement.rs` differential
suite the other six are (see `docs/backends.md`). It only helps prove `YES` when the two
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

`hkc` is a partial case in the other direction: independent for
`equivalent`/`includes` (its own combined-NFA bisimulation search, and
covered by `backend_agreement.rs`), but not for `overlap` (delegates
verbatim to `automata` — a disagreement there is definitionally
impossible) or `empty` (shares the same NFA-reachability check `automata`
and `antichain` already use). Its congruence-closure pruning is also the
naive version from the paper (O(|R|) per check), not the incremental one
(inverted index, pending-count queue) — measured against `minimized`, it
did not show the technique's expected speedup on any pattern family tried
so far, including a regression on this project's own suffix/window hard
suite (see `docs/backends.md`, `hkc`, "Performance").


## Correctness work

These items affect the declared semantic boundary and should be addressed before broadening claims:

1. Add Unicode property escapes with a pinned Unicode data version.
2. Add differential corpus tests against a second automata implementation for the common subset.
3. Add randomized algebraic tests for complement and symbolic partitioning across Unicode boundary values.
4. Define and test behavior for extremely large UTF-8 patterns and diagnostic truncation.
5. Add a stable JSON Schema file and compatibility tests across releases.
6. Add `AbstractionBackend` to the `tests/backend_agreement.rs` differential suite (it currently only has targeted unit tests in `abstraction.rs`, not the same randomized cross-backend fuzzing the other six engines get).
7. Make `max_product_states` cumulative across an `abstraction` query's internal rounds, the way `timeout_ms` already is, so a single query can't visit somewhat more than the configured cap in total (see `docs/backends.md`, Resource limits).
8. Implement `hkc`'s incremental congruence-closure check (inverted index + pending-count queue, per Bonchi & Pous) in place of the current naive O(|R|)-per-check version, and re-measure against `minimized`/`antichain` on the suffix/window hard suite.

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

The implementation does not enforce a hard memory limit. The documented 512 MB target should be evaluated with corpus benchmarks and process-level CI limits. `hkc`'s naive congruence-closure check is a concrete case where this matters in practice, not just in theory: on self-equivalence checks over the suffix/window family at moderate `n`, memory use grew large enough to be killed by the OS before either `max_product_states` or `timeout_ms` caught it, because per-node closure cost (and the `R` list backing it) can compound faster than the visited-state counter that those two limits watch.

Dense visitation tables, binary-search charset membership, and linear charset union reduce hot-path overhead in v0.1.1. Product keys still own sorted state vectors, and symbolic representative boundaries are still recomputed per expanded BFS state. Dynamic bitsets and partition caching remain benchmark-guided optimization work.
