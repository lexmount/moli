#!/bin/sh
# Usage: commit-from-merge.sh <path/to/servo/servo> <merge commit> [extra git-commit(1) arguments ...]
# Given a merge commit made by bors, runs git-commit(1) with your local changes
# while borrowing the author name/email from the right-hand parent of the merge,
# and the author date from the committer date of the merge.
set -eu

lookup_repo=$1; shift
merge_commit=$1; shift
author_name_email=$(git -C "$lookup_repo" log -n1 --pretty='%aN <%aE>' "$merge_commit"\^2)
committer_date=$(git -C "$lookup_repo" log -n1 --pretty='%cd' "$merge_commit")

set -- git commit --author="$author_name_email" --date="$committer_date" "$@"
echo "$@"
"$@"
