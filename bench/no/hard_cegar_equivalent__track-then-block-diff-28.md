# equivalent NO: shared tracking n=28 + block count off-by-one
equivalent '(a|b)*a(a|b){28}(xy){60}' '(a|b)*a(a|b){28}(xy){59}'
