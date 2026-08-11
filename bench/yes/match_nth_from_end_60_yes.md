# match: (a|b)*a(a|b){60} accepts a+b^60 — only ~60+1 residual steps; relation product is Θ(2^60)
match '(a|b)*a(a|b){60}' 'abbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
