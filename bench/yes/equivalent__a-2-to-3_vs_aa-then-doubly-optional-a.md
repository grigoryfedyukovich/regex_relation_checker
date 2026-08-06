# equivalent: a{2,3} vs aa(a?)?
# Broken by making that final piece doubly-optional, which now also accepts just "a".
equivalent a{2,3} 'aa(a?)?'
