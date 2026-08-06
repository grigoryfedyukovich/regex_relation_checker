#!/usr/bin/env bash
# Run every benchmark under bench/yes and bench/no and verify the verdict.
# Expected: files in bench/yes produce a first line of "YES";
#           files in bench/no  produce a first line of "NO".
#
# Usage (from the project root, after `cargo build --release`):
#   ./bench/run.sh
#   ./bench/run.sh --keep-going
#   ./bench/run.sh "--backend derivatives"
#   ./bench/run.sh --keep-going "--backend minimized --max-states 20000"
#
# At most one extra argument (quote it) is word-split and passed to regexrel
# before the benchmark file. The binary is taken from ./target/release/regexrel
# when present; otherwise falls back to `cargo run --quiet --release --`.

set -uo pipefail

KEEP_GOING=0
EXTRA=""
for arg in "$@"; do
  if [[ "$arg" == "--keep-going" ]]; then
    KEEP_GOING=1
  else
    EXTRA="$arg"
  fi
done

if [[ ! -f Cargo.toml ]]; then
  echo "error: run this from the project root (no Cargo.toml found here)" >&2
  exit 2
fi

if [[ -x ./target/release/regexrel ]]; then
  RUN=(./target/release/regexrel)
else
  RUN=(cargo run --quiet --release --)
fi

YES_DIR=bench/yes
NO_DIR=bench/no

if [[ ! -d "$YES_DIR" || ! -d "$NO_DIR" ]]; then
  echo "error: expected directories $YES_DIR and $NO_DIR" >&2
  exit 2
fi

FAILED=0
PASSED=0
TOTAL=0

run_one() {
  local expected="$1"
  local file="$2"
  TOTAL=$((TOTAL + 1))

  local out
  # Intentional unquoted $EXTRA so a single quoted arg like
  # "--backend derivatives" is split into separate CLI words.
  # shellcheck disable=SC2086
  if ! out=$("${RUN[@]}" $EXTRA "$file" 2>&1); then
    local status=$?
    echo "FAIL  $file  (tool exited $status)"
    echo "      output: $out"
    FAILED=$((FAILED + 1))
    if [[ "$KEEP_GOING" -ne 1 ]]; then
      exit "$status"
    fi
    return
  fi

  local first
  first=$(printf '%s\n' "$out" | head -n1 | tr -d '\r')

  if [[ "$first" == "$expected" ]]; then
    echo "OK    $file  -> $first"
    PASSED=$((PASSED + 1))
  else
    echo "FAIL  $file  expected $expected, got: $first"
    echo "      full output:"
    printf '%s\n' "$out" | sed 's/^/      /'
    FAILED=$((FAILED + 1))
    if [[ "$KEEP_GOING" -ne 1 ]]; then
      exit 1
    fi
  fi
}

echo "Using: ${RUN[*]}"
if [[ -n "$EXTRA" ]]; then
  echo "Extra regexrel args: $EXTRA"
fi
echo

echo "==> YES benchmarks (expect YES)"
for f in "$YES_DIR"/*.md; do
  [[ -e "$f" ]] || continue
  run_one YES "$f"
done

echo
echo "==> NO benchmarks (expect NO)"
for f in "$NO_DIR"/*.md; do
  [[ -e "$f" ]] || continue
  run_one NO "$f"
done

echo
if [[ "$FAILED" -eq 0 ]]; then
  echo "All $TOTAL benchmarks passed ($PASSED OK)."
  exit 0
else
  echo "$FAILED of $TOTAL benchmarks failed ($PASSED passed)."
  exit 1
fi
