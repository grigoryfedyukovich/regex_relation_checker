# includes: (a|ab)+ vs (a|b)+
# Everything built from repeating "a" or "ab" only ever uses a's and b's.
includes '(a|ab)+' '(a|b)+'
