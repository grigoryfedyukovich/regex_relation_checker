# Analysis backends

`regexrel` implements five engines behind one CLI and library API. Select
with `--backend <name>`. Four are fully independent decision procedures and
must agree on every completed `YES` / `NO`; the fifth, `abstraction`, is a
CEGAR driver that delegates each abstract round *and* the concrete fall-back
to a configurable inner engine (default: `automata`; override with
`--abstraction-inner`). Its verdicts are only as independent as that
inner's — but it carries its own soundness argument for the fast path, and
its own witness-replay check, on top.
Disagreement among the four independent engines is a bug; `UNKNOWN` means a
resource limit was hit, not a soft “maybe”.

| Flag value | Module | Core technique |
|------------|--------|----------------|
| `automata` (default) | `analysis.rs` | On-the-fly NFA subset construction + product BFS |
| `minimized` | `minimize.rs` | Determinize → minimize → isomorphism or DFA product |
| `derivatives` | `derivative.rs` | Brzozowski residuals + product BFS on residual pairs |
| `antimirov` | `antimirov.rs` | Antimirov partial derivatives (linear forms) + product BFS |
| `abstraction` | `abstraction.rs` | Common-subexpression CEGAR; inner via `--abstraction-inner` |

Cross-checking the four independent engines is intentional: a defect that is
local to one implementation is far more likely to surface as a backend
disagreement than as a silent shared wrong answer. Integration tests in
`tests/backend_agreement.rs` exercise this across those four backends
(`automata`, `minimized`, `derivatives`, `antimirov`) on every `cargo test`
run. `abstraction` is not yet in that differential-fuzzing loop (it has its
own targeted unit tests in `abstraction.rs` instead, plus every returned
witness is replayed against the concrete, unabstracted automata before it
can reach the caller) — adding it to `backend_agreement.rs` is open
follow-up work.

---

## 1. `automata` — on-the-fly product (default)

**Pipeline**

1. Parse each pattern to an AST, compile to a Thompson ε-NFA.
2. Run a single BFS over *pairs* of deterministic subsets `(L, R)`.
3. Symbolic transitions: character-set partitions from both NFAs; explore the
   lowest codepoint representative of each partition first (deterministic
   shortest-witness order).
4. Accepting product states decide the query (empty / overlap / includes /
   equivalent) and reconstruct a witness along parent pointers.

**Strengths**

- No upfront determinization cost.
- Good default for small and medium patterns.
- Witness search is integrated with the decision procedure.

**Weaknesses**

- Product size tracks the *subset* lattice, not the minimal DFA.
- Suffix-tracking languages such as `(a|b)*a(a|b){n}` still explore Θ(2ⁿ)
  structure in the worst case.

**When to use**

Everyday checks, CI smoke tests, and any case where you want the least
surprising cost model.

---

## 2. `minimized` — determinize, minimize, then decide

**Pipeline**

1. Determinize each NFA fully (subset construction to completion).
2. Minimize each DFA (Moore-style partition refinement).
3. **Equivalence fast path:** if the two minimal DFAs are isomorphic under a
   canonical BFS relabeling, return `YES` *without* a product search
   (unique-minimal-DFA theorem).
4. Otherwise (not equivalent, or query is overlap / includes / empty): product
   BFS over the *minimized* DFAs — same shape as the automata backend, but on
   a smaller, total transition function (explicit dead sink).

**Strengths**

- Equivalence of identical or near-identical languages can short-circuit.
- Product starts from a reduced state space when minimization helps.
- Independent proof technique from the default engine.

**Weaknesses**

- Pays full determinization even when a shallow product would have sufficed.
- Languages with large minimal DFAs (again, nth-from-end style) make the
  determinize step dominate or hit `max_product_states` / timeout during
  determinization or product.

**When to use**

Equivalence-heavy workloads, cross-checks against `automata`, and patterns
that minimize well (long `a+a+…` chains, optional chains, factorable stars).

---

## 3. `derivatives` — Brzozowski residual product

**Pipeline**

1. Compile AST to a normalized residual algebra (not only via NFA): empty /
   epsilon / atoms / concat / alternation / star, with flattening, sorting,
   and deduplication.
2. Derive residuals symbolically with respect to alphabet partitions taken
   from the character sets appearing in the current residual pair.
