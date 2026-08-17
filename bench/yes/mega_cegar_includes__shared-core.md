# includes: same shared-core construction, under the Includes relation.
#
# `((a|b){25}c){10}x` included in itself -- trivially true, and (like the
# equivalent-query sibling in this family) decided purely by an abstract
# exhaustive search with no counterexample, no witness to expand: 2
# abstracted product states vs. 512 for `automata`.
includes '((a|b){25}c){10}x' '((a|b){25}c){10}x'
