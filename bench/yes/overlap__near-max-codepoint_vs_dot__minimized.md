# overlap: U+10FFFE vs .  (--alphabet unicode)
# Backend-agnostic: expected answer is the same for every backend.
# To specifically re-check the MinimizedBackend regression this covers,
# run: ./bench/run.sh --keep-going "--backend minimized"
# U+10FFFE is one below the maximum scalar value, and isn't \n, so it's
# genuinely in `.`'s language -- overlap must be YES. Regression for
# alphabet_partition (src/minimize.rs): when `.`'s own top interval
# reaches U+10FFFF, its whole trailing range (including U+10FFFE) used to
# be dropped from the compiled DFA entirely, so `minimized` answered NO.
--alphabet unicode overlap '􏿾' '.'
