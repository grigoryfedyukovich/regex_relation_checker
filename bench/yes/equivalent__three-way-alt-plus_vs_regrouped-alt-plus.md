# equivalent: (a|b|c)+ vs (a|(b|c))+
# Alternation is associative -- regrouping a three-way alternation changes nothing.
equivalent '(a|b|c)+' '(a|(b|c))+'
