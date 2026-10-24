# optional_records_64
# n optional copies of the same field block admit precisely 0 through n copies.
equivalent '(f[0-9]{2};){0,64}' '((f[0-9]{2};)?){64}'
