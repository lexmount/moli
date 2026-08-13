# Releasing Moli

The release workflow builds four native archives with stable asset names:

| System | Rust target | Archive |
| --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `moli-x86_64-unknown-linux-gnu.tar.gz` |
| macOS Intel | `x86_64-apple-darwin` | `moli-x86_64-apple-darwin.tar.gz` |
| macOS Apple Silicon | `aarch64-apple-darwin` | `moli-aarch64-apple-darwin.tar.gz` |
| Windows x86_64 | `x86_64-pc-windows-msvc` | `moli-x86_64-pc-windows-msvc.zip` |

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
signed. The Windows build disables the default `jemalloc` feature and uses the
system allocator because upstream treats the Windows/MSVC combination as
untested.

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
   python scripts/release.py --version 0.1.1 --no-default-features
   ```

   Artifacts are written to `dist/`. The packager rejects a version that does
   not match `moli/Cargo.toml`, checks the native Rust target, strips the staged
   executable, and verifies the packaged binary's reported version.

## Trigger GitHub Release

1. Open **Actions** in GitHub and choose the **Release** workflow.
2. Select **Run workflow** and choose the Git ref containing the release.
3. Enter the version (with or without a leading `v`).
4. Choose whether the release should be a prerelease or a draft, then run it.

The workflow validates the selected commit, builds all four native artifacts
in parallel, verifies the expected archives, creates the corresponding
`vX.Y.Z` tag, generates release notes, and uploads six assets: four archives
and two installers. It stops without creating a release if any platform fails,
if the requested version does not match the manifest, or if the tag already
exists. A published, non-prerelease release is explicitly marked as the latest
release so the stable installer URLs switch to it immediately.
