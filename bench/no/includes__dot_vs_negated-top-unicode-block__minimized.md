# includes: . vs [^\n-􏿿]  (--alphabet unicode)
# Backend-agnostic: expected answer is the same for every backend.
# To specifically re-check the MinimizedBackend regression this covers,
# run: ./bench/run.sh --keep-going "--backend minimized"
# includes(left, right) asks whether every left-language string is also in
# the right language. `.` (left) really does match U+10FFFF (it's not \n),
# and the right side explicitly excludes the whole U+E000..U+10FFFF block,
# so this must be NO -- witnessed by the single-character string U+10FFFF.
# alphabet_partition (src/minimize.rs) used to drop that same trailing
# range from `.`'s own compiled DFA, making `minimized` answer YES.
--alphabet unicode includes '.' '[^
-􏿿]'
