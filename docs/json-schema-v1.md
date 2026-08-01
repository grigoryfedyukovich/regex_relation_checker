# JSON report schema v1

The JSON object emitted by `--json` is versioned by `schema_version: "1"`.

Required top-level fields:

- `schema_version`: machine-readable contract version
- `tool_version`: crate version
- `backend`: backend name and version
- `query`: `empty`, `overlap`, `includes`, or `equivalent`
- `verdict`: `YES`, `NO`, `UNKNOWN`, or `UNSUPPORTED`
- `diagnostic`: stable ID, message, optional frontend error, and effective assumptions
- `statistics`: NFA sizes, visited states, generated symbolic transitions, limits, alphabet, and phase timings
- `semantics`: full-match, witness-order, shorthand-class, dot, and proof-mode contract

`witness` is present only when the verdict has constructive evidence. It contains:

- `value`: the string, JSON-escaped as needed
- `relation`: `in_language`, `in_both`, `left_only`, or `right_only`
- `codepoints`: Unicode scalar count

The witness is the structured evidence for relation reports; schema v1 does not duplicate it inside `diagnostic`.

## Timing fields

`statistics.timings` contains:

- `parsing_ms`
- `automata_build_ms`
- `backend_ms`
- `witness_extraction_ms`
- `witness_validation_ms`
- `rendering_ms`
- `total_ms`

`total_ms` is exactly the sum of the six component fields. `backend_ms` excludes separately measured witness extraction. `rendering_ms` covers report serialization/template construction; terminal I/O and the tiny in-place timing-field finalization step are excluded.

## `semantics.bounded`

The in-process automata backend emits exact completed verdicts, so `bounded` is `false`. Counted repetition such as `{2,5}` is exact regex syntax and does not make the proof bounded. Operational timeout and state limits are reported in `statistics`; reaching either produces `UNKNOWN`.

Consumers must ignore unknown additive fields within schema v1. A removal, renamed field, changed field type, or changed verdict meaning requires a new schema version.
