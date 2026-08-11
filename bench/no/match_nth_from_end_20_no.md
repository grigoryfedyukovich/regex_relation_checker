# match: (a|b)*a(a|b){20} rejects b^21
match '(a|b)*a(a|b){20}' 'bbbbbbbbbbbbbbbbbbbbb'
