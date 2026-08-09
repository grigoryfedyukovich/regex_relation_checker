# Analysis backends

`regexrel` implements three independent engines behind one CLI and library API.
Select with `--backend <name>`. All three decide the same regular subset and
must agree on every completed `YES` / `NO`. Disagreement is a bug; `UNKNOWN`
means a resource limit was hit, not a soft “maybe”.

| Flag value | Module | Core technique |
|------------|--------|----------------|
| `automata` (default) | `analysis.rs` | On-the-fly NFA subset construction + product BFS |
| `minimized` | `minimize.rs` | Determinize → minimize → isomorphism or DFA product |
| `derivatives` | `derivative.rs` | Brzozowski residuals + product BFS on residual pairs |

Cross-checking the three is intentional: a defect that is local to one
implementation is far more likely to surface as a backend disagreement than
as a silent shared wrong answer. Integration tests in `tests/backend_agreement.rs`
exercise this.

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

## Resource limits (all backends)

| Config / flag | Meaning |
|---------------|---------|
| `max_product_states` / `--max-states` | Cap on visited product (or determinization) states |
| `timeout_ms` / `--timeout-ms` | Wall-clock analysis budget |

Hitting either yields **`UNKNOWN`**, never a guessed `YES`/`NO`. Witnesses
from completed runs are replayed on both sides before being returned.

---

## Choosing a backend (practical)

```text
small / unknown shape     →  automata (default)
equivalence of "same-ish" →  minimized
star / optional chains    →  derivatives  (and compare)
nth-from-end / windows    →  any may UNKNOWN; needs new methods
agreement testing         →  run all three, require identical verdicts
```

```bash
./target/release/regexrel --backend automata    --stats equivalent 'a+' 'aa*'
./target/release/regexrel --backend minimized   --stats equivalent 'a+' 'aa*'
./target/release/regexrel --backend derivatives --stats equivalent 'a+' 'aa*'
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
