# Releasing Moli

The release workflow currently builds one precompiled artifact for
`x86_64-unknown-linux-gnu`. The archive contains the `moli` binary, project
licenses, README, version marker, and third-party license notices. A SHA-256
checksum is published beside it. The packager strips its staging copy of the
binary while leaving `target/release/moli` unchanged for local debugging.

## Prepare the release

1. Update `version` in `moli/Cargo.toml` and refresh `Cargo.lock`.
2. Commit and push those changes to the ref that should be released.
3. Optionally build the exact same package locally:

   ```bash
   scripts/release.sh --version 0.1.0
   ```

   Artifacts are written to `dist/`. The script rejects a version that does
   not match `moli/Cargo.toml` and verifies the packaged binary's reported
   version.

## Trigger GitHub Release

1. Open **Actions** in GitHub and choose the **Release** workflow.
2. Select **Run workflow** and choose the Git ref containing the release.
3. Enter the version (with or without a leading `v`).
4. Choose whether the release should be a prerelease or a draft, then run it.

The workflow builds the selected commit, creates the corresponding `vX.Y.Z`
tag, generates release notes, and uploads the archive and checksum. It stops
without publishing if the requested version does not match the manifest or if
the tag already exists.
