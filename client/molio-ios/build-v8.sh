#!/bin/bash
set -euo pipefail
work=${MOLI_IOS_BUILD_DIR:-"${TMPDIR:-/tmp}/moli-ios"}
ios=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$ios/../.." && pwd)
rusty_commit=2768994f664e8a6e3aba27503606c58339136e2a
v8_commit=c4ca1eccb90c5464d826b7713dc27178a53b0cfe
target=aarch64-apple-ios

[[ $(uname -sm) == 'Darwin arm64' ]] || { echo 'Apple Silicon and Xcode required' >&2; exit 1; }
mkdir -p "$work"
work=$(cd "$work" && pwd)
source="$work/rusty_v8"
if [[ ! -d "$source/.git" ]]; then
    git clone --depth 1 --branch v152.2.0 https://github.com/denoland/rusty_v8.git "$source"
fi
cd "$source"
[[ $(git rev-parse HEAD) == "$rusty_commit" ]]
git submodule update --init --recursive --depth 1 --jobs 8
[[ $(git -C v8 rev-parse HEAD) == "$v8_commit" ]]
linker_patch="$ios/xcode27-host-linker.patch"
if ! git -C build apply --reverse --check "$linker_patch" 2>/dev/null; then
    git -C build apply "$linker_patch"
fi
cage_patch="$ios/ios-cage-reservation.patch"
if ! git -C v8 apply --reverse --check "$cage_patch" 2>/dev/null; then
    git -C v8 apply "$cage_patch"
fi
git submodule status --recursive > "$work/submodules.txt"
# Upstream explicitly skips the Windows-only tools/win submodule.
if grep -E '^[-+U]' "$work/submodules.txt" | grep -v ' tools/win$'; then
    echo 'Source dependencies are missing or do not match the pin' >&2
    exit 1
fi

export RUSTUP_TOOLCHAIN=$(cat "$repo/rust-toolchain")
developer_dir=$(xcode-select -p)
python=$(command -v "${PYTHON:-python3}")
# Rust host tools execute Python linkers with an SDKROOT for their GN
# toolchain. Use a real interpreter to avoid the /usr/bin xcrun shim.
if [[ "$python" == /usr/bin/python3 ]]; then
    python="$developer_dir/usr/bin/python3"
fi
mkdir -p "$work/host-tools/bin"
ln -sfn "$python" "$work/host-tools/bin/python3"
export PATH="$work/host-tools/bin:$PATH"
export PYTHON="$work/host-tools/bin/python3"
export LIBCLANG_PATH=${LIBCLANG_PATH:-"$developer_dir/Toolchains/XcodeDefault.xctoolchain/usr/lib"}
if [[ -z ${CLANG_BASE_PATH:-} ]]; then
    clang_version=$(PYTHONPATH=tools/clang/scripts PYTHONDONTWRITEBYTECODE=1 "$PYTHON" -c 'from update import PACKAGE_VERSION; print(PACKAGE_VERSION)')
    export CLANG_BASE_PATH="$work/host-tools/clang-$clang_version"
    if [[ ! -x "$CLANG_BASE_PATH/bin/clang" ]]; then
        mkdir -p "$CLANG_BASE_PATH"
        clang_archive="$work/chromium-clang.tar.xz"
        curl --retry 3 --max-time 600 -fsSL \
            "https://commondatastorage.googleapis.com/chromium-browser-clang/Mac_arm64/clang-$clang_version.tar.xz" \
            -o "$clang_archive"
        tar -xJf "$clang_archive" -C "$CLANG_BASE_PATH"
    fi
fi