3. Product BFS over pairs of normalized residuals; nullability decides
   acceptance.
4. Query-specific dead-end pruning drops product states that can no longer
   affect the verdict.

Counted repetition is expanded into concat / optional / star form before
derivation begins.

**Strengths**

- Algebraic simplification: nested stars, long `a*a*…`, and `a?a?…` chains
  often collapse to compact residuals.
- Third independent decision procedure for agreement testing.
- Can outperform automata on “syntax-heavy, language-simple” patterns.

**Weaknesses**

- Residual sets for suffix-tracking / sliding-window languages grow like the
  minimal DFA — derivatives do **not** magically beat Θ(2ⁿ) on
  `(a|b)*a(a|b){n}`.
- Normalization cost sits on the hot path; pathological alternation can
  produce large residual DAGs.

**When to use**

Cross-checks; patterns dominated by stars, optionals, and concatenations of
the same atom; experiments comparing residual size to NFA subset size
(`--stats`).

---


---

## 4. `antimirov` — partial derivatives (linear forms)

**Pipeline**

1. Compile AST to the same residual algebra used by the Brzozowski backend
   (normalized concat / alt / star, counted repeats expanded).
2. Derive **Antimirov partial derivatives**: each character yields a *finite
   set* of residuals (a linear form), not a single expression.
   - `∂ₐ(E|F) = ∂ₐ(E) ∪ ∂ₐ(F)`
   - `∂ₐ(EF) = ∂ₐ(E)·F ∪ (ν(E) ? ∂ₐ(F) : ∅)`
   - `∂ₐ(E*) = ∂ₐ(E)·E*`
3. Product BFS over pairs of linear forms. A form accepts when **any** member
   is nullable.
4. Empty linear form is the dead language on that side (same pruning rules as
   other backends).

**Why a fourth engine**

Antimirov's construction builds an NFA whose states are residual expressions;
the linear-form product is that idea applied to relation search. Under heavy
alternation, a set of small residuals can be cheaper than one large Brzozowski
term. Suffix-tracking languages such as `(a|b)*a(a|b){n}` remain exponential
in `n` for all current engines.

Concretely: `bench/yes/mega_equivalent__antimirov-block-position-{150,500}.md`
compare `(((a|b|c*)?){N})*` (N concatenated copies of an optional-a /
optional-b / any-run-of-c block, wrapped in an outer star) against the
language-equivalent `(a|b|c)*`. The block count is language-irrelevant —
the two sides are equivalent for any `N >= 1` — but not
representation-irrelevant: `derivatives` must track *which of the N block
positions* the current lap is in as part of one combined residual term, so
its visited-state count grows like `Θ(N)`; `antimirov`'s linear form has no
such positional bookkeeping and stays at a handful of states regardless of
`N`. Both backends still return `YES`; the difference only shows up in
`--stats` / `--json`, e.g.:

```bash
./bench/compare_antimirov.sh bench/yes/mega_equivalent__antimirov-block-position-150.md
./bench/compare_antimirov.sh --backends "derivatives antimirov" \
    equivalent '(((a|b|c*)?){500})*' '(a|b|c)*'
```

**When to use**

Cross-checks against `derivatives` and `automata`; patterns with wide
alternation where residual *sets* stay narrow; research comparisons on the
`mega_*` suite.

```bash
./target/release/regexrel --backend antimirov --stats equivalent '(a|b)*' '(a*b*)*'
./bench/run.sh --keep-going "--backend antimirov --max-states 1000000 --timeout-ms 60000"
```

---

## 5. `abstraction` — CEGAR common-subexpression reduction

**Pipeline**

0. Before anything else, check whether the configured `--alphabet` leaves
   any scalar value free for a marker (`alphabet_has_room_for_markers`).
   `--alphabet ascii` does (the Private Use Area sits above it); `--alphabet
   unicode` does not — its declared scalar range is every valid Unicode
   scalar value there is, so no `char` is left over for a marker to safely
   claim. If there's no room, skip straight to step 3's inner backend on
   the original, unabstracted patterns; steps 1–2 never run.
1. Walk both ASTs and collect every subexpression, keyed by a *structural*
   signature (kind + children, independent of source span). A candidate is
   worth abstracting once it's large enough (`MIN_ABSTRACT_SIZE`) and its
   signature appears in **both** patterns.
