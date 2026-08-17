# equivalent NO: two trackers; second window differs
# Shared first tracker can be abstracted; second still exp but smaller,
# or both abstracted if structure matches partially.
equivalent '(a|b)*a(a|b){16}(a|b)*a(a|b){16}' '(a|b)*a(a|b){16}(a|b)*a(a|b){15}'
