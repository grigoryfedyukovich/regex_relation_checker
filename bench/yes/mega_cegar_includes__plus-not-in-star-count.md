# includes YES: ((ab)+){30} = {(ab)^N : N>=30}, and ((ab)*){20} collapses
# (star absorbs the repetition count) to {(ab)^N : N>=0} = L((ab)*), so the
# left side is a subset. (Originally miscategorized as NO; all of
# automata/minimized/derivatives/antimirov/abstraction agree on YES.)
includes '((ab)+){30}' '((ab)*){20}'
