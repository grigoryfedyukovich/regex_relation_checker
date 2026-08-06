# includes: \d+[a-z]* vs \d+[a-z]+
# The reverse direction fails: the star form also accepts digits with no letters at all.
includes '\d+[a-z]*' '\d+[a-z]+'
