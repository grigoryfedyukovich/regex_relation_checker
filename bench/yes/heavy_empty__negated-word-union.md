# empty: ([a-z][A-Z]){0}a{0} still has eps. True empty: empty alt?
# (a{1,0}) is invalid. Use class that matches nothing: [^\w\W]+
empty '[^\w\W]+'
