# equivalent: 500 redundant lap-position blocks vs (a|b|c)*
#
# Same construction and same reasoning as
# mega_equivalent__antimirov-block-position-150.md, scaled up to 500
# blocks (matching the scale already used by
# mega_equivalent__500-plus-vs-counted.md elsewhere in this suite) to
# push the gap further: derivatives' single-term residual count grows to
# roughly 2*500+1 while antimirov's linear-form residual count is still 3.
# A good candidate for `bench/compare_antimirov.sh` and for experimenting
# with `--backend derivatives --timeout-ms` / `--max-states` at larger N
# than this file uses.
equivalent '(((a|b|c*)?){500})*' '(a|b|c)*'
