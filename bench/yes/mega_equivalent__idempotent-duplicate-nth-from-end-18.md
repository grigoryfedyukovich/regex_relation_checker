# Same idempotent-duplication construction as the n=14 control, at n=18
# -- the depth where your automata backend's own logs show it already
# failing on plain (non-duplicated) nth-from-end. If ACI-idempotence is
# actually doing real work for the derivatives backend, this is the
# depth where it should show up as a real gap rather than both backends
# just being fast. If derivatives *also* fails here, that's a real
# negative result: it means idempotence isn't being applied early
# enough (e.g. only after full expansion, not before) to avoid the cost.
equivalent '((a|b)*a(a|b){18})|((a|b)*a(a|b){18})' '(a|b)*a(a|b){18}'
