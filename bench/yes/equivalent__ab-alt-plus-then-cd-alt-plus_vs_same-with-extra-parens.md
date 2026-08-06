# equivalent: (a|b)+(c|d)+ vs (a|b)+((c|d)+)
# Two chained alternation-plus groups; extra grouping parentheses around the second one change nothing.
equivalent '(a|b)+(c|d)+' '(a|b)+((c|d)+)'
