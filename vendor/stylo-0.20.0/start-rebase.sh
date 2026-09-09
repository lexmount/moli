#!/bin/sh
# Usage: start-rebase.sh <new base> [extra git-rebase(1) arguments ...]
# Equivalent to git rebase --onto <new base> <last old upstream commit>.
set -eu

new_base=$1; shift
first_commit=$(git log --pretty=\%H --grep='Servo initial downstream commit')
old_base=$first_commit~

git rebase --onto "$new_base" "$old_base" "$@"
