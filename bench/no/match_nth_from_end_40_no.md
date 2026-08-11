# match: (a|b)*a(a|b){40} rejects b^41
match '(a|b)*a(a|b){40}' 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
