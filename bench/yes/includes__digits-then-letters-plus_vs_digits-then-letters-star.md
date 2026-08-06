# includes: \d+[a-z]+ vs \d+[a-z]*
# Requiring at least one trailing letter is stricter than allowing zero or more.
includes '\d+[a-z]+' '\d+[a-z]*'
