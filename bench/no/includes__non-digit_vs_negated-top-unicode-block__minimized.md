# includes: \D vs [^0-9-􏿿]  (--alphabet unicode)
# Backend-agnostic: expected answer is the same for every backend.
# To specifically re-check the MinimizedBackend regression this covers,
# run: ./bench/run.sh --keep-going "--backend minimized"
# includes() variant of the \D regression above.
--alphabet unicode includes '\D' '[^0-9-􏿿]'