# Fetch the tool versions pinned in V8 DEPS. curl also honors the Mac's
# configured proxy, unlike the upstream Python HTTPSConnection downloader.
fetch_tool() {
    local package=$1 version=$2 tool=$3
    local dir="$work/host-tools/$tool" api instance url
    [[ -x "$dir/$tool" ]] && return 0
    api=https://chrome-infra-packages.appspot.com/_ah/api/repo/v1
    mkdir -p "$dir"
    instance=$(curl --retry 3 --max-time 120 -fsSLG "$api/instance/resolve" \
        --data-urlencode "package_name=$package" --data-urlencode "version=$version" |
        python3 -c 'import json,sys; print(json.load(sys.stdin)["instance_id"])')
    url=$(curl --retry 3 --max-time 120 -fsSLG "$api/instance" \
        --data-urlencode "package_name=$package" --data-urlencode "instance_id=$instance" |
        python3 -c 'import json,sys; print(json.load(sys.stdin)["fetch_url"])')
    curl --retry 3 --max-time 120 -fsSL "$url" -o "$dir/package.zip"
    unzip -oq "$dir/package.zip" -d "$dir"
    chmod +x "$dir/$tool"
}
if [[ -z ${GN:-} ]]; then
    fetch_tool gn/gn/mac-arm64 git_revision:641ace93dd9560e75e7add0d08f77b446fbb3b78 gn
    export GN="$work/host-tools/gn/gn"
fi
if [[ -z ${NINJA:-} ]]; then
    fetch_tool infra/3pp/tools/ninja/mac-arm64 version:3@1.12.1.chromium.4 ninja
    export NINJA="$work/host-tools/ninja/ninja"
fi
# The Chromium host Rust toolchain is distinct from the Cargo toolchain.
read -r chromium_rust_url chromium_rust_sha < <(
    PYTHONPATH=tools PYTHONDONTWRITEBYTECODE=1 "$PYTHON" -c '
from v8_deps import deps
dep = deps["third_party/rust-toolchain"]
obj = next(o for o in dep["objects"] if o["object_name"].startswith("Mac_arm64/"))
print("https://storage.googleapis.com/" + dep["bucket"] + "/" + obj["object_name"], obj["sha256sum"])
'
)
chromium_rust=third_party/rust-toolchain
if [[ ! -f "$chromium_rust/.rusty_v8_version" ]] ||
   [[ $(cat "$chromium_rust/.rusty_v8_version") != "$chromium_rust_url" ]]; then
    chromium_archive="$work/chromium-rust.tar.xz"
    curl --retry 3 --max-time 600 -fsSL "$chromium_rust_url" -o "$chromium_archive"
    printf '%s  %s\n' "$chromium_rust_sha" "$chromium_archive" | shasum -a 256 -c -
    mkdir -p "$chromium_rust"
    tar -xJf "$chromium_archive" -C "$chromium_rust"
    printf '%s' "$chromium_rust_url" > "$chromium_rust/.rusty_v8_version"
fi

for name in binding.cc binding.hpp; do
    vendored="$repo/vendor/v8-152.2.0/src/$name"
    cmp -s "$vendored" "src/$name" || cp "$vendored" "src/$name"
done

export CARGO_TARGET_DIR="$work/v8-target"
export IPHONEOS_DEPLOYMENT_TARGET=14.0
export V8_FROM_SOURCE=1
export PRINT_GN_ARGS=1
export GN_ARGS='target_os="ios" target_environment="device" ios_deployment_target="14.0" ios_enable_code_signing=false use_lld=false cppgc_enable_caged_heap=false v8_jitless=true v8_enable_pointer_compression_shared_cage=true v8_enable_31bit_smis_on_64bit_arch=true v8_use_external_startup_data=false v8_enable_webassembly=true v8_enable_drumbrake=true v8_enable_drumbrake_tracing=true v8_drumbrake_bounds_checks=true v8_enable_sparkplug=false v8_enable_maglev=false v8_enable_turbofan=false'
unset EXTRA_GN_ARGS DOCS_RS DENO_TRYBUILD V8_FORCE_DEBUG
# Setting target_os explicitly bypasses upstream's iOS defaults, including
# v8_enable_webassembly=false. Cargo supplies v8_enable_pointer_compression.
cargo build --locked --release --target "$target" --features v8_enable_pointer_compression -vv \
    2>&1 | tee "$work/v8-build.log"

gn="$CARGO_TARGET_DIR/$target/release/gn_out"
# Cargo 1.100's default build-dir uses an extra package directory.
[[ -f "$gn/args.gn" ]] || gn="$CARGO_TARGET_DIR/$target/release/build/gn_out"
"$GN" --script-executable="$PYTHON" args "$gn" --list --short > "$work/effective-gn-args.txt"
python3 - "$gn/args.gn" "$work/effective-gn-args.txt" <<'PY'
import pathlib
import re
import sys

