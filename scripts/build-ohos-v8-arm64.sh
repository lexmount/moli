#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
work="${MOLI_OHOS_V8_BUILD_DIR:-$repo_root/target/ohos-v8}"
target=aarch64-unknown-linux-musl
v8_version=152.2.0
rusty_commit=2768994f664e8a6e3aba27503606c58339136e2a
v8_commit=c4ca1eccb90c5464d826b7713dc27178a53b0cfe

: "${RUSTY_V8_MUSL_SYSROOT:?Set RUSTY_V8_MUSL_SYSROOT to an ARM64 musl sysroot}"
export RUSTY_V8_MUSL_SYSROOT="$(cd "$RUSTY_V8_MUSL_SYSROOT" && pwd)"
mkdir -p "$work"
work="$(cd "$work" && pwd)"
artifacts="${MOLI_OHOS_V8_ARTIFACTS:-$work/artifacts}"
mkdir -p "$artifacts"
artifacts="$(cd "$artifacts" && pwd)"
source="$work/rusty_v8"
if [[ ! -d "$source/.git" ]]; then
    git clone --depth 1 --branch "v$v8_version" https://github.com/denoland/rusty_v8.git "$source"
fi
cd "$source"
[[ "$(git rev-parse HEAD)" == "$rusty_commit" ]]
git submodule update --init --recursive --depth 1 --jobs 8
[[ "$(git -C v8 rev-parse HEAD)" == "$v8_commit" ]]

for name in binding.cc binding.hpp; do
    vendored="$repo_root/vendor/v8-$v8_version/src/$name"
    cmp -s "$vendored" "src/$name" || cp "$vendored" "src/$name"
done

export RUSTUP_TOOLCHAIN="$(cat "$repo_root/rust-toolchain")"
export CARGO_TARGET_DIR="$work/v8-target"
export V8_FROM_SOURCE=1
export GN_ARGS='v8_enable_webassembly=true v8_enable_drumbrake=true v8_drumbrake_bounds_checks=true'
unset EXTRA_GN_ARGS DOCS_RS DENO_TRYBUILD RUSTY_V8_SKIP_DOWNLOAD V8_FORCE_DEBUG
cargo build --locked --release --target "$target" --features v8_enable_pointer_compression

gn="$CARGO_TARGET_DIR/$target/release/gn_out"
[[ -f "$gn/args.gn" ]] || gn="$CARGO_TARGET_DIR/$target/release/build/gn_out"
cp "$gn/obj/librusty_v8.a" "$gn/src_binding.rs" "$gn/args.gn" "$artifacts/"
(
    cd "$source/src"
    shasum -a 256 binding.cc binding.hpp
) > "$artifacts/binding-source.sha256"
(
    cd "$artifacts"
    shasum -a 256 librusty_v8.a src_binding.rs args.gn binding-source.sha256 > SHA256SUMS
)
