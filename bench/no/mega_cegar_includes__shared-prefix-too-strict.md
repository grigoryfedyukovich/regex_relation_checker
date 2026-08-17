# includes NO: after shared (a|b)* prefix, a hard 'z' separator then more a's
# than the right side allows. z blocks absorption into the star.
includes '(a|b)*za{50}' '(a|b)*za{0,30}'
