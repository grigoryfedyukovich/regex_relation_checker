# event_two_windows_128_equivalent
# Harder synchronizing-separator case: unique '#' splits two independent event windows.
equivalent '[RW]*R[RW]{128}#[RW]{128}R[RW]*' '[RW]*W[RW]{128}#[RW]{128}R[RW]*'
