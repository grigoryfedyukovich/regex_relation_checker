# overlap: same construction as mega_cegar_overlap__shared-core.md, scaled
# down to a 15-wide, 6-block core (`((a|b){15}c){6}x`).
#
# `abstraction` still collapses to a 2-state product; `automata` visits 188
# states / 361 transitions here (vs. 512/1001 at the 25-wide/10-block scale).
# Included alongside the larger case to show the reduction ratio growing
# with block count/width rather than being an artifact of one specific size
# -- roughly 94x here vs. roughly 256x at the larger scale.
overlap '((a|b){15}c){6}x' '((a|b){15}c){6}x'
