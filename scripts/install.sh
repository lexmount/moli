#!/bin/sh
set -eu

moli_release_base_url=${MOLI_RELEASE_BASE_URL:-https://github.com/lexmount/moli/releases/latest/download}
moli_install_dir=${MOLI_INSTALL_DIR:-${HOME:?HOME is not set}/.local/bin}
moli_tmp_dir=
moli_staged_binary=

moli_fail() {
    printf 'moli installer: %s\n' "$*" >&2
    exit 1
}

moli_cleanup() {
    if [ -n "$moli_staged_binary" ] && \
        { [ -e "$moli_staged_binary" ] || [ -L "$moli_staged_binary" ]; }; then
        rm -f "$moli_staged_binary"
    fi
    if [ -n "$moli_tmp_dir" ] && [ -d "$moli_tmp_dir" ]; then
        rm -rf "$moli_tmp_dir"
    fi
}

moli_download() {
    moli_download_url=$1
    moli_download_output=$2
    case "$moli_download_url" in
        https://*)
            curl --proto '=https' --tlsv1.2 -fsSL \
                "$moli_download_url" -o "$moli_download_output"
            ;;
        *)
            curl -fsSL "$moli_download_url" -o "$moli_download_output"
            ;;
    esac
}

moli_system=$(uname -s)
moli_machine=$(uname -m)
case "$moli_system:$moli_machine" in
    Linux:x86_64 | Linux:amd64)
        moli_target=x86_64-unknown-linux-gnu
        ;;
    Darwin:x86_64 | Darwin:amd64)
        moli_target=x86_64-apple-darwin
        ;;
    Darwin:arm64 | Darwin:aarch64)
        moli_target=aarch64-apple-darwin
        ;;
    *)
        moli_fail "unsupported platform: $moli_system $moli_machine"
        ;;
esac

moli_archive_name="moli-$moli_target.tar.gz"
moli_release_base_url=${moli_release_base_url%/}
moli_tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/moli-install.XXXXXX")
trap moli_cleanup EXIT HUP INT TERM

moli_archive_path="$moli_tmp_dir/$moli_archive_name"
moli_package_dir="$moli_tmp_dir/package"

printf 'Downloading %s...\n' "$moli_archive_name"
moli_download "$moli_release_base_url/$moli_archive_name" "$moli_archive_path"

mkdir "$moli_package_dir"
tar -xzf "$moli_archive_path" -C "$moli_package_dir" --strip-components=1
if [ ! -f "$moli_package_dir/moli" ]; then
    moli_fail "downloaded archive does not contain moli"
fi

mkdir -p "$moli_install_dir"
moli_staged_binary="$moli_install_dir/.moli.install.$$"
install -m 0755 "$moli_package_dir/moli" "$moli_staged_binary"
mv -f "$moli_staged_binary" "$moli_install_dir/moli"
printf 'Installed moli to %s/moli\n' "$moli_install_dir"

case ":${PATH:-}:" in
    *":$moli_install_dir:"*) ;;
    *)
        printf 'Add %s to PATH, then run: moli version\n' "$moli_install_dir"
        ;;
esac
