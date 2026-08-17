# equivalent: same shared-core construction as
# mega_cegar_overlap__shared-core.md, under the Includes/Equivalent sound-YES
# path instead of Overlap.
#
# `((a|b){25}c){10}x` compared against itself. Equivalence's sound-YES needs
# no witness-expansion step (unlike Overlap): an abstract search that
# exhausts with no distinguishing counterexample is immediately trusted, so
# this is the cheapest possible case for the driver -- 2 abstracted product
# states vs. 512 for `automata` on the concrete NFAs, with zero witness
# replay overhead.
equivalent '((a|b){25}c){10}x' '((a|b){25}c){10}x'
