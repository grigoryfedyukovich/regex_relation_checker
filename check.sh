#!/usr/bin/env bash
# Runs every check that .github/workflows/ci.yml runs in CI, locally, in one shot.
# Mirrors the "test" job step-by-step so a clean local run means CI should pass too
# (modulo the rustfmt/clippy version actually installed on your machine matching
# whatever "stable" resolves to on the runners -- see the note at the bottom).
#
# Usage:
#   ./check.sh            run everything, stop at the first failure
#   ./check.sh --keep-going   run everything, report all failures at the end
#
# Run from the project root (the directory containing Cargo.toml).

set -uo pipefail

KEEP_GOING=0
if [[ "${1:-}" == "--keep-going" ]]; then
  KEEP_GOING=1
fi

if [[ ! -f Cargo.toml ]]; then
  echo "error: run this from the project root (no Cargo.toml found here)" >&2
  exit 2
fi

FAILED=0
STEP_NUM=0

run_step() {
  local desc="$1"
  shift
  STEP_NUM=$((STEP_NUM + 1))
  echo
  echo "==> [$STEP_NUM] $desc"
  echo "    \$ $*"
  if "$@"; then
    echo "==> [$STEP_NUM] OK"
  else
    local status=$?
    echo "==> [$STEP_NUM] FAILED (exit $status): $desc"
    FAILED=$((FAILED + 1))
    if [[ "$KEEP_GOING" -ne 1 ]]; then
      exit "$status"
    fi
  fi
}

# 1. Formatting -- must match rustfmt's canonical output exactly (whitespace only).
run_step "cargo fmt --all -- --check" \
  cargo fmt --all -- --check

# 2. Lints -- warnings are errors, same as CI.
run_step "cargo clippy --all-targets --all-features -- -D warnings" \
  cargo clippy --all-targets --all-features -- -D warnings

# 3. Unit + integration tests (golden.rs, cli.rs, corpus.rs, and all #[cfg(test)] modules).
run_step "cargo test --all-targets" \
  cargo test --all-targets

# 4. The three smoke-test invocations CI runs directly against the built binary.
run_step "cargo run -- overlap 'a+b' 'ab+'" \
  cargo run --quiet -- overlap 'a+b' 'ab+'

run_step "cargo run -- includes '[a-z]+' '[a-z]{2,}'" \
  cargo run --quiet -- includes '[a-z]+' '[a-z]{2,}'

run_step "cargo run -- equivalent 'a|b' '[ab]'" \
  cargo run --quiet -- equivalent 'a|b' '[ab]'

echo
if [[ "$FAILED" -eq 0 ]]; then
  echo "All $STEP_NUM checks passed."
  exit 0
else
  echo "$FAILED of $STEP_NUM checks failed."
  exit 1
fi

# Note: CI runs on both ubuntu-latest and macos-latest via dtolnay/rust-toolchain@stable.
# rustfmt/clippy's "stable" channel can resolve to a different patch version at different
# times, and some of their width/wrapping heuristics (especially for struct literals and
# array/tuple elements) have changed across releases -- that's why a few earlier rounds of
# this exact project needed platform-specific rustfmt tweaks. A clean run here is strong
# evidence but not an absolute guarantee both matrix legs will agree; if CI still disagrees
# with your local run, the version skew is the first thing to check
# (`cargo fmt --version`, `cargo clippy --version`) before assuming a real regression.
