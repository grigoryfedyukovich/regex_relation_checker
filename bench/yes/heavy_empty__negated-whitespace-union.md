# empty: [a-z&& something] — use impossible alternation of empty class via [0-9][^0-9] wait non-empty
# empty: a&~a style via [0-9][^\d] but that is non-empty for letters... 
# Use [\d\D] complement already covered. Double-empty concat with empty class:
# [\s\S] is full; [^\s\S]+ is empty language
empty '[^\s\S]+'
