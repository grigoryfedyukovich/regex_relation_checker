# equivalent: [^\t] vs [^\t-􏿿]  (--alphabet unicode)
# Backend-agnostic: expected answer is the same for every backend.
# To specifically re-check the MinimizedBackend regression this covers,
# run: ./bench/run.sh --keep-going "--backend minimized"
# Same shape with \t as the excluded control character.
--alphabet unicode equivalent '[^\t]' '[^	-􏿿]'
