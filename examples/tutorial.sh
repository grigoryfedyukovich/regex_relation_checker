#!/usr/bin/env bash
set -euo pipefail

if [[ -x ./target/release/regexrel ]]; then
  regexrel=(./target/release/regexrel)
else
  regexrel=(cargo run --quiet --)
fi

run() {
  printf '\n$'
  printf ' %q' "${regexrel[@]}" "$@"
  printf '\n'
  "${regexrel[@]}" "$@"
}

run overlap 'a+b' 'ab+'
run includes '[a-z]+' '[a-z]{2,}'
run equivalent 'a|b' '[ab]'
run empty ''
run --alphabet unicode equivalent 'é+' 'éé*'
run --dot-matches-newline true overlap '.' '\n'
run --stats overlap 'a.*z' 'ab+z'
run --json equivalent 'b' 'a|b'
