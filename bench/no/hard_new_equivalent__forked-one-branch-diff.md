# equivalent NO: one branch off-by-one, other identical
equivalent '((a|c)*a(a|c){16})|((b|d)*b(b|d){16})' '((a|c)*a(a|c){15})|((b|d)*b(b|d){16})'
