# includes: (a|b)* vs a*b*
# The reverse direction fails: "ba" alternates freely but isn't all-a's-then-all-b's.
includes '(a|b)*' 'a*b*'
