#!/bin/bash
set -euo pipefail

ios=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$ios/../.." && pwd)
work=${MOLI_IOS_BUILD_DIR:-"${TMPDIR:-/tmp}/moli-ios"}
target=aarch64-apple-ios
feature=${1:-browser}
case "$feature" in
    smoke|browser) ;;
    *) echo "Usage: $0 [smoke|browser]" >&2; exit 2 ;;
esac

mkdir -p "$work"
work=$(cd "$work" && pwd)
artifacts="$work/v8-device"
if [[ ! -f "$artifacts/SHA256SUMS" ]]; then
    echo "Build the iPhoneOS V8 artifacts with $ios/build-v8.sh first." >&2
    exit 1
fi
(cd "$artifacts"; shasum -a 256 -c SHA256SUMS)
if ! (cd "$repo/vendor/v8-152.2.0/src"; shasum -a 256 -c "$artifacts/binding-source.sha256"); then
    echo "V8 bindings changed. Run $ios/build-v8.sh again." >&2
    exit 1
fi
grep -Fxq 'cppgc_enable_caged_heap = false' "$artifacts/args.gn"
grep -Fxq 'v8_drumbrake_bounds_checks = true' "$artifacts/args.gn"

export RUSTY_V8_ARCHIVE="$artifacts/librusty_v8.a"
export RUSTY_V8_SRC_BINDING_PATH="$artifacts/src_binding.rs"
export RUSTY_V8_ARCHIVE_SHA256=$(awk '$2 == "librusty_v8.a" {print $1}' "$artifacts/SHA256SUMS")
export MOLIO_IOS_V8_CXX=$(cat "$artifacts/clangxx.path")
export MOLIO_IOS_V8_SOURCE="$work/rusty_v8"
export MOLIO_IOS_V8_DEFINES="$artifacts/cxx-defines.txt"
export CARGO_TARGET_DIR="$work/engine/$feature"
export IPHONEOS_DEPLOYMENT_TARGET=14.0
unset V8_FROM_SOURCE RUSTY_V8_SKIP_DOWNLOAD DOCS_RS DENO_TRYBUILD V8_FORCE_DEBUG GN_ARGS

cd "$repo"
cargo rustc --color never --locked --release --target "$target" -p molio-ios --lib \
    --no-default-features --features "$feature" -- --print native-static-libs \
    2>&1 | tee "$work/$feature-cargo.log"
native_libs=$(sed -n 's/^note: native-static-libs: //p' "$work/$feature-cargo.log")
[[ -n "$native_libs" ]]
printf '%s\n' "$native_libs" > "$CARGO_TARGET_DIR/native-static-libs.txt"
