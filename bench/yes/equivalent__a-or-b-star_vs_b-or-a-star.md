# equivalent: (a|b)* vs (b|a)*
# Contrast with this: alternation order never matters, so swapping the branches changes nothing.
equivalent '(a|b)*' '(b|a)*'
