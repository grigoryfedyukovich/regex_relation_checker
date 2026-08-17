# overlap: shared (((a|b){25}c){10}) core, disjoint trailing symbol (x vs y)
#
# Same shared core as bench/yes/mega_cegar_overlap__shared-core.md, but the
# two sides diverge on the one symbol that isn't abstracted away, so the
# real answer is NO. The abstracted round can't produce a sound YES here
# (marker+x vs marker+y disagree once expanded), so `abstraction` refines
# and falls back to running `automata` on the concrete patterns -- the same
# 513-state product `automata` alone would visit, plus the abstracted
# round's own small overhead. This is the "abstraction never does much
# worse than the inner alone" case referenced in docs/backends.md: measured
# overhead here is a few milliseconds on top of `automata`'s own runtime,
# not a multiplier.
overlap '((a|b){25}c){10}x' '((a|b){25}c){10}y'
