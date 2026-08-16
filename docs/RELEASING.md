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

The workflow also assembles an npm CLI package set from those same native
archives. The public launcher is `@lexmount/moli@<version>`. Its optional
dependencies select one of four platform versions such as
`@lexmount/moli@<version>-linux-x64`, so npm downloads only the binary for the
host operating system and architecture. The launcher exposes the `moli`
command and forwards arguments, standard I/O, signals, and exit status to the
native executable. Linux npm installs currently require x86-64 glibc; Linux
ARM64 and musl are not advertised as supported targets.

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
signed. The allocator behind the default `jemalloc` feature is target-gated
out on Windows, so Windows builds use the system allocator because upstream
treats the Windows/MSVC combination as untested.

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
4. Choose whether the release should be a prerelease or a draft.
5. Enable npm publishing only after the npm trusted publisher described below
   is configured, then run the workflow.

The workflow validates the selected commit, builds all four native artifacts
in parallel, verifies the expected archives, creates the corresponding
`vX.Y.Z` tag, generates release notes, and uploads six assets: four archives
and two installers. It stops without creating a release if any platform fails,
if the requested version does not match the manifest, or if the tag already
exists. A published, non-prerelease release is explicitly marked as the latest
release so the stable installer URLs switch to it immediately.

Before creating the GitHub Release, the workflow builds five npm tarballs and
smoke-tests the Linux launcher against the real packaged binary. npm publishing
is disabled by default and is never attempted for draft releases. For a stable
release the launcher receives the `latest` dist-tag; for a prerelease it
receives `next`. Native platform versions are published first under
platform-specific dist-tags, and the launcher is published last so users never
receive a package whose required binary versions are incomplete.

## Configure npm publishing

The unscoped `moli` package name is already owned by another publisher, so the
release uses the public scoped package `@lexmount/moli`. Confirm that the npm
organization or user controlling `@lexmount` can publish public packages before
enabling the workflow option.

An npm Trusted Publisher can only be attached after the package exists. For the
first npm release, leave npm publishing disabled when running the Release
workflow, download its `npm-packages` artifact, sign in to npm interactively,
and publish the verified package set from the extracted artifact:

```sh
npm login
python3 scripts/publish_npm.py /path/to/npm-packages/npm-packages.json \
  --main-tag latest
```

Use `--main-tag next` instead when bootstrapping from a prerelease. The script
publishes the four native versions first and the launcher last. It is safe to
retry: versions whose registry integrity matches the artifact are skipped.
After this one-time publication, configure Trusted Publishing for subsequent
releases.

Configure an npm Trusted Publisher for the `lexmount/moli` GitHub repository
with these values:

- Workflow filename: `release.yml`
- Environment: `npm`
- Allowed action: `npm publish`

The publish job runs on a GitHub-hosted runner with Node 24, npm 11.5.1 or
newer, and the `id-token: write` permission. The publish script checks these
minimum versions before touching the registry. It does not use a long-lived
npm token, and npm generates provenance automatically for the public package.
Add any required reviewers or branch/tag restrictions to the GitHub `npm`
environment. After the first successful publication, installation is:

```sh
npm install --global @lexmount/moli
moli --version
```

For local package validation, place all four native archives in `dist/`, then
run:

```sh
python3 scripts/package_npm.py --version 1.0.0
python3 scripts/publish_npm.py dist/npm/npm-packages.json --dry-run
```

The package manifest records the SHA-512 integrity of every tarball. A retried
publish skips an existing version only when the registry reports the same
integrity; a mismatch stops the release. npm does not allow a published
name/version pair to be replaced, so release versions must be bumped before
publishing changed contents.
