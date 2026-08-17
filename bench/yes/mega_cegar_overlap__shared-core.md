# overlap: identical (((a|b){25}c){10}) core, only the trailing x is checked
#
# Both sides are exactly `((a|b){25}c){10}x` -- 10 concatenated copies of a
# 25-way `a`/`b` block followed by `c`, then a trailing `x`. Structurally
# identical on both sides, so `abstraction` abstracts the whole
# `((a|b){25}c){10}` prefix to a single fresh marker and reduces the query
# to `marker x` vs `marker x`: a 2-state product (empty prefix, then the
# accepting state after `x`). `automata`, working from the concrete NFAs,
# explores the full pairwise product: 512 states / 1001 transitions.
#
# This is the case cited by name in the "abstraction" section of
# docs/backends.md ("visits 2 product states where automata visits 512").
# Compare directly with:
#   ./target/release/regexrel --backend abstraction --stats overlap \
#     '((a|b){25}c){10}x' '((a|b){25}c){10}x'
#   ./target/release/regexrel --backend automata --stats overlap \
#     '((a|b){25}c){10}x' '((a|b){25}c){10}x'
overlap '((a|b){25}c){10}x' '((a|b){25}c){10}x'
