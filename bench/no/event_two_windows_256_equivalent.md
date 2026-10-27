# event_two_windows_256_equivalent
# Harder synchronizing-separator case: unique '#' splits two independent event windows.
equivalent '[RW]*R[RW]{256}#[RW]{256}R[RW]*' '[RW]*W[RW]{256}#[RW]{256}R[RW]*'
