# equivalent NO: shared nested structure, outer counted block differs by one
# ((ab)*)* is a star, but the outer {n} on a non-absorbing unit (xy) is exact.
# Skeleton after abstracting the shared ((ab)*)* core: X(xy){20} vs X(xy){21}.
equivalent '(((ab)*)*)(xy){20}' '(((ab)*)*)(xy){21}'
