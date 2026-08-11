#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd -- "$script_dir/.." && pwd -P)"

usage() {
    cat <<'EOF'
Build and package a Moli release.

Usage:
  scripts/release.sh --version <VERSION> [options]

Options:
  --version <VERSION>     Release version, with or without a leading "v".
  --output-dir <DIR>      Artifact directory (defaults to ./dist).
  --binary <PATH>         Binary to package (defaults to Cargo's release output).
  --skip-build            Package an existing binary instead of running Cargo.
  -h, --help              Show this help.

Examples:
  scripts/release.sh --version 0.1.0
  scripts/release.sh --version v0.1.0 --skip-build --binary target/release/moli
EOF
}

fail() {
    printf 'release: %s\n' "$*" >&2
    exit 1
}

require_value() {
    local option="$1"
    local value="${2:-}"
    [[ -n "$value" ]] || fail "$option requires a value"
}

release_version=""
output_dir="$repo_root/dist"
binary=""
skip_build=false

while (( $# > 0 )); do
    case "$1" in
        --version)
            require_value "$1" "${2:-}"
            release_version="$2"
            shift 2
            ;;
        --output-dir)
            require_value "$1" "${2:-}"
            output_dir="$2"
            shift 2
            ;;
        --binary)
            require_value "$1" "${2:-}"
            binary="$2"
            shift 2
            ;;
        --skip-build)
            skip_build=true
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            fail "unknown argument: $1"
            ;;
    esac
done

[[ -n "$release_version" ]] || fail "--version is required"

version="${release_version#v}"
semver_pattern='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'
[[ "$version" =~ $semver_pattern ]] || fail "invalid semantic version: $release_version"

manifest_version="$(
    awk '$1 == "version" && $2 == "=" { gsub(/"/, "", $3); print $3; exit }' \
        "$repo_root/moli/Cargo.toml"
)"
[[ -n "$manifest_version" ]] || fail "could not read the version from moli/Cargo.toml"
[[ "$version" == "$manifest_version" ]] || \
    fail "requested version $version does not match moli/Cargo.toml ($manifest_version)"

cd -- "$repo_root"

target="$(rustc -vV | awk '$1 == "host:" { print $2; exit }')"
[[ "$target" =~ ^[A-Za-z0-9_][A-Za-z0-9_.-]*$ ]] || fail "invalid Rust target: $target"

if [[ "$skip_build" == false ]]; then
    cargo build --locked --release --package moli
fi

if [[ -z "$binary" ]]; then
    target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
    if [[ "$target_dir" != /* ]]; then
        target_dir="$repo_root/$target_dir"
    fi
    binary="$target_dir/release/moli"
elif [[ "$binary" != /* ]]; then
    binary="$repo_root/$binary"
fi

[[ -f "$binary" ]] || fail "binary does not exist: $binary"
[[ -x "$binary" ]] || fail "binary is not executable: $binary"

binary_version="$("$binary" version)"
[[ "$binary_version" == "$version" ]] || \
    fail "binary reports version $binary_version, expected $version"

for required_file in README.md RELEASING.md LICENSE LICENSE-APACHE LICENSE-MIT; do
    [[ -f "$repo_root/$required_file" ]] || fail "missing release file: $required_file"
done
for required_dir in third_party/licenses third_party/notices; do
    [[ -d "$repo_root/$required_dir" ]] || fail "missing release directory: $required_dir"
done

mkdir -p -- "$output_dir"
output_dir="$(cd -- "$output_dir" && pwd -P)"

package_name="moli-v${version}-${target}"
archive_name="$package_name.tar.gz"
checksum_name="$archive_name.sha256"

[[ ! -e "$output_dir/$archive_name" ]] || fail "artifact already exists: $output_dir/$archive_name"
[[ ! -e "$output_dir/$checksum_name" ]] || fail "artifact already exists: $output_dir/$checksum_name"

staging_root="$(mktemp -d "$output_dir/.moli-release.XXXXXX")"
cleanup() {
    if [[ -n "${staging_root:-}" && -d "$staging_root" ]]; then
        rm -rf -- "$staging_root"
    fi
}
trap cleanup EXIT

package_dir="$staging_root/$package_name"
mkdir -p -- "$package_dir/third_party"
install -m 0755 -- "$binary" "$package_dir/moli"

strip_tool="${STRIP:-strip}"
command -v -- "$strip_tool" >/dev/null 2>&1 || fail "strip tool does not exist: $strip_tool"
unstripped_size="$(wc -c < "$package_dir/moli")"
"$strip_tool" "$package_dir/moli"
stripped_size="$(wc -c < "$package_dir/moli")"
packaged_version="$("$package_dir/moli" version)"
[[ "$packaged_version" == "$version" ]] || \
    fail "stripped binary reports version $packaged_version, expected $version"
printf 'Stripped packaged binary: %s -> %s bytes\n' "$unstripped_size" "$stripped_size"

install -m 0644 -- \
    "$repo_root/README.md" \
    "$repo_root/RELEASING.md" \
    "$repo_root/LICENSE" \
    "$repo_root/LICENSE-APACHE" \
    "$repo_root/LICENSE-MIT" \
    "$package_dir/"
cp -R -- "$repo_root/third_party/licenses" "$package_dir/third_party/licenses"
cp -R -- "$repo_root/third_party/notices" "$package_dir/third_party/notices"
printf '%s\n' "$version" > "$package_dir/VERSION"

tar -C "$staging_root" -czf "$staging_root/$archive_name" "$package_name"
tar -tzf "$staging_root/$archive_name" > "$staging_root/archive-contents.txt"
grep -Fxq "$package_name/moli" "$staging_root/archive-contents.txt" || \
    fail "packaged archive does not contain the moli binary"

if command -v sha256sum >/dev/null 2>&1; then
    (
        cd -- "$staging_root"
        sha256sum "$archive_name" > "$checksum_name"
        sha256sum --check "$checksum_name"
    )
elif command -v shasum >/dev/null 2>&1; then
    (
        cd -- "$staging_root"
        shasum -a 256 "$archive_name" > "$checksum_name"
        shasum -a 256 --check "$checksum_name"
    )
else
    fail "sha256sum or shasum is required"
fi

mv -- "$staging_root/$archive_name" "$output_dir/$archive_name"
mv -- "$staging_root/$checksum_name" "$output_dir/$checksum_name"

printf 'Created %s\n' "$output_dir/$archive_name"
printf 'Created %s\n' "$output_dir/$checksum_name"
