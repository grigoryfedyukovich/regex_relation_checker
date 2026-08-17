# equivalent NO: two shared-looking blocks but order swapped
# Common subexpr discovery may still abstract each block; skeleton ab vs ba differs.
equivalent '(ab){40}x(cd){40}' '(cd){40}x(ab){40}'
