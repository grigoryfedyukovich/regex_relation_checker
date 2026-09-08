# equivalent YES: three ordered markers with free binary filler between each
# Antichain must remember which markers have been seen (subset of 3) × filler NFA.
equivalent '(a|b)*x(a|b){10}y(a|b){10}z(a|b)*' '(a|b)*x(a|b){10}y(a|b){10}z(a|b)*'
