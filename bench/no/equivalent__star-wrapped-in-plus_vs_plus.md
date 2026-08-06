# equivalent: (a*)+ vs a+
# Broken by comparing against plain plus instead: this side still accepts the empty string, plus doesn't.
equivalent '(a*)+' a+
