# match: (a|b)*a(a|b){60} rejects b^61
match '(a|b)*a(a|b){60}' 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
