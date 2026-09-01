# Semantics

## Language model

Each accepted input denotes a set of finite strings over a configured alphabet. Relations are defined as follows:

- `empty(R)`: whether `L(R) = ∅`
- `overlap(A, B)`: whether `L(A) ∩ L(B) ≠ ∅`
- `includes(A, B)`: whether `L(A) ⊆ L(B)`
- `equivalent(A, B)`: whether `L(A) = L(B)`

All four queries use full-string matching. An input character must be consumed by the automaton unless the transition is epsilon. The outer anchors `^` and `$` are accepted but compile to epsilon because they restate full-string boundaries. Any nested or interior anchor is `UNSUPPORTED`.

## Alphabets

### ASCII

The alphabet is scalar values `U+0000..U+007F`.

### Unicode

The alphabet is all Unicode scalar values:

- `U+0000..U+D7FF`
- `U+E000..U+10FFFF`

Surrogate code points are excluded. No normalization or case folding is performed. Canonically equivalent Unicode strings remain distinct unless written identically.

In v0.1.1, shorthand classes are ASCII-defined:

- `\d = [0-9]`
- `\w = [A-Za-z0-9_]`
- `\s = [\t-\r ]`

Their uppercase forms are complements with respect to the selected alphabet.

## Dot

`.` denotes the selected alphabet minus newline by default. With `dot_matches_newline = true`, it denotes the entire selected alphabet.

## Character classes

Classes are unions of literals and inclusive ranges. A negated class is complemented against the selected alphabet. Range endpoints must be literals. Unicode property escapes and locale-dependent classes are unsupported.

## Quantifiers

`*`, `+`, and `?` have their conventional language meaning. Counted forms are inclusive:

- `{m}` means exactly `m`
- `{m,}` means at least `m`
- `{m,n}` means between `m` and `n`

Greedy, lazy, and possessive preferences do not alter a regular language, but suffix syntax such as `*?` and `*+` is rejected as unsupported rather than silently normalized.

A `{` that doesn't form one of the three forms above (`{2,1}` with the bounds
reversed, `{abc}` with a non-numeric body, an unterminated `{2`, and so on)
is a syntax error here, not a literal `{` -- unlike JS, Python, and the Rust
`regex` crate, which all fall back to treating an unparseable `{...}` as a
literal character. This is a deliberate divergence in favor of catching a
likely typo (a malformed counted repetition) rather than silently reinterpreting
it as a literal brace the person probably didn't intend.

## Witness ordering

The search uses BFS over deterministic subset/product states. Consequently, a returned witness has minimum codepoint length. Symbolic transition intervals are partitioned at every relevant endpoint. The lowest scalar representative of each partition is explored first, making output deterministic.

This tie-breaking rule is not intended to produce printable text. JSON output correctly escapes control characters.

## Proof status

The state search is exact and complete when it exhausts the reachable on-the-fly product. Operational limits do not weaken a completed proof. If timeout or product-state limit is reached first, the verdict is `UNKNOWN`.

`UNSUPPORTED` means the input requests semantics outside the model. Syntax errors are invalid input and use a distinct process exit code.
