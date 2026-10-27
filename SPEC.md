# Regex Relation Checker — Functional Specification

**Binary:** `regexrel`  
**Crate:** `regex-relation-checker`  
**Edition:** Rust 2021  
**Spec revision:** v0.1.3  

## 1. Purpose

`regexrel` decides exact full-string language relations for a documented
regular-expression subset: emptiness, overlap, left-to-right inclusion, and
equivalence. When a constructive counterexample or overlap witness exists, it
returns a shortest deterministic string (by Unicode scalar count).

Analysis is local, deterministic, and exact when the reachable state space is
exhausted. Resource limits produce `UNKNOWN`, never a guessed verdict.

## 2. Queries

Let `L(R)` be the full-string language of pattern `R` under the active alphabet
and configuration.

| Query | Meaning | `YES` |
|-------|---------|-------|
| `empty R` | `L(R) = ∅` | language is empty |
| `overlap A B` | `L(A) ∩ L(B) ≠ ∅` | some shared string |
| `includes A B` | `L(A) ⊆ L(B)` | every string of `A` is in `B` |
| `equivalent A B` | `L(A) = L(B)` | same language |
| `match R S` | `S ∈ L(R)` | concrete string membership |

`includes` is directional: left language contained in right language.

## 3. Analysis backends

Nine engines implement the same contract (`RelationBackend`). Select with
`--backend`. Completed `YES`/`NO` results must agree across backends; see
[docs/backends.md](docs/backends.md).

### 3.1 Orientation layer

Search direction is independent of the selected backend via
`--orientation forward|reversed|auto` (default `forward`). `reversed` reverses
the regular-language AST (concatenation order recursively; outer `^`/`$` swap)
and the compiled NFA before calling the selected backend, then maps any witness
back to the original direction. Therefore the transformation applies equally
to NFA-based and AST/residual-based engines.

For binary queries, `auto` first runs a forward probe and, if inconclusive, a
reversed probe. Each probe gets 5% of the caller's state/time budget, capped at
512 states and 50 ms. A definitive probe result is returned immediately. If
both probes are inconclusive, `auto` computes a backend-neutral projected cost
from state/time budget pressure, transitions per visited state, and the NFA
start-frontier subset/branching complexity in each orientation. Reversal must
beat forward by at least 20%; otherwise hysteresis preserves the historical
forward direction. Probe work is charged to the same global budget. `empty`
and `match` stay forward under `auto`; explicit `reversed` still applies.

The `adaptive-v1` integer score is deterministic given probe counters:
`observed = 1000*states + 250*transitions`; `pressure` is the larger of
state/time budget utilization in per-mille units; and
`cost = observed*(1000+pressure)/1000 + 64*fanout_milli +
128*start_subset_product + 256*start_symbolic_branches`. State-limit probes
receive a 1.25x multiplier, timeout probes a 2x multiplier, and unsupported
probes are effectively infinite. A timeout on only one side is an immediate
preference for the other side. Otherwise reverse must have both a strictly
lower cost and at most 80% of the forward cost.

`automata` implements a specialized reverse-search hook that reconstructs from
the full shortest-path DAG and therefore preserves the original lexical
tie-break. Other backends expose only their final witness; reversal preserves
shortest codepoint length but may select a different same-length witness after
mapping back. Reports expose the active mode as `backend.orientation` and use
`semantics.witness_order` to distinguish the two contracts.

### 3.2 `portfolio` (hard-instance policy)

`portfolio` is a composition backend, not an independent proof procedure. For
`includes` and `equivalent`, it first looks for a common top-level singleton
separator character that no other component on either side can consume. Such a
separator occurs exactly once in every accepted word and makes concatenation
across the split injective. It can therefore decide `A#B ⊆ C#D` from the two
segment inclusions (with the empty-source product handled explicitly), and
equivalence from the two directional decomposed inclusions. Counterexamples
are composed from canonical segment witnesses and shortest segment members,
then compared by the normal global shortlex order. The decomposition attempt is
bounded to 8,192 states / 250 ms and shares the caller budget.

If decomposition is unavailable or inconclusive, `portfolio` spends at most
two thirds of the remaining budget on automata and gives all actually remaining
time/states to HKC. `overlap`, `empty`, and `match` keep the automata path. The
outer `--orientation` layer still wraps the portfolio normally.

### 3.3 `automata` (default)

On-the-fly subset construction and a product BFS over pairs of NFA subsets.
No upfront determinization. Symbolic transitions use charset partitions;
lowest-codepoint representatives yield deterministic shortest witnesses.

### 3.4 `reversal`

