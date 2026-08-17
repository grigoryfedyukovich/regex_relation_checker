# equivalent YES: (X*)* == X* regardless of outer repetition count, since X*
# already contains the empty string — {20} and {21} copies of ((ab)*)* both
# collapse to exactly L((ab)*). (Originally miscategorized as NO; all of
# automata/minimized/derivatives/antimirov/abstraction agree on YES.)
equivalent '(((ab)*)*){20}' '(((ab)*)*){21}'
