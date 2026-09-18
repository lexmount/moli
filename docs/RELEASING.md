# Releasing Moli

The release workflow builds six native archives with stable asset names:

| System | Rust target | Archive |
| --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `moli-x86_64-unknown-linux-gnu.tar.gz` |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | `moli-aarch64-unknown-linux-gnu.tar.gz` |
| macOS Intel | `x86_64-apple-darwin` | `moli-x86_64-apple-darwin.tar.gz` |
| macOS Apple Silicon | `aarch64-apple-darwin` | `moli-aarch64-apple-darwin.tar.gz` |
| Windows x86_64 | `x86_64-pc-windows-msvc` | `moli-x86_64-pc-windows-msvc.zip` |
| Windows ARM64 | `aarch64-pc-windows-msvc` | `moli-aarch64-pc-windows-msvc.zip` |

Every archive contains the `moli` executable, project licenses, README,
version marker, and third-party license notices. The workflow also publishes
`moli-installer.sh` and `moli-installer.ps1`. Skills are maintained separately
in the repository and are not included in release assets.

Stable names are intentional: the latest non-prerelease asset is always
available at
`https://github.com/lexmount/moli/releases/latest/download/<asset-name>`.
The installers use those URLs, select the archive for the current platform,
and install the executable.

Each artifact is built on its native GitHub-hosted runner. The packager strips
only a staging copy, leaving the binary under `target/release` unchanged for
debugging. Because stripping invalidates a Mach-O signature, macOS staging
binaries are ad-hoc signed again and verified before packaging. They are not
Developer ID signed or notarized. Windows executables are not Authenticode
signed. Linux x86_64 and both macOS targets use the allocator shim bundled in
their official pointer-compressed V8 archives, routing native allocations to
PartitionAlloc. Targets without such an archive retain their existing
allocator: Linux ARM64 uses jemalloc and Windows uses the system allocator.

Release builds use one code-generation unit. The release workflow and the CI
release-regression binaries both invoke Cargo with `--release`, so they consume
the same production profile. Normal development and unit-test builds retain
their separate profiles and parallel code generation.

## Linux compatibility baseline

Linux builds and runtime checks use Ubuntu 22.04 (Jammy) containers. Release
runners are pinned to `ubuntu-22.04` and `ubuntu-22.04-arm`; self-hosted CI uses
the same container userspace. Pinning the runner alone is insufficient: native
libraries linked inside a newer container can raise the binary's minimum libc.
The Rust version still comes from `rust-toolchain`, installed with rustup.
CI builds both HEAD and the common ancestor on this baseline for comparable A/B
measurements. Cargo and rustup volumes are isolated from the former Debian image.

Before publishing, each Linux archive is extracted into a fresh Ubuntu 22.04
container without build dependencies. `scripts/check_linux_compat.py` checks the
packaged ELF's imported ABI versions against `GLIBC_2.35`, `GLIBCXX_3.4.30`, and
`CXXABI_1.3.13` (Jammy with distribution updates), rejecting newer or private ABI
requirements, including `GLIBC_ABI_DT_RELR`. The check then runs `--version` and
a local HTTP / JavaScript smoke test, covering page scripts, DOM access, and
JavaScript `fetch`. CI checks its HEAD release binary in the same way and runs
the CDP and WebDriver suites on Jammy. Browser-comparison jobs use the Jammy
variant of the pinned Playwright image.

This establishes Ubuntu 22.04 with updates as the tested minimum Linux runtime;
it does not claim compatibility with every older distribution or kernel.
Runtime packages are `ca-certificates`, `libstdc++6`, `zlib1g`, and
`libfontconfig1`, in addition to the base system libraries. The Python packager
needs Python 3.11+, or Python 3.10 with `tomli` (`apt-get install python3-tomli`
on Ubuntu 22.04).

## Prepare the release

1. Update `version` in `moli/Cargo.toml` and refresh `Cargo.lock`.
2. Commit and push those changes to the ref that should be released.
3. Optionally build the package for your current operating system locally.

   Linux or macOS:

   ```bash
   python3 scripts/release.py --version 0.1.1
   ```

   Windows PowerShell:

   ```powershell
   python scripts/release.py --version 0.1.1
   ```

   Artifacts are written to `dist/`. The packager rejects a version that does
   not match `moli/Cargo.toml`, checks the native Rust target, strips the staged
   executable, and verifies the packaged binary's reported version.

## Trigger GitHub Release

1. Open **Actions** in GitHub and choose the **Release** workflow.
2. Select **Run workflow** and choose the Git ref containing the release.
3. Enter the version (with or without a leading `v`).
4. Choose whether the release should be a prerelease or a draft, then run it.

The workflow validates the selected commit, builds all six native artifacts
in parallel, verifies the expected archives, creates the corresponding
`vX.Y.Z` tag, generates release notes, and uploads eight assets: six archives
and two installers. It stops without creating a release if any platform fails,
if the requested version does not match the manifest, or if the tag already
exists. A published, non-prerelease release is explicitly marked as the latest
release so the stable installer URLs switch to it immediately.
