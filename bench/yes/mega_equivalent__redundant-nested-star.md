# equivalent: ((a*)*)* vs a*
# Classic catastrophic-backtracking pattern for backtracking regex
# engines -- trivial for an automata/derivative-based approach, since
# neither backtracks. Included as a deliberate contrast case.
equivalent '((a*)*)*' 'a*'
