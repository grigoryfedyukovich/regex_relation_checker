# empty: [􏿿]  (--alphabet unicode)
# Backend-agnostic: expected answer is the same for every backend.
# To specifically re-check the MinimizedBackend regression this covers,
# run: ./bench/run.sh --keep-going "--backend minimized"
# Regression for alphabet_partition (src/minimize.rs) silently dropping
# a range that ends at U+10FFFF: a singleton class containing only the
# maximum scalar value used to be reported empty by `minimized` (the
# transition for it never made it into the DFA), while `automata`
# correctly answers NO -- the class has exactly one member.
--alphabet unicode empty '[􏿿]'
