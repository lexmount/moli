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
v8_archive_name=librusty_v8_release_aarch64-unknown-linux-musl.a.gz
v8_archive_sha256=419a242557833151cf9dde82c12c8272c3c00015bfe7e340e48164a600d34039
v8_crate_sha256=a10fe1a92da5c32c7c7f838ce36c0ccfcfd5edf0865b58bdde820aa64cea9888
cache_dir="$repo_root/target/ohos-v8"
mkdir -p "$cache_dir"
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

# The release archive and vendored ARM64 bindings come from the same musl build.
download_pinned "$v8_archive_name" "$v8_archive_sha256" \
    "https://github.com/denoland/rusty_v8/releases/download/v$v8_version/$v8_archive_name"
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
export RUSTY_V8_ARCHIVE="$cache_dir/$v8_archive_name"
export RUSTY_V8_ARCHIVE_SHA256="$v8_archive_sha256"
export RUSTY_V8_SRC_BINDING_PATH="$repo_root/vendor/v8-$v8_version/gen/src_binding_release_aarch64-unknown-linux-musl.rs"
export V8_FROM_SOURCE=0 V8_FORCE_DEBUG=0 RUSTY_V8_SKIP_DOWNLOAD=0

if [[ "${1:-}" == "--manifest-path" ]]; then
    cargo build --locked --release --target "$target" "$@"
else
    cargo build --locked --release -p moli --target "$target" "$@"
fi
