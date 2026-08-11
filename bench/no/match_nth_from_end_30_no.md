# match: (a|b)*a(a|b){30} rejects b^31
match '(a|b)*a(a|b){30}' 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
