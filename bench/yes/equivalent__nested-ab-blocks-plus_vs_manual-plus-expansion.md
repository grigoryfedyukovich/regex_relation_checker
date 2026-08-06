# equivalent: (a+b+)+ vs (a+b+)(a+b+)*
# X+ is always the same as X followed by X* -- checked here with X itself already a composite piece.
equivalent '(a+b+)+' '(a+b+)(a+b+)*'
