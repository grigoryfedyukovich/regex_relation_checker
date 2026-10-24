# event_two_windows_20_equivalent
# Two event segments around a checkpoint; left segment requires different event types at the same offset. All accepted words have length at least 2*n+3.
equivalent '[RW]*R[RW]{20}#[RW]{20}R[RW]*' '[RW]*W[RW]{20}#[RW]{20}R[RW]*'
