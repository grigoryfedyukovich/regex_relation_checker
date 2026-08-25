# equivalent: [^\n] vs [^\n-􏿿]  (--alphabet unicode)
# Backend-agnostic: expected answer is the same for every backend.
# To specifically re-check the MinimizedBackend regression this covers,
# run: ./bench/run.sh --keep-going "--backend minimized"
# Same family as the `.`-based regression, spelled with an explicit
# negated class instead of the `.` shorthand -- exercises the same
# alphabet_partition boundary bug (src/minimize.rs) via the parser's
# negation path rather than dot-expansion.
--alphabet unicode equivalent '[^\n]' '[^
-􏿿]'
