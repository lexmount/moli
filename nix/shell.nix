{
  mkShell,
  cargo,
  cargo-nextest,
  clippy,
  cmake,
  curl,
  git,
  perl,
  pkg-config,
  python3,
  rustc,
  rustPlatform,
  rustfmt,
}:

mkShell {
  # A dev shell is not sandboxed, so the `v8` build script can fetch its own
  # prebuilt archive instead of taking one through `RUSTY_V8_ARCHIVE`. That
  # fetch shells out to Deno or `curl`, and the slim vendored tree drops the
  # `tools/download_file.py` fallback, so `curl` has to be here. `git` is here
  # because `moli-protocol/build.rs` runs vergen-gitcl under `fail_on_error`.
  packages = [
    cargo
    cargo-nextest
    clippy
    cmake
    curl
    git
    perl
    pkg-config
    python3
    rustc
    rustfmt
  ];

  nativeBuildInputs = [ rustPlatform.bindgenHook ];
}
