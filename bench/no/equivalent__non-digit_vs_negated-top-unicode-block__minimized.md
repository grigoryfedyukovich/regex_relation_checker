# equivalent: \D vs [^0-9-􏿿]  (--alphabet unicode)
# Backend-agnostic: expected answer is the same for every backend.
# To specifically re-check the MinimizedBackend regression this covers,
# run: ./bench/run.sh --keep-going "--backend minimized"
# \D (non-digit) is documented as always ASCII-defined for \d, so \D still
# spans the full Unicode alphabet minus 0-9 -- including U+E000..U+10FFFF,
# which the right side explicitly excludes. Regression for the same
# alphabet_partition boundary bug via a shorthand class instead of `.`.
--alphabet unicode equivalent '\D' '[^0-9-􏿿]'
