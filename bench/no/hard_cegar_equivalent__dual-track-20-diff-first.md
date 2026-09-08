# equivalent NO: two trackers n=20; first window off-by-one
equivalent '(a|b)*a(a|b){20}(a|b)*a(a|b){20}' '(a|b)*a(a|b){19}(a|b)*a(a|b){20}'
