# equivalent: (a|b)+(c|d)+ vs (a|b)*(c|d)+
# Broken by loosening the first group to star: now the empty prefix is allowed, which wasn't true before.
equivalent '(a|b)+(c|d)+' '(a|b)*(c|d)+'