required = {
    'target_os': '"ios"',
    'target_environment': '"device"',
    'target_cpu': '"arm64"',
    'use_custom_libcxx': 'true',
    'cppgc_enable_caged_heap': 'false',
    'is_debug': 'false',
    'v8_enable_sandbox': 'false',
    'v8_enable_v8_checks': 'false',
    'v8_jitless': 'true',
    'v8_enable_webassembly': 'true',
    'v8_enable_drumbrake': 'true',
    'v8_enable_drumbrake_tracing': 'true',
    'v8_drumbrake_bounds_checks': 'true',
    'v8_enable_pointer_compression': 'true',
    'v8_enable_pointer_compression_shared_cage': 'true',
    'v8_enable_31bit_smis_on_64bit_arch': 'true',
    'v8_use_external_startup_data': 'false',
    'v8_enable_sparkplug': 'false',
    'v8_enable_maglev': 'false',
    'v8_enable_turbofan': 'false',
}
for filename in sys.argv[1:]:
    actual = dict(re.findall(r'(?m)^\s*(\w+)\s*=\s*(\S+)', pathlib.Path(filename).read_text()))
    for key, value in required.items():
        if actual.get(key) != value:
            sys.exit(f'{filename}: {key}: expected {value}, found {actual.get(key)}')
PY

artifacts="$work/v8-device"
mkdir -p "$artifacts"
cp "$gn/obj/librusty_v8.a" "$gn/src_binding.rs" "$gn/args.gn" "$work/effective-gn-args.txt" "$artifacts/"
python3 - "$gn/project.json" "$artifacts/cxx-defines.txt" <<'PYCODE'
import json
import pathlib
import sys

project = json.loads(pathlib.Path(sys.argv[1]).read_text())
defines = project['targets']['//:rusty_v8']['defines']
retained = [
    d for d in defines
    if d == 'NDEBUG' or d.startswith(('V8_', 'CPPGC_', '_LIBCPP_', '_LIBCXXABI_'))
]
required = {'V8_COMPRESS_POINTERS', 'V8_COMPRESS_POINTERS_IN_SHARED_CAGE',
            'V8_31BIT_SMIS_ON_64BIT_ARCH', 'V8_JITLESS', 'V8_ENABLE_DRUMBRAKE',
            'V8_ENABLE_DRUMBRAKE_TRACING', 'V8_DRUMBRAKE_BOUNDS_CHECKS', 'V8_ENABLE_WEBASSEMBLY'}
if missing := required.difference(retained):
    sys.exit(f'Missing target ABI or interpreter macros: {sorted(missing)}')
pathlib.Path(sys.argv[2]).write_text(''.join(d + '\n' for d in retained))
PYCODE
cd "$artifacts"
# Inspect every object because the simulator can also report arm64.
[[ $(xcrun lipo -archs librusty_v8.a) == arm64 ]]
xcrun otool -l librusty_v8.a > "$work/v8-load-commands.log"
awk '
    $2 ~ /^LC_VERSION_MIN_/ && $2 != "LC_VERSION_MIN_IPHONEOS" { invalid++ }
    $1 == "platform" { seen++; if ($2 != "IOS" && $2 != "2") invalid++ }
    END { exit (!seen || invalid) }
' "$work/v8-load-commands.log"
xcrun llvm-nm --defined-only --demangle librusty_v8.a > "$work/v8-symbols.log"
grep 'WasmInterpreterRuntime::ContinueExecution(' "$work/v8-symbols.log"
grep 'Snapshot::DefaultSnapshotBlob()' "$work/v8-symbols.log"
(
    cd "$source/src"
    shasum -a 256 binding.cc binding.hpp
) > binding-source.sha256
shasum -a 256 binding-source.sha256 librusty_v8.a src_binding.rs args.gn cxx-defines.txt effective-gn-args.txt > SHA256SUMS
{
    printf 'Moli %s\nrusty_v8 152.2.0 %s\nV8 15.2.124.1 %s\n' "$(git -C "$repo" rev-parse HEAD)" "$rusty_commit" "$v8_commit"
    xcodebuild -version
    xcrun --sdk iphoneos --show-sdk-version
    cargo -V
    "$CLANG_BASE_PATH/bin/clang" --version
} > versions.txt
printf '%s\n' "$CLANG_BASE_PATH/bin/clang++" > "$artifacts/clangxx.path"
