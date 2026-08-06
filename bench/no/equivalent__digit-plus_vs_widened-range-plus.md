# equivalent: \d+ vs [0-9a-z]+
# Broken by widening the right side to also allow letters.
equivalent '\d+' '[0-9a-z]+'
