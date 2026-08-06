# includes: (a|b)+ vs (ab)+
# The reverse direction fails: "a" alone fits (a|b)+ but not a whole number of "ab" units.
includes '(a|b)+' '(ab)+'
