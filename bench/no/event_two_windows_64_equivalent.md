# event_two_windows_64_equivalent
# Harder synchronizing-separator case: unique '#' splits two independent event windows.
equivalent '[RW]*R[RW]{64}#[RW]{64}R[RW]*' '[RW]*W[RW]{64}#[RW]{64}R[RW]*'
