# Changelog

All notable changes are documented here.

## Unreleased

- Added `--draw <nfa|dfa|minimized>` (and the equivalent `draw` subcommand) to
  dump a Graphviz PDF of a single regex. `nfa` uses the Thompson ε-NFA,
  `dfa` the existing subset construction in `minimize::determinize`, and
  `minimized` Moore minimization in `minimize::minimize`. `--output` selects
  the file (default `<kind>.pdf`); `--emit-dot` or a `.dot` suffix writes
  Graphviz source without invoking `dot`.
- Fixed: `minimize.rs`'s `alphabet_partition` added a boundary after an
  interval's end only when `end < 0x10ffff`, so under `--alphabet unicode`
  an interval ending exactly at `U+10FFFF` got no trailing boundary. Because
  this function recovers each alphabet-partition range by pairing *adjacent*
  boundaries via `windows(2)`, the missing trailing boundary left the
  interval's own start boundary unpaired -- silently dropping the whole top
  range (and the DFA transitions built from it) from `MinimizedBackend`,
  independent of whether any other backend was affected. `.`, an unbounded
  upper-range class, or a negated class could all trigger it under
  `--alphabet unicode`, producing a `minimized`/`automata` verdict
  disagreement. `representative_symbols` (`minimize.rs`) and the sibling
  `representative_chars` copies in `analysis.rs`, `derivative.rs`, and
  `antimirov.rs` have the identical `end < 0x10ffff` guard but don't pair
  boundaries into ranges, so they were never affected. Fixed by always
  pushing the trailing boundary. Regression tests added in `minimize.rs`,
  plus 13 `bench/{yes,no}` cases (`equivalent`/`includes`/`overlap`, several
  near-U+10FFFF shapes) that fail against the pre-fix `minimized` backend.
- Fixed: `abstraction.rs`'s CEGAR common-subexpression reduction encoded
  its fresh marker symbols as ordinary `char` literals starting at
  `U+E000` (Private Use Area), sound only when that value -- and the range
  above it -- lies outside the configured alphabet. `--alphabet ascii`
  satisfies this; `--alphabet unicode` does not, since its declared scalar
  range (`U+0000..U+D7FF` ∪ `U+E000..U+10FFFF`) is every valid Unicode
  scalar value there is. A marker chosen from that alphabet is a real
  character, not a fresh one, and a real occurrence of it anywhere in
  either pattern collides with the substitution, breaking the `σ ∉ Σ`
  precondition the homomorphism argument depends on -- concretely, a
  shared subexpression abstracted to `U+E000` could become
  indistinguishable from an unrelated literal `U+E000` elsewhere in either
  pattern, and CEGAR's "abstract YES" is trusted with no independent
  concrete re-check. Fixed by adding `alphabet_has_room_for_markers`,
  checked once per `analyze_binary_expr` call: when the configured
  alphabet leaves no scalar value free for a marker, abstraction is
  skipped entirely and the call goes straight to the configured inner
  backend on the original, unabstracted patterns -- the same fallback
  path already used when there's nothing to abstract or refinement is
  exhausted. Regression tests added in `abstraction.rs`, including one
  that reproduces the unsound `YES` directly through
  `AbstractionBackend::analyze_binary_expr` with the gate removed, and
  confirms it's gone with the gate restored.
- Fixed: `apply_abstraction` matched candidate nodes against `AbstractionMap`
  entries via `structural_key`, but the two sides of that comparison were
  built inconsistently. `build_initial_map` discovers common subexpressions
  from *normalized* trees (`normalize`: alt branches sorted, concats
  flattened, both purely structural, language-preserving canonicalizations)
  and stores the normalized node as each map entry's target -- but
  `analyze_binary_expr` ran `apply_abstraction` against the *original*,
  un-normalized `left_expr`/`right_expr`. A subexpression written in a
  different-but-equivalent branch order on one side (`(b|a)` against a map
  entry discovered as `(a|b)`) never matched, so that occurrence silently
  kept its real, unabstracted form -- usually just forgoing the speedup
  (abstract NO still falls back to concrete analysis), but when combined
  with the `structural_key` span leak below, it could abstract one
  occurrence of a repeated subexpression and not the other, since only one
  occurrence's happened to line up. Separately, `structural_key` only
  formats `expr.kind` (never the whole `Expr`), so a node's *own* span was
  already excluded -- but `ExprKind::Concat`/`Alt`/`Repeat` all embed full
  child `Expr` values, and their span fields leaked back in through the
  derived `Debug` impl on those children, so two occurrences of the exact
  same subexpression at different byte offsets in the source -- the normal
  case for anything repeated more than once in a pattern -- produced
  different keys and silently failed to match each other too, independent
  of the normalize-vs-original issue. Fixed both: `analyze_binary_expr` now
  builds `left_n`/`right_n` once via `normalize` and runs `abstract_pair`
  against those instead of the originals, and `structural_key` now hashes a
  span-zeroed clone of the node (`zero_spans`) instead of the node itself.
  Together these let CEGAR abstract every occurrence of a shared
  subexpression regardless of branch order or repetition, which is what
  the module's own doc comment already claimed it did. Regression tests
  added in `abstraction.rs`, including one that fails to collapse a
  differently-ordered shared alternation pre-fix (falling back to a
  1262-state concrete product) and collapses it to 2 states post-fix.
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
