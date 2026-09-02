# Changelog

All notable changes are documented here.

## Unreleased

- Fixed: CEGAR `build_initial_map` treated the entire pattern as a common
  subexpression whenever both sides shared a root (including any identical
  pair). Round 1 replaced each side with a single fresh marker, so abstract
  YES was trivial but `expand_witness` had to solve the original language —
  Θ(2ⁿ) NFA subset construction for `(a|b)*a(a|b){n}`. That burned the
  shared `--timeout-ms` budget, after which `budget_config` refused the
  useful refinement round and reported `UNKNOWN` with `product=0` on
  `hard_cegar_overlap__nth26-self`. Roots of either pattern are no longer
  abstracted; shared *proper* subexpressions still are (the skeleton `σaτ`
  for that family, 4 product states). Regression tests in `abstraction.rs`.

- Improved: `Builder::repeat`'s Thompson construction for an unbounded
  counted repetition (`a{m,}`, including `a+` as `m == 1`) built `m`
  required copies of the body, then -- for the star tail -- built one
  *more* copy whose only job was to serve as the loop body, discarding the
  `m`-th (last required) copy's fragment as dead weight the moment the
  loop was through with it. The last required copy already sits at exactly
  the position the tail's loop body would need to be built at; looping
  directly on it (a back-edge from its own end to its own start, the usual
  Thompson `a+` shape) instead needs one fewer body-sized fragment, with
  the identical language either way. `a{3,}` drops from 11 states to 9;
  `a+` (`m == 1`) from 7 to 5. `a*` (`m == 0`, no required copy exists to
  reuse) is unaffected -- already the minimal construction for that case.
  Regression tests added in `nfa.rs` pinning both the new, smaller exact
  state count and the unaffected `m == 0` case.

- Added `--draw <nfa|dfa|minimized>` (and the equivalent `draw` subcommand) to
  dump a Graphviz PDF of a single regex. `nfa` uses the Thompson ε-NFA,
  `dfa` the existing subset construction in `minimize::determinize`, and
  `minimized` Moore minimization in `minimize::minimize`. `--output` selects
  the file (default `<kind>.pdf`); `--emit-dot` or a `.dot` suffix writes
  Graphviz source without invoking `dot`.

- Unified: `derivative.rs` and `antimirov.rs` each defined their own, fully
  independent copy of the residual-expression term algebra --
  `Reg`/`RegKind` (hash-consed via `Rc` for O(1) hash/equality), the smart
  constructors (`null`/`eps`/`atom`/`star`/`concat`/`alt`), `nullable`,
  `first_sets`, `reg_ord`, `from_expr`, and `expand_repeat` -- structurally
  identical (down to matching doc comments in most cases) but maintained
  separately. They had already drifted: `antimirov.rs`'s `Reg::alt` was
  still doing the older, slower sort-then-dedup, while `derivative.rs`'s
  copy had been upgraded to hash-based dedup-then-sort (a documented,
  deliberate optimization once hashing a `Reg` became O(1)) that was never
  ported over; `expand_repeat`'s `Vec::with_capacity` sizing hint had
  likewise only made it into one copy. Neither drift changed either
  backend's answers -- both were performance-only -- but nothing prevented
  a future one from being more than that, and two copies whose whole
  premise is independence have no way to catch it if it happens. Extracted
  the shared algebra into a new `residual.rs` module (both backends'
  algorithm-specific logic -- `Reg::derivative`'s single-residual step,
  `partial_der`/`LinearForm`'s set-of-residuals step, and each backend's
  own `*Interner` -- stays put; only the genuinely-identical term algebra
  moved) and added `dead_end_verdict`, factoring out the query-specific
  pruning decision `derivative::is_dead_end`/`antimirov::is_dead_end` each
  re-implemented identically over their own notion of "dead" (a lone `Reg`
  vs. an empty `LinearForm`). `--backend derivatives`/`--backend antimirov`
  bench runs are unchanged (0 FAIL either way, same OK/LIMIT split as
  before the refactor), as expected for a pure deduplication.

