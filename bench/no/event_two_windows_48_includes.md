# event_two_windows_48_includes
# Two event segments around a checkpoint; left segment requires different event types at the same offset. All accepted words have length at least 2*n+3.
includes '[RW]*R[RW]{48}#[RW]{48}R[RW]*' '[RW]*W[RW]{48}#[RW]{48}R[RW]*'
