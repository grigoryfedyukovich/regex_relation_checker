# includes: shared (a|b)* prefix, left is stricter suffix
includes '(a|b)*a{30}' '(a|b)*a{20,}'
