# optional_records_128
# n optional copies of the same field block admit precisely 0 through n copies.
equivalent '(f[0-9]{2};){0,128}' '((f[0-9]{2};)?){128}'
