# match: (a|b)*a(a|b){30} accepts a+b^30 — only ~30+1 residual steps; relation product is Θ(2^30)
match '(a|b)*a(a|b){30}' 'abbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
