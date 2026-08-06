# equivalent: (a+)+ vs a*
# Broken by comparing against star instead: nested-plus still can't match the empty string.
equivalent '(a+)+' 'a*'