- Unified: four independent copies of the same alphabet-partitioning loop
  -- `representative_chars` in `analysis.rs`/`derivative.rs`/`antimirov.rs`
  and `representative_symbols`/`alphabet_partition` in `minimize.rs` --
  were the direct cause of the `alphabet_partition` boundary bug further
  down this changelog: three of the four copies happened to be correct for
  their own use case, so only the fourth's test coverage could have caught
  its bug, and didn't need to. Extracted one canonical
  `representative_chars`/`alphabet_partition` pair into `charset.rs`,
  generic over `T: Borrow<CharSet>` so callers holding either `&[CharSet]`
  (owned sets, as `derivative.rs`/`antimirov.rs`'s `first_sets` produce) or
  `&[&CharSet]` (references into longer-lived data, as `minimize.rs`'s DFA
  transitions are) can call it directly, with no extra allocation either
  way. `minimize.rs`, `analysis.rs`, `derivative.rs`, and `antimirov.rs`
  all now use the shared versions; their own copies are gone.

- Hardened: `Expr`/`ExprKind` (`ast.rs`) and `CharSet::from_u32_intervals`
  (`charset.rs`) were fully public, so an external library caller could
  hand-construct an AST or character set bypassing every invariant the
  parser normally enforces -- e.g. `ExprKind::Alt(vec![])` (an alternation
  with zero branches; the parser can't produce one, and not everything
  downstream necessarily expects one) or a `CharSet` built directly over
  the UTF-16 surrogate range or values past `U+10FFFF` (impossible via any
  `char`-based constructor, since `char` itself excludes both, but
  `from_u32_intervals` takes raw `u32` boundaries with no such check).
  `Expr::new` and `CharSet::from_u32_intervals` are now `pub(crate)`, and
  `ExprKind` is `#[non_exhaustive]`: external code can still parse a
  pattern and inspect the resulting `Expr` (match non-exhaustively, walk
  `.span`), but can no longer construct one bypassing the parser. Caught a
  real instance of exactly this on the first build: `tests/property.rs` (a
  separate compilation unit, and so "external" to the library crate in
  exactly the sense this change targets) had an intentionally-exhaustive
  match over `ExprKind` as its own independent reference interpreter;
  fixed with a documented wildcard arm that only ever fires if the library
  grows a variant that reference interpreter hasn't been taught to walk.

- Documented: a `{` that doesn't form a valid `{m}`/`{m,}`/`{m,n}` counted
  repetition (reversed bounds, a non-numeric body, unterminated) is a hard
  syntax error here, not a literal `{` -- unlike JS, Python, and Rust's own
  `regex` crate, which all fall back to treating unparseable `{...}` as a
  literal character. Noted as a deliberate divergence (catching a likely
  typo beats silently reinterpreting it) in `docs/semantics.md` and the
  `regexrel syntax` CLI text, rather than changed to match those engines.

- Added: `\b` inside a character class (`[\b]`) is now the literal
  backspace character, `U+0008` -- the reading every mainstream engine
  (JS, Python, PCRE, ICU, .NET, Rust's `regex` crate) gives it, since a
  zero-width word-boundary assertion has no meaning as one member of a
  character set. Previously rejected unconditionally: `\b` was only ever
  special-cased outside a class (where it remains the unsupported
  word-boundary assertion), so inside one it fell through to the generic
  "unsupported escape" error instead. `SYNTAX_HELP` updated to note the
  in-class reading explicitly.

- Fixed: `AbstractionBackend::analyze_binary_expr`'s shared-deadline helper,
  `budget_config`, computed each round's remaining budget as
  `config.timeout_ms.saturating_sub(elapsed_ms).max(1)` -- the `.max(1)`
  meant it could never actually report the budget as exhausted, only ever
  clamp down to a minimum of 1ms and hand that back as a normal, usable
  config. A call already past its deadline still got a (tiny but nonzero)
  budget and launched another round of real work regardless, and that round
  itself could take meaningfully longer than 1ms to notice its own budget
  was gone before the *next* `budget_config` call repeated the same
  mistake -- up to `MAX_REFINEMENT_ROUNDS + 1` times. The existing
  regression test for the sibling bug this shared-deadline mechanism was
  originally built to fix only asserted `elapsed < 400` for a 50ms budget,
  loose enough not to catch this. Fixed by changing `budget_config` to
  return `Option<Config>` -- `None` once `elapsed_ms >= config.timeout_ms`,
  forcing every call site to handle "stop now" explicitly rather than
  silently being handed a workable-looking config -- and adding
  `timeout_result`/`concrete_fallback` helpers so all six call sites (the
  main refinement loop, witness expansion, and four identical "give up,
  run the original patterns" exits, now de-duplicated into one) return a
  proper `BackendStatus::Timeout` immediately instead of launching more
  work. `max_product_states` remains per-round, not cumulative, across
  these same rounds -- already tracked as a separate, deliberately-deferred
  item in `docs/limitations.md` and out of scope here. Tightened the
  existing wall-clock test's threshold and added a deterministic regression
  test in `abstraction.rs` that calls `budget_config` directly with an
  artificially backdated start time (no timing race).

- Fixed: `nfa_match_input` (the default `RelationBackend::match_input`, used
  by `AutomataBackend`/`MinimizedBackend`/`AbstractionBackend` for
  `Query::Match`) checked `seen.len() >= config.max_product_states` *before*
  stepping on the current character, against the state count left over from
  the *previous* character -- so it rejected a character it had never even
  tried stepping on, regardless of whether that step would land on a brand
  new subset or revisit one already seen (e.g. a self-loop). At
  `--max-states 1`, the start subset alone already meets the cap, so *every*
  non-empty input was rejected before the first character was ever read
  (`match a a` → `UNKNOWN`, unconditionally); more generally, a pattern like
  `a*` matching a long run of `a`s failed as soon as the cap was reached by
  the *first* repetition, even though every later repetition revisits the
  exact same subset and costs nothing further. `derivative.rs`'s
  `ResidualInterner::intern` and `antimirov.rs`'s `LinearFormInterner::intern`
  already had the correct shape for this (a revisit of an already-known
  state always succeeds, even exactly at the cap; only a genuinely new state
  checks it) -- `nfa_match_input` now follows the same pattern: compute the
  next subset first, and only check the cap if that subset isn't already in
  `seen`. Regression tests added in `analysis.rs`.

- Fixed: `DerivativeBackend` and `AntimirovBackend`'s NFA-only entry points
  (`RelationBackend::analyze_binary`/`analyze_empty` -- distinct from
  `analyze_binary_expr`/`analyze_empty_expr`, which both backends genuinely
  implement) had no AST to derive from and always claimed
  `BackendStatus::Timeout` unconditionally, regardless of `config.timeout_ms`
  or whether any work was attempted at all. The normal CLI/library path
  (`analyze_binary_with_backend`) always calls `analyze_binary_expr`, so this
  was invisible there -- but any direct caller of the NFA-only trait methods
  on these two backends, or of `AbstractionBackend::analyze_binary` (which
  delegates straight to `self.inner.analyze_binary`, bypassing CEGAR
  entirely since there's no AST at that entry point either) with a
  derivatives/antimirov inner, got a plausible-looking `Verdict::Unknown`
  indistinguishable from a real search that ran and genuinely exhausted its
  budget. Fixed by adding `BackendStatus::Unsupported` -- a caller-error
  signal distinct from any resource limit, since retrying with a larger
  budget would never help -- and returning it from all four stubs instead.
  Updated the three exhaustive `BackendStatus` matches in `analysis.rs`
  (report rendering for match/binary/empty outcomes) and the one in
  `abstraction.rs` (`abstract_verdict`, where it's folded into the existing
  `StateLimit | Timeout => None` inconclusive case) to handle the new
  variant; the compiler's exhaustiveness check confirmed there were no
  others. Regression tests added in `derivative.rs`, `antimirov.rs`, and
  `abstraction.rs`.

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
