# equivalent NO: identical tracking core n=26, different final letter
# Common subexpr (a|b)*a(a|b){26} → X; skeleton Xz vs Xy → immediate NO.
equivalent '(a|b)*a(a|b){26}z' '(a|b)*a(a|b){26}y'
