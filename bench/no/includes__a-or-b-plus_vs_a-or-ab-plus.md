# includes: (a|b)+ vs (a|ab)+
# The reverse direction fails: "b" alone is reachable from (a|b)+ but every (a|ab)+ block starts with 'a'.
includes '(a|b)+' '(a|ab)+'
