# equivalent YES: identical nth-from-end n=34
# Concrete: Θ(2^34) residual/DFA structure — UNKNOWN under typical limits.
# Abstraction: entire pattern is one common subexpr → X vs X.
equivalent '(a|b)*a(a|b){34}' '(a|b)*a(a|b){34}'
