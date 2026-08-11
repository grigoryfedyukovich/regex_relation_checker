# match: (a|b)*a(a|b){40} accepts a+b^40 — only ~40+1 residual steps; relation product is Θ(2^40)
match '(a|b)*a(a|b){40}' 'abbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
