# overlap: [U+10FFF0-U+10FFFE] vs .  (--alphabet unicode)
# Backend-agnostic: expected answer is the same for every backend.
# To specifically re-check the MinimizedBackend regression this covers,
# run: ./bench/run.sh --keep-going "--backend minimized"
# A small range just below the maximum scalar value, rather than a single
# codepoint -- same alphabet_partition regression, confirming it isn't
# specific to singleton classes.
--alphabet unicode overlap '[􏿰-􏿾]' '.'
