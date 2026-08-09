# equivalent: 150 redundant lap-position blocks vs (a|b|c)*
#
# Left is `(((a|b|c*)?){150})*`: 150 concatenated copies of a block that
# optionally matches a single 'a', a single 'b', or any run of 'c's,
# the whole thing wrapped in an outer `*`. Any string over {a,b,c} can
# be split into at most 150 pieces per lap around the outer star (each
# 'a'/'b' as its own piece, each maximal run of 'c' as one piece,
# spilling into further laps as needed), so the language is exactly
# `(a|b|c)*` for any block count >= 1 -- this file just picks 150 to
# make the representation gap between engines visible.
#
# The block count is language-irrelevant but NOT representation-irrelevant:
# Brzozowski's derivative must track *which of the 150 block positions*
# the current lap is in as part of one combined residual term, so its
# distinct-state count grows linearly with the block count (~2N+1 states).
# Antimirov's linear form is a flat set of residuals with no such
# positional bookkeeping and stays at 3 states regardless of N. Both
# backends still return YES; `--stats` (or `bench/compare_antimirov.sh`
# on this file) is what makes the difference visible. See the "Antimirov
# partial derivatives" section of docs/backends.md.
equivalent '(((a|b|c*)?){150})*' '(a|b|c)*'
