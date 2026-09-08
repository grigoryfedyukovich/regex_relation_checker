# equivalent NO: shared tracking n=24 + block count off-by-one
equivalent '(a|b)*a(a|b){24}(xy){80}' '(a|b)*a(a|b){24}(xy){79}'
