# includes NO: union of two tracks is not subset of the first track alone
includes '((a|c)*a(a|c){14})|((b|d)*b(b|d){14})' '(a|c)*a(a|c){14}'
