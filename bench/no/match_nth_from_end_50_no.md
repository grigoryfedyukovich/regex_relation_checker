# match: (a|b)*a(a|b){50} rejects b^51
match '(a|b)*a(a|b){50}' 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
