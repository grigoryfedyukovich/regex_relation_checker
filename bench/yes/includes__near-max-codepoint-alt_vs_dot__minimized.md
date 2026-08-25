# includes: U+10FFFD vs .  (--alphabet unicode)
# Backend-agnostic: expected answer is the same for every backend.
# To specifically re-check the MinimizedBackend regression this covers,
# run: ./bench/run.sh --keep-going "--backend minimized"
# Same includes() regression at a different near-max codepoint.
--alphabet unicode includes '􏿽' '.'
