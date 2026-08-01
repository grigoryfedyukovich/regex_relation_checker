# Architecture

## Pipeline

```text
regex text
   │
   ▼
hand-written parser ── spans/diagnostics
   │
   ▼
language-neutral AST
   │
   ▼
Thompson NFA builder
   │
   ▼
on-the-fly epsilon-closure subsets
   │
   ▼
product BFS + predecessor tree
   │
   ├── exact verdict
   ├── shortest witness
   └── resource statistics
```

## Frontend boundary

`parser.rs` owns source syntax, byte spans, recovery hints, and unsupported-feature classification. It emits `ast::Expr`, whose nodes contain only semantic constructors and source spans. Analysis never depends on parser cursor state or source-node identity.

## Character representation

`charset.rs` represents labels as normalized, disjoint inclusive scalar intervals. Membership uses binary search, and union linearly merges two already-normalized lists. Intersection, subtraction, complement, and alphabet clipping preserve deterministic interval order. Static slices represent the ASCII and Unicode scalar universes.

## NFA construction

`nfa.rs` uses Thompson fragments with one start and one end state. Counted repetition is expanded up to the configured `max_repeat`. Anchors and the empty expression compile to epsilon fragments.

## On-the-fly determinization

A DFA state is a sorted, duplicate-free vector of NFA state identifiers closed under epsilon transitions. Dense visitation tables compute transition targets and epsilon closure without tree-set allocation. DFA transitions are computed only for reachable subsets. The complete DFA is never materialized ahead of the query.

## Product search

Binary relations use a product key `(left_subset, right_subset)`. At each key, outgoing NFA interval endpoints from both sides induce symbolic character partitions. One lowest-scalar representative per partition is sufficient because both subset transitions are constant inside that partition.

BFS target predicates are:

- overlap: `left_accepting ∧ right_accepting`
- inclusion counterexample: `left_accepting ∧ ¬right_accepting`
- equivalence counterexample: `left_accepting XOR right_accepting`

Emptiness runs the same BFS idea over a unary `SubsetKey`; it does not allocate a meaningless right-hand subset. Timeout checks occur at BFS-state granularity.

Each discovered node stores one predecessor and character. Reconstructing that chain produces a shortest witness. The witness is then replayed against the NFAs as an independent internal check.

## Backend boundary

The default backend is an in-process exact automata backend. No solver or subprocess is used. The public `RelationBackend` trait accepts compiled NFAs and returns evidence plus statistics; `analyze_*_with_backend` keeps report assembly and witness replay outside the backend. Tests include deterministic fake backends to verify this boundary.

## Caching

v0.1.1 performs no persistent caching. This deliberately avoids stale semantic artifacts. A future cache must key entries by tool semantic version, pattern digest, complete configuration digest, query, and backend mode.

## Failure modes

- syntax/configuration error: process exit `2`
- internal invariant/replay error: process exit `3`
- unsupported semantics: report verdict `UNSUPPORTED`
- state or timeout exhaustion: report verdict `UNKNOWN`
- CI policy failure: configured nonzero exit code

## Report rendering

Text output is built once with a timing placeholder and finalized in place. JSON is serialized once and its `rendering_ms` and `total_ms` numeric fields are finalized in place. This avoids the former double-render path while preserving self-reported timing.
