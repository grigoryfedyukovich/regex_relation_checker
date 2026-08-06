# includes: [ab]+ vs (a+b+)+
# The reverse direction fails: "ba" is in the plain class but can't start with a 'b' block.
includes '[ab]+' '(a+b+)+'
