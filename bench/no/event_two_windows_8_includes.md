# event_two_windows_8_includes
# Two event segments around a checkpoint; left segment requires different event types at the same offset. All accepted words have length at least 2*n+3.
includes '[RW]*R[RW]{8}#[RW]{8}R[RW]*' '[RW]*W[RW]{8}#[RW]{8}R[RW]*'
