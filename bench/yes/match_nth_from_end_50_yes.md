# match: (a|b)*a(a|b){50} accepts a+b^50 — only ~50+1 residual steps; relation product is Θ(2^50)
match '(a|b)*a(a|b){50}' 'abbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
