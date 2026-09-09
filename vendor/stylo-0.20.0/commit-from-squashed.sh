#!/bin/sh
# Usage: commit-from-squashed.sh <squashed commit> [extra git-commit(1) arguments ...]
# Given a squashed commit made by the GitHub merge queue, runs git-commit(1) with your local changes
# while borrowing our author name/email from that commit, our author date from its committer date,
# and our commit message from that commit.
set -eu

squashed_commit=$1; shift
committer_date=$(git log -n1 --pretty='%cd' "$squashed_commit")

# -c is equivalent to --author=$(...'%aN <%aE>') -m $(...'%B'), but allows editing
set -- git commit -c "$squashed_commit" --date="$committer_date" "$@"
echo "$@"
"$@"
