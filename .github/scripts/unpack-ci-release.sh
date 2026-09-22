#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 head|base EXPECTED_REVISION" >&2
  exit 2
fi

side=$1
expected_revision=$2
case "$side" in
  head|base) ;;
  *) echo "invalid release side: $side" >&2; exit 2 ;;
esac

release_name="moli-release-$side"
archive="target/ci-artifacts/$side/$release_name.tar.gz"
test -f "$archive"
mkdir -p target/ci-bin
tar -xzf "$archive" -C target/ci-bin
(cd "target/ci-bin/$release_name" && sha256sum --check SHA256SUMS)
actual_revision=$(<"target/ci-bin/$release_name/revision.txt")
if [[ "$actual_revision" != "$expected_revision" ]]; then
  echo "$side artifact revision mismatch: expected $expected_revision, got $actual_revision" >&2
  exit 1
fi
