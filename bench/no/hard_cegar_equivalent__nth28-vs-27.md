# equivalent NO: nth-from-end 28 vs 27 — different languages, both exponential
# Abstraction may replace the common (a|b)*a(a|b) prefix structure partially;
# even one refinement or fall-back still hurts less than full 2^28 product.
# Best case: if collector only matches identical full trees, falls back —
# still a stress target. Prefer shared-core+marker variants below for pure CEGAR win.
equivalent '(a|b)*a(a|b){28}' '(a|b)*a(a|b){27}'
