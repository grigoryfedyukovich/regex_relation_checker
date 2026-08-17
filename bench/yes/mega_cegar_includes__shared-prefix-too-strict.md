# includes YES: the (a|b)* prefix on the right can absorb extra leading a's,
# so a string ending in exactly 40 a's can always be repartitioned as
# (a|b)* followed by at most 30 trailing a's. (Originally miscategorized as
# NO; all of automata/minimized/derivatives/antimirov/abstraction agree on
# YES.)
includes '(a|b)*a{40}' '(a|b)*a{0,30}'
