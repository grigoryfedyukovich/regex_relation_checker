# equivalent: . vs [^\n-􏿿]  (--alphabet unicode)
# Backend-agnostic: expected answer is the same for every backend.
# To specifically re-check the MinimizedBackend regression this covers,
# run: ./bench/run.sh --keep-going "--backend minimized"
# Regression for alphabet_partition (src/minimize.rs) silently dropping
# the alphabet range that ends at U+10FFFF: `minimized` used to answer
# YES here (both sides missing U+E000..U+10FFFF from their DFA the same
# way), while `automata` correctly answers NO -- `.` really does include
# that range and the right-hand class really does exclude it.
--alphabet unicode equivalent '.' '[^\n-􏿿]'
