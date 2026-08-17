# equivalent YES: identical nth-from-end n=28
# Concrete: Θ(2^28) residual/DFA structure — minutes or UNKNOWN under limits.
# Abstraction: entire pattern is one common subexpr → X vs X.
equivalent '(a|b)*a(a|b){28}' '(a|b)*a(a|b){28}'
