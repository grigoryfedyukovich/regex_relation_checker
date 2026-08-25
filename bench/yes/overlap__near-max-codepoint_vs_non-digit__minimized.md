# overlap: U+10FFFE vs \D  (--alphabet unicode)
# Backend-agnostic: expected answer is the same for every backend.
# To specifically re-check the MinimizedBackend regression this covers,
# run: ./bench/run.sh --keep-going "--backend minimized"
# Same regression as the `.`-based overlap case, via \D (non-digit)
# instead -- \D also spans up to U+10FFFF and triggers the same dropped
# trailing range in alphabet_partition.
--alphabet unicode overlap '􏿾' '\D'