2. Rank candidates largest-first (ties broken by structural key, so the
   choice is deterministic across runs), keep up to `max_abstractions` of
   them, and assign each a fresh marker character from the Unicode Private
   Use Area.
3. Rewrite both ASTs, replacing every occurrence of each chosen
   subexpression — wherever it appears, in either pattern — with its
   marker. Run the configured *inner* backend (default `automata`; set
   with `--abstraction-inner`) on the resulting, much smaller pair.
4. **Sound YES** (Includes/Equivalent: abstract search exhausted with no
   counterexample; Overlap: abstract search found a match) is accepted
   immediately. For Overlap, every marker in the returned witness is first
   expanded back into a genuine shortest string drawn from the language of
   the subexpression it stands for (another inner-backend search, on just
   that subexpression) before the witness is trusted or returned; if a
   marker turns out to stand for a subexpression with an *empty* language,
   no substitution exists, and the round is treated as inconclusive rather
   than trusted.
5. **Abstract NO / UNKNOWN** is never trusted directly. The distinguishing
   counterexample (if any) tells the driver which marker(s) to expand back
   into their real subexpression, and it retries. After
   `MAX_REFINEMENT_ROUNDS`, or once nothing is left to expand, it falls
   back to running the same inner backend on the *original*, fully concrete
   patterns — the same procedure `--backend <inner>` would have run from
   the start.
6. All of the above — every abstracted round, every witness-expansion
   lookup, and the final concrete fallback — share **one** deadline,
   measured from the start of the call: each gets whatever is left of the
   caller's configured `timeout_ms`, not a fresh full budget per round.

**Soundness, briefly**

Replacing a subexpression `S` by a fresh marker `σ` (consistently, in both
patterns) is a language substitution `h` with `h(σ) = L(S)`. This gives
`L(A) = h(L(A'))` unconditionally, and `h` is monotonic under subset — so an
abstract `YES` on Includes/Equivalent always implies the concrete `YES`. An
abstract `YES` on Overlap needs one more step (the witness must be
expandable into a real string), which is exactly what step 4 checks. Both
steps depend on `σ ∉ Σ` (step 0) — a marker that's actually a member of the
configured alphabet is just an ordinary character, and a real occurrence of
it anywhere in either pattern breaks the substitution `h` is supposed to be.

**Strengths**

- When the two patterns share a large block verbatim, the reduced product
  can be *orders of magnitude* smaller than the concrete one. On
  `bench/yes/mega_cegar_overlap__shared-core.md` (`((a|b){25}c){10}x` vs.
  itself), `abstraction` visits 2 product states where `automata` visits
  512; several of the `mega_cegar_*` cases show a similar 100–250x
  reduction.
- Strictly bounded downside: every fallback path re-runs `automata`
  unmodified, so a case `abstraction` can't shortcut costs at most a small,
  now timeout-budget-respecting constant more than running `automata`
  directly.

**Weaknesses**

- Only the `YES` side benefits. A real `NO` is never provable in abstract
  space (an abstract `NO` is definitionally inconclusive), so refinement
  always runs to completion and the concrete fallback does the same work
  `automata` alone would have — plus the abstracted round(s)' overhead.
  Measured overhead on the `mega_cegar_*` `NO` cases is small (single-digit
  milliseconds) but nonzero.
- No shared structure, no benefit: patterns that are simply different in
  shape (the nth-from-end family `(a|b)*a(a|b){n}` vs. an unrelated
  right-hand side, for example) have nothing for this technique to grab
  onto, and `abstraction` degenerates to plain `automata` with no
  meaningful overhead.
- No benefit at all under `--alphabet unicode` (step 0): every call
  degenerates to plain `automata` (or whichever `--abstraction-inner`),
  including on patterns that share large blocks verbatim and would have
  gotten the full CEGAR speedup under `--alphabet ascii`. This is a real
  performance cost, not a rare corner case, for any Unicode-alphabet
  workload — the trade-off is deliberate (see `docs/limitations.md`).
- Unlike the other four engines, it is not currently exercised by
  `tests/backend_agreement.rs`'s differential fuzzing (see the note in the
  overview above).

**When to use**

