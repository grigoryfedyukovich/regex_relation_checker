# equivalent YES: fixed-width a-run then b-run, repeated
# NFA is small but subset construction tracks phase × run-length residue.
equivalent '((a{8}b{8}){6})' '((a{8}b{8}){6})'
