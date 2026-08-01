#!/usr/bin/env sh
set -eu

regexrel overlap 'a+b' 'ab+'
regexrel includes '[a-z]+' '[a-z]{2,}'
regexrel equivalent 'a|b' '[ab]'
regexrel --json empty 'a*'
