# event_two_windows_256_includes
# Harder synchronizing-separator case: unique '#' splits two independent event windows.
includes '[RW]*R[RW]{256}#[RW]{256}R[RW]*' '[RW]*W[RW]{256}#[RW]{256}R[RW]*'
