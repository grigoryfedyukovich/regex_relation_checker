# event_two_windows_512_equivalent
# Harder synchronizing-separator case: unique '#' splits two independent event windows.
equivalent '[RW]*R[RW]{512}#[RW]{512}R[RW]*' '[RW]*W[RW]{512}#[RW]{512}R[RW]*'
