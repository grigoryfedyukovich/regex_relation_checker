# overlap: ca+t vs c[a-z]*t
# Same left side; the right side now allows anything in the middle, so it captures the left entirely.
overlap ca+t 'c[a-z]*t'
