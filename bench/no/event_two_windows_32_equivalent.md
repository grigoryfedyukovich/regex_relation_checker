# event_two_windows_32_equivalent
# Two event segments around a checkpoint; left segment requires different event types at the same offset. All accepted words have length at least 2*n+3.
equivalent '[RW]*R[RW]{32}#[RW]{32}R[RW]*' '[RW]*W[RW]{32}#[RW]{32}R[RW]*'
