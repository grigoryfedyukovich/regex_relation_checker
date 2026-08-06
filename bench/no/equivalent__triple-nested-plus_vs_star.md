# equivalent: ((a+)+)+ vs a*
# Broken by comparing against star instead: the nested form still can't match the empty string.
equivalent '((a+)+)+' 'a*'
