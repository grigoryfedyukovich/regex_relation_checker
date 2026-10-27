# event_two_windows_512_includes
# Harder synchronizing-separator case: unique '#' splits two independent event windows.
includes '[RW]*R[RW]{512}#[RW]{512}R[RW]*' '[RW]*W[RW]{512}#[RW]{512}R[RW]*'
