# equivalent: ((a|b)*a(a|b){14})|((a|b)*a(a|b){14}) vs (a|b)*a(a|b){14}
# The left side is the exact same nth-from-end subpattern OR'd with an
# identical copy of itself -- R|R, syntactically, not just semantically.
# Brzozowski's idempotence rule (R|R = R under ACI) should collapse this
# on the first derivative step. Plain subset construction has no
# equivalent shortcut: it hashes raw NFA-state-sets, and states coming
# from the two duplicated copies are distinct sets even when they behave
# identically, so it has no structural reason to notice the redundancy.
# n=14 kept comfortably under both backends' individual wall (per your
# logs, ~16-17) so this is a control: expect both to pass, and this is
# mainly useful as a timing baseline for the n=18 variant below.
equivalent '((a|b)*a(a|b){14})|((a|b)*a(a|b){14})' '(a|b)*a(a|b){14}'
