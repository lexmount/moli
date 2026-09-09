#!/bin/sh
# Usage: sync.sh <path/to/filtered>
set -eu

root=$(pwd)
mkdir -p "$1"
cd -- "$1"
filtered=$(pwd)
mkdir -p "$root/_cache"
cd "$root/_cache"
export PATH="$PWD:$PATH"

step() {
    if [ "${TERM-}" != '' ]; then
        tput setaf 12
    fi
    >&2 printf '* %s\n' "$*"
    if [ "${TERM-}" != '' ]; then
        tput sgr0
    fi
}

step Downloading git-filter-repo if needed
if ! git filter-repo --version 2> /dev/null; then
    curl -O https://raw.githubusercontent.com/newren/git-filter-repo/v2.38.0/git-filter-repo
    chmod +x git-filter-repo

    git filter-repo --version
fi

step Cloning upstream if needed
if ! [ -e upstream ]; then
    git clone --bare --single-branch --branch main --progress https://github.com/mozilla-firefox/firefox.git upstream
fi

step Updating upstream
branch=$(git -C upstream rev-parse --abbrev-ref HEAD)
git -C upstream fetch origin $branch:$branch

step Filtering upstream
# Cloning and filtering is much faster than git filter-repo --source --target.
git clone --bare upstream -- "$filtered"
git -C "$filtered" filter-repo --force --paths-from-file "$root/style.paths"
