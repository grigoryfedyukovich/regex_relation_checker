# equivalent: (a+b+)+ vs a+(b+a+)*b+
# The same language, regrouped around the boundary between blocks instead of within them.
equivalent '(a+b+)+' 'a+(b+a+)*b+'
