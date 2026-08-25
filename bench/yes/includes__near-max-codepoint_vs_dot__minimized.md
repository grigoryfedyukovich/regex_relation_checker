# includes: U+10FFFE vs .  (--alphabet unicode)
# Backend-agnostic: expected answer is the same for every backend.
# To specifically re-check the MinimizedBackend regression this covers,
# run: ./bench/run.sh --keep-going "--backend minimized"
# includes() variant: does the single string "U+10FFFE" lie in `.`'s
# language? Yes -- it isn't \n. Before the alphabet_partition fix,
# `minimized`'s dropped trailing range made this fail outright (the
# tool's own witness-replay consistency check caught the resulting
# contradiction and aborted with an internal error, rather than silently
# returning a wrong answer).
--alphabet unicode includes '􏿾' '.'
