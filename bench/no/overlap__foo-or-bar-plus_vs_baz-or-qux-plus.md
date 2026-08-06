# overlap: (foo|bar)+ vs (baz|qux)+
# Same left side; the right alternation no longer shares a word.
overlap '(foo|bar)+' '(baz|qux)+'
