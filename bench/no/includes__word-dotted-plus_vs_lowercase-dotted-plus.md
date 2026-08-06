# includes: \w+\.\w+ vs [a-z]+\.[a-z]+
# The reverse direction fails: word characters include digits and underscore, which lowercase-only can't match.
includes '\w+\.\w+' '[a-z]+\.[a-z]+'
