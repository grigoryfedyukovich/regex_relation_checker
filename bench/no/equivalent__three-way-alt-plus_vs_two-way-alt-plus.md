# equivalent: (a|b|c)+ vs (a|b)+
# Dropping one branch entirely, unsurprisingly, does change the language.
equivalent '(a|b|c)+' '(a|b)+'