Two patterns you already expect to share a large common block — generated
or templated regexes, a proposed edit to one branch of a larger shared
grammar, schema migrations that reuse most of a pattern and change one
piece. Comparing an unmodified pattern against a near-duplicate is the
sweet spot.

```bash
./target/release/regexrel --backend abstraction --stats \
  bench/yes/mega_cegar_overlap__shared-core.md
./bench/run.sh --keep-going "--backend abstraction --max-states 1000000 --timeout-ms 60000"
```

## Concrete membership: `match`
`regexrel match <regex> <string>` is full-string membership. The default path
simulates the NFA along the input. **Derivative** and **Antimirov** backends
override `match_input` to walk residuals character-by-character with memoization.

This is the setting where derivatives are classically strong: a regex whose
*language* automaton is enormous can still match a given string after only as
many residual steps as the string is long (e.g. `(a|b)*a(a|b){60}` vs 61
characters). Relation queries on the same pattern explore Θ(2ⁿ) product states.

## Resource limits (all backends)

| Config / flag | Meaning |
|---------------|---------|
| `max_product_states` / `--max-states` | Cap on visited product (or determinization) states |
| `timeout_ms` / `--timeout-ms` | Wall-clock analysis budget |

Hitting either yields **`UNKNOWN`**, never a guessed `YES`/`NO`. Witnesses
from completed runs are replayed on both sides before being returned.

`abstraction` can make several internal calls to its *inner* backend per
query (one per CEGAR round, plus witness expansion, plus a possible concrete
fallback); all of them share the *one* `timeout_ms` budget the caller
configured, measured from the start of the query — an individual round never
gets a fresh full timeout of its own. `max_product_states`, by contrast, is
currently applied fresh to each internal call rather than shared
cumulatively; in the worst case a query can visit somewhat more than
`max_product_states` product states in total across all of `abstraction`'s
rounds, though each round individually still respects the cap.

The inner engine is selected independently of the outer flag:

```bash
# historical default (inner = automata)
./target/release/regexrel --backend abstraction ...

# CEGAR over Brzozowski residuals
./target/release/regexrel --backend abstraction --abstraction-inner derivatives ...

# CEGAR over Antimirov / minimized
./target/release/regexrel --backend abstraction --abstraction-inner antimirov ...
./target/release/regexrel --backend abstraction --abstraction-inner minimized ...
```

Library users construct the driver directly:

```rust
use regexrel::{AbstractionBackend, DerivativeBackend, RelationBackend};

let backend = AbstractionBackend::with_inner(DerivativeBackend);
// or with an explicit budget:
let backend = AbstractionBackend::with_inner_and_budget(DerivativeBackend, 12);
```

---

## Choosing a backend (practical)

```text
small / unknown shape     →  automata (default)
equivalence of "same-ish" →  minimized
star / optional chains    →  derivatives  (and compare)
nth-from-end / windows    →  any may UNKNOWN; needs new methods
agreement testing         →  run all four independent engines, require identical verdicts
wide alternation residuals→  antimirov (compare to derivatives)
large shared block        →  abstraction (inner defaults to automata;
                             try --abstraction-inner derivatives/antimirov
                             when the residual space of the *shrunk* pair is small)
```

```bash
./target/release/regexrel --backend automata    --stats equivalent 'a+' 'aa*'
./target/release/regexrel --backend minimized   --stats equivalent 'a+' 'aa*'
./target/release/regexrel --backend derivatives --stats equivalent 'a+' 'aa*'
./target/release/regexrel --backend abstraction --abstraction-inner derivatives --stats equivalent 'a+' 'aa*'
```

Benchmark driver:

```bash
./bench/run.sh --keep-going "--backend derivatives --max-states 1000000 --timeout-ms 60000"
```

See [../bench/README.md](../bench/README.md) for suite layout and the meaning of
`OK` / `LIMIT` / `FAIL`.

---

## Research directions (hard suite)

The `mega_*` benchmarks include families that are decided in theory but may
return `LIMIT` (`UNKNOWN`) under practical bounds:

- suffix / window properties `(a|b)*a(a|b){n}`;
- multi-window and sparse double markers;
- ternary and wide alphabets;
- concatenated independent trackers.

Promising directions: suffix transducers / ring-buffer abstractions, antichain
algorithms for inclusion, symbolic (BDD) windows, and decomposition of
regular constraints into independent projections before product.
