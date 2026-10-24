# event_two_windows_4_equivalent
# Two event segments around a checkpoint; left segment requires different event types at the same offset. All accepted words have length at least 2*n+3.
equivalent '[RW]*R[RW]{4}#[RW]{4}R[RW]*' '[RW]*W[RW]{4}#[RW]{4}R[RW]*'
