#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
target=aarch64-unknown-linux-ohos
sdk="${OHOS_SDK_NATIVE:-/Applications/DevEco-Studio.app/Contents/sdk/default/openharmony/native}"
if [[ ! -x "$sdk/llvm/bin/$target-clang" || ! -d "$sdk/sysroot" ]]; then
    echo 'Set OHOS_SDK_NATIVE to the DevEco/OpenHarmony native SDK directory.' >&2
    exit 1
fi
sdk="$(cd "$sdk" && pwd)"

v8_version=152.2.0
v8_crate_sha256=a10fe1a92da5c32c7c7f838ce36c0ccfcfd5edf0865b58bdde820aa64cea9888
v8_work="${MOLI_OHOS_V8_BUILD_DIR:-$repo_root/target/ohos-v8}"
v8_artifacts="${MOLI_OHOS_V8_ARTIFACTS:-$v8_work/artifacts}"
if [[ ! -f "$v8_artifacts/SHA256SUMS" ]]; then
    echo 'Build the ARM64 DrumBrake archive with scripts/build-ohos-v8-arm64.sh first.' >&2
    exit 1
fi
v8_artifacts="$(cd "$v8_artifacts" && pwd)"
(cd "$v8_artifacts" && shasum -a 256 --check SHA256SUMS)
(cd "$repo_root/vendor/v8-$v8_version/src" && shasum -a 256 --check "$v8_artifacts/binding-source.sha256")
grep -Fxq 'v8_enable_pointer_compression = true' "$v8_artifacts/args.gn"
grep -Fxq 'v8_drumbrake_bounds_checks = true' "$v8_artifacts/args.gn"
if ! "$sdk/llvm/bin/llvm-ar" t "$v8_artifacts/librusty_v8.a" |
    awk '/wasm-interpreter-runtime\.o$/ { found = 1 } END { exit !found }'; then
    echo 'The ARM64 V8 archive must contain the DrumBrake interpreter.' >&2
    exit 1
fi

cache_dir="$v8_work"
mkdir -p "$cache_dir"
cache_dir="$(cd "$cache_dir" && pwd)"
scratch="$(mktemp -d "$cache_dir/.download.XXXXXX")"
trap 'rm -rf -- "$scratch"' EXIT

verify_sha256() {
    printf '%s  %s\n' "$1" "$2" | shasum -a 256 --check --status
}

download_pinned() {
    local file="$1" sha256="$2" url="$3"
    if [[ -f "$cache_dir/$file" ]] && verify_sha256 "$sha256" "$cache_dir/$file"; then
        return
    fi
    curl --fail --silent --show-error --location --retry 2 --output "$scratch/$file" "$url"
    if ! verify_sha256 "$sha256" "$scratch/$file"; then
        echo "Checksum mismatch for $file" >&2
        return 1
    fi
    mv -f "$scratch/$file" "$cache_dir/$file"
}

download_pinned "v8-$v8_version.crate" "$v8_crate_sha256" \
    "https://static.crates.io/crates/v8/v8-$v8_version.crate"
v8_source="$cache_dir/v8-$v8_version"
if [[ ! -d "$v8_source" ]]; then
    tar -xf "$cache_dir/v8-$v8_version.crate" -C "$scratch" \
        "v8-$v8_version/buildtools/third_party/libc++" \
        "v8-$v8_version/third_party/libc++/src/include" \
        "v8-$v8_version/third_party/libc++abi/src/include"
    mv -n "$scratch/v8-$v8_version" "$cache_dir/"
fi

export OHOS_SDK_NATIVE="$sdk"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_OHOS_LINKER="$sdk/llvm/bin/$target-clang"
export CC_aarch64_unknown_linux_ohos="$sdk/llvm/bin/$target-clang"
export CXX_aarch64_unknown_linux_ohos="$sdk/llvm/bin/$target-clang++"
export AR_aarch64_unknown_linux_ohos="$sdk/llvm/bin/llvm-ar"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_unknown_linux_ohos="--sysroot=\"$sdk/sysroot\" -D__MUSL__"

# The V8 bridges use a recent host Clang with the OHOS sysroot and Chromium's libc++.
export MOLI_OHOS_V8_SOURCE="$v8_source"
export RUSTY_V8_ARCHIVE="$v8_artifacts/librusty_v8.a"
export RUSTY_V8_ARCHIVE_SHA256="$(awk '$2 == "librusty_v8.a" { print $1 }' "$v8_artifacts/SHA256SUMS")"
export RUSTY_V8_SRC_BINDING_PATH="$v8_artifacts/src_binding.rs"
export V8_FROM_SOURCE=0 V8_FORCE_DEBUG=0 RUSTY_V8_SKIP_DOWNLOAD=0

if [[ "${1:-}" == "--manifest-path" ]]; then
    cargo build --locked --release --target "$target" "$@"
else
    cargo build --locked --release -p moli --target "$target" "$@"
fi
