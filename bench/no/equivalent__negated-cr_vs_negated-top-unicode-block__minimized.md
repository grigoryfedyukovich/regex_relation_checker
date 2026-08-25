# equivalent: [^\r] vs [^\r-􏿿]  (--alphabet unicode)
# Backend-agnostic: expected answer is the same for every backend.
# To specifically re-check the MinimizedBackend regression this covers,
# run: ./bench/run.sh --keep-going "--backend minimized"
# Same shape again with a different excluded control character (\r
# instead of \n), confirming the bug isn't specific to newline handling.
--alphabet unicode equivalent '[^\r]' '[^
-􏿿]'