Runs a bounded product probe on the reversed languages for `includes` and
`equivalent`, then falls back to the same forward subset-product search as
`automata` with the remaining global budget. Reversal is opt-in: selecting
`automata` never runs this probe. `overlap`, `empty`, and concrete `match`
use the same paths as `automata`.

### 3.5 `minimized`

1. Determinize each pattern to a DFA.  
2. Minimize (Moore partition refinement).  
3. For **equivalence**, if minimal DFAs are isomorphic (canonical BFS
   relabeling), return `YES` without product search.  
4. Otherwise product-search the minimized DFAs (overlap, includes, empty, or
   non-isomorphic equivalence).

### 3.6 `derivatives`

Brzozowski residual expressions with normalization (empty/epsilon elimination,
sorted deduplicated alternations, flattened concatenations). Product BFS over
pairs of residuals; nullability decides acceptance. Counted repetition is
expanded before derivation.

### 3.7 `antimirov`

Antimirov partial derivatives: each character produces a finite set of
residuals (linear form). Product BFS over pairs of linear forms; a form
accepts when any member is nullable. Language-equivalent residual theory to
Brzozowski with a set-shaped state key.

### 3.8 `antichain`

Antichain inclusion (De Wulf et al., CAV'06). `A` is explored as individual
NFA states; `B` is tracked as a subset, pruned by subsumption against a
minimal antichain of previously-seen `(state, subset)` pairs. `overlap` and
`empty` use a plain individual-state search (no subsets on either side);
`includes` runs the antichain search directly; `equivalent` runs both
directional inclusions, interleaved. Independent decision procedure.

### 3.9 `hkc`

Bisimulation up to congruence (Bonchi & Pous, POPL'13). `A` and `B` are
combined into one NFA over a shared, disjoint state space; the search
tracks a pair of subsets `(X ⊆ A's states, Y ⊆ B's states)`, starting from
`(IA, IB)`, pruning a newly-reached pair without expanding it when it is
already implied ("congruent") — provably bisimilar via a union of
already-confirmed pairs. `equivalent` runs this directly; `includes`
reduces to it (`(IA ∪ IB, IB)` in the same combined space, since `L(left) ⊆
L(right)` iff `L(left) ∪ L(right) = L(right)`); `empty` delegates to the
same NFA-reachability check `automata`/`antichain` use; `overlap` delegates
to `automata` entirely (nothing to determinize away). Independent decision
procedure for `equivalent`/`includes`; not independent for `overlap`
(verbatim delegation) or `empty` (shared reachability check with two other
backends). Ships with the paper's *naive* congruence-closure check — see
`docs/backends.md` for measured performance characteristics, which did not
reproduce the technique's headline speedups against `minimized` on this
project's hard suite.

### 3.10 `abstraction`

Common-subexpression CEGAR reduction. Subexpressions occurring structurally
identically in both patterns are replaced by a shared fresh marker symbol;
the configured inner engine (`--abstraction-inner`, default `automata`) runs
on the reduced pair. A sound `YES` (Includes/Equivalent:
abstract search exhausted with no counterexample; Overlap: found match, with
every marker in the witness expanded back to a real substring) is returned
directly. An abstract `NO`/`UNKNOWN` triggers counterexample-guided
refinement, bounded by `MAX_REFINEMENT_ROUNDS`, after which analysis falls
back to the inner engine on the original, unabstracted patterns. Unlike the six
independent engines (`automata`, `minimized`, `derivatives`, `antimirov`,
`antichain`, `hkc`), this is not an independent decision procedure — it
delegates to whichever inner engine is configured — but every internal
round shares one `timeout_ms` budget, and every Overlap witness is expanded
and independently replay-validated before it can reach the caller (§5).

### 3.11 Limits

`max_product_states` (`--max-states`) and `timeout_ms` (`--timeout-ms`) bound
all backends. Exhausting either yields `UNKNOWN`.

## 4. Supported syntax

- Literals and escaped metacharacters  
- Concatenation and `a|b`  
- Groups `(...)`  
- `.` (newline membership configurable)  
- Classes `[abc]`, `[a-z]`, `[^...]`  
- `\n` `\r` `\t` `\0` `\d` `\D` `\w` `\W` `\s` `\S`  
- `*` `+` `?` `{m}` `{m,}` `{m,n}`  
- Optional outer `^` and `$` only  

**Not supported** (must error as `UNSUPPORTED`, never approximated):
backreferences, lookaround, word boundaries, Unicode properties, lazy or
possessive suffixes, inline flags, conditionals, nested anchors.

Run `regexrel syntax` for the installed contract text.

## 5. Semantics summary

- Full-string matching only (not search/substring).  
- Default alphabet ASCII `U+0000..U+007F`; `--alphabet unicode` expands to
  scalar values excluding surrogates.  
- In v0.1.x, `\d` `\w` `\s` remain ASCII-defined in both alphabet modes.  
- Witnesses are shortest by codepoint length. In forward orientation (and the
  automata reverse hook), ties are broken by lowest scalar in each symbolic
  partition. Generic reversed/auto wrappers around other backends may choose a
  different deterministic same-length witness after mapping back; this is
  reported in `semantics.witness_order`.  
- Every returned witness is replayed on both automata before emission.

Full detail: [docs/semantics.md](docs/semantics.md).

## 6. Configuration

Load order: defaults → optional `regexrel.toml` / `--config` → CLI overrides →
validate. Unknown TOML keys are errors.

| Key | CLI | Default | Role |
|-----|-----|---------|------|
| `alphabet` | `--alphabet` | `ascii` | ASCII vs Unicode scalars |
| `max_product_states` | `--max-states` | `100000` | product / determinize cap |
| `timeout_ms` | `--timeout-ms` | `5000` | analysis time budget |
| `max_repeat` | `--max-repeat` | `1000` | max explicit `{m,n}` bound |
| `dot_matches_newline` | `--dot-matches-newline` | `false` | `.` includes newline |
| `ci_exit_code` | `--ci-exit-code` | `10` | exit when `--fail-on` trips |

`--orientation` is a CLI/library execution policy rather than a TOML semantic
configuration key; its default is `forward`.

## 7. CLI surface

```text
regexrel [OPTIONS] <COMMAND>
regexrel [OPTIONS] <BENCHMARK_FILE>
```

Commands: `empty`, `match`, `overlap`, `includes`, `equivalent`, `syntax`,
`draw`.

`--draw <nfa|dfa|minimized>` (equivalent to the `draw` subcommand) renders
exactly one regex as a Graphviz automaton and dumps a PDF via `dot`. `nfa`
uses the Thompson construction, `dfa` the existing subset construction, and
`minimized` Moore minimization. `--output PATH` selects the file (default
`<kind>.pdf`); `--emit-dot` prints Graphviz source instead of invoking `dot`.

Global options include `--backend`, `--orientation`, `--json`, `--stats`, `--print-config`,
`--fail-on`, `--draw`, `--output`, `--emit-dot`, and the configuration flags
above.

### Benchmark files

If the **last** argument is an existing readable file and is not a reserved
subcommand name, it is a benchmark file: strip lines whose first non-blank
character is `#`, shell-tokenize the rest (single/double quotes), and treat the
tokens as CLI arguments after any preceding flags.

```bash
regexrel --backend minimized path/to/case.md
```

Suite layout and runner: [bench/README.md](bench/README.md).

## 8. Output and exit codes

Text mode prints the verdict on the first line, then optional witness and
diagnostics. `--json` emits schema v1 ([docs/json-schema-v1.md](docs/json-schema-v1.md)).

| Code | Meaning |
|------|---------|
| 0 | Completed analysis (even if verdict is `NO`) unless `--fail-on` applies |
| 2 | Invalid input / config / syntax |
| 3 | Internal invariant failure |

`--fail-on never|no|unknown|non-yes` maps selected verdicts to `--ci-exit-code`.

## 9. Library API

```rust
use regexrel::{analyze_binary, Config, Query, Verdict};

let report = analyze_binary(
    Query::Equivalent,
    "a|b",
    "[ab]",
    &Config::default(),
)?;
assert_eq!(report.verdict, Verdict::Yes);
```

Backend-specific entry points: `analyze_binary_with_backend`,
`analyze_empty_with_backend`.

`Orientation` and `OrientedBackend` expose the same direction layer to library
callers, e.g. `OrientedBackend::new(&DerivativeBackend, Orientation::Auto)`.

## 10. Correctness boundary

- A completed `YES` or `NO` is exact for the supported subset and configuration.
- `UNKNOWN` is incomplete analysis, not a third semantic truth value.
- `UNSUPPORTED` is rejected syntax or feature.
- Backends may differ in *resource* behaviour (which limit hits first) but not
  in completed verdicts.

## 11. Non-goals

No PCRE/JS/Rust-`regex` compatibility layer; no backtracking simulation; no
network services; no locale/case-folding completeness; no silent approximation
of unsupported constructs.

## 12. Document map

| Doc | Contents |
|------|----------|
| [README.md](README.md) | Tour, demos, configuration |
| [docs/backends.md](docs/backends.md) | Engines in depth |
| [docs/semantics.md](docs/semantics.md) | Language model |
| [docs/architecture.md](docs/architecture.md) | Module map |
| [docs/limitations.md](docs/limitations.md) | Known bounds |
| [docs/json-schema-v1.md](docs/json-schema-v1.md) | JSON contract |
| [bench/README.md](bench/README.md) | Benchmark suite |
