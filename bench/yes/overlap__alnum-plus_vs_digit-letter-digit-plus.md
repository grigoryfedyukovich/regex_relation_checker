# overlap: [a-z0-9]+ vs [0-9]+[a-z]+[0-9]+
# As an includes question the other direction fails: not every alphanumeric string has that exact three-segment shape.
overlap '[a-z0-9]+' '[0-9]+[a-z]+[0-9]+'
