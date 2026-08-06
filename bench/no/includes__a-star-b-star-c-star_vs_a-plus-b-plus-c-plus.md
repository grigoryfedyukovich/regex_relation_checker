# includes: a*b*c* vs a+b+c+
# The reverse direction fails: the all-star side can match the empty string, the all-plus side can't.
includes 'a*b*c*' a+b+c+
