# match: (a|b)*a(a|b){20} accepts a+b^20 — only ~20+1 residual steps; relation product is Θ(2^20)
match '(a|b)*a(a|b){20}' 'abbbbbbbbbbbbbbbbbbbb'
