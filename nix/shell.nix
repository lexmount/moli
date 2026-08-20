{
  mkShell,
  cargo,
  cargo-nextest,
  clippy,
  cmake,
  perl,
  pkg-config,
  python3,
  rustc,
  rustPlatform,
  rustfmt,
}:

mkShell {
  # A dev shell is not sandboxed, so the `v8` build script can fetch its own
  # prebuilt archive. Only the native toolchain has to be provided here.
  packages = [
    cargo
    cargo-nextest
    clippy
    cmake
    perl
    pkg-config
    python3
    rustc
    rustfmt
  ];

  nativeBuildInputs = [ rustPlatform.bindgenHook ];
}
