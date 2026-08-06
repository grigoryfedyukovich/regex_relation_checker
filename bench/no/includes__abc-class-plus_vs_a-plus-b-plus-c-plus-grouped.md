# includes: [abc]+ vs (a+)(b+)(c+)
# The reverse direction fails: "ba" is in the class but the grouped form requires a's before b's before c's.
includes '[abc]+' '(a+)(b+)(c+)'
