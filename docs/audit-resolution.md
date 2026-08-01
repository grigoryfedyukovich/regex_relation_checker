# v0.1 code-review resolution

This document records how the external v0.1 audit was handled in v0.1.1. It separates correctness fixes from safe local refactors and larger benchmark-dependent work.

## Confirmed bugs

| Audit item | Resolution |
|---|---|
| B1: `total_ms` omitted witness extraction | Fixed. `Timings::refresh_total` sums all six component phases. A unit test and JSON integration assertion verify the arithmetic. |
| B2: report rendered/serialized twice | Fixed. Text is rendered once with a timing placeholder. JSON is serialized once and the two timing number fields are finalized in place. |
| B3: repetition-bound overflow reported as missing digits | Fixed. Numeric parsing distinguishes no digits from `usize` overflow and emits a dedicated span and hint. |
| B4: configuration validated before CLI overrides | Fixed. `Config::load_raw` supports deserialize → override → validate. `Config::load` remains the validated standalone API. |
| B5: relation classification duplicated | Fixed. BFS target detection and independent witness replay share `classify_relation`; stable relation strings are centralized in `report::relation`. |

The timing repair also separates backend graph-search time from witness extraction before summing totals, avoiding double counting after B1.

## Maintainability findings

| Audit item | Resolution |
|---|---|
| CS1: unary and binary BFS duplication | Partially reduced through generic predecessor nodes and shared stopped-result construction. Full unification is deferred until benchmark data justifies the abstraction. |
| CS2: repeated resource-limit results | Fixed with `stopped_result`. |
| CS3: unary query mixed into `Query` | Deferred. Splitting the public enum would be an API change; the current entry guard remains. Candidate for v0.2. |
| CS4 / CS11: magic relation labels and permissive text fallback | Fixed with centralized constants and explicit left/right matching. Unknown future labels render as a neutral distinguishing-witness label rather than silently becoming right-only. |
| CS5: long `base_report` argument list | Deferred as a local cleanup; it does not affect correctness. |
| CS6: product key reused for unary search | Fixed with `SubsetKey`. |
| CS7: alphabet interval allocation | Fixed with static interval slices. |
| CS8: sorted-subset precondition undocumented | Fixed with documentation and a debug assertion. |
| CS9: assumptions omitted semantic configuration | Fixed; reports include `max_repeat` and `dot_matches_newline`. |
| CS10: `semantics.bounded` and counted repetition | The proposed change was not adopted. `{m,n}` is exact regex language syntax, not a bounded proof. The specification now defines `bounded=false` for completed exact automata analysis; operational limits yield `UNKNOWN` and are printed separately. |
| CS12: default diagnostic hint allocated then discarded | Fixed. The default hint is attached lazily only when no specific hint was supplied. |

## Performance findings

Implemented safe changes:

- replaced NFA `BTreeSet` visitation with dense boolean visitation tables;
- changed charset membership to binary search;
- changed normalized charset union to a linear merge;
- removed the meaningless unary right-subset allocation;
- reduced deadline checks to BFS-state granularity;
- removed duplicate rendering;
- reused static alphabet interval slices.

Deferred benchmark-dependent work:

- dynamic bitset product keys;
- precomputed or cached symbolic alphabet partitions;
- full generic unification of unary and binary BFS;
- hard process-level memory budgeting.

These deferred optimizations may improve throughput and allocation behavior, but they do not affect the soundness of completed `YES` or `NO` verdicts.

## Specification gaps

- Configuration is now explicitly TOML-only; the previous YAML/TOML wording was broader than the implementation.
- Constructive evidence is the top-level structured `witness`, rather than a duplicate `diagnostic.evidence` field in JSON schema v1.
- Rendering timing now has a precise definition: serialization/template construction only; terminal I/O and the small timing-field finalization patch are excluded.
- Counted repetition and operational analysis bounds are explicitly distinguished.
