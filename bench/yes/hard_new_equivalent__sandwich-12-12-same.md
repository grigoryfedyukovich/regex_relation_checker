# equivalent YES: suffix-window AND prefix-window around a fixed marker
# Hard for antichain: two independent tracking cores; product of two exp spaces.
# Distinct from dual-track (concatenated windows) — marker splits the word.
equivalent '(a|b)*a(a|b){12}z(a|b)*a(a|b){12}' '(a|b)*a(a|b){12}z(a|b)*a(a|b){12}'
