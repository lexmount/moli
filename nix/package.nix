{
  lib,
  stdenv,
  rustPlatform,
  fetchurl,
  cmake,
  perl,
  pkg-config,
  python3,
  src,
  revision ? "unknown",
  version ? "1.0.2",
}:

let
  # The vendored `v8` crate links a prebuilt static library that its build
  # script downloads over the network. `RUSTY_V8_ARCHIVE` is copied rather than
  # downloaded whenever it is not an http(s) URL, so pointing it at a
  # fixed-output fetch keeps the build sandbox-clean. Building V8 from source is
  # not an option here: the vendored tree is prebuilt-only and rejects
  # `V8_FROM_SOURCE`.
  rustyV8Version = "146.9.0";

  archives = {
    aarch64-darwin = {
      target = "aarch64-apple-darwin";
      hash = "sha256-5N5IQxBpGKWBEemXd4Y7nLdXzCg+ZcBb11Td0mbjokI=";
    };
    aarch64-linux = {
      target = "aarch64-unknown-linux-gnu";
      hash = "sha256-Jk7v/5DEnFmbLCGGzIaZVjtzeL6k66CY/p6mXZvFt9I=";
    };
    x86_64-darwin = {
      target = "x86_64-apple-darwin";
      hash = "sha256-YOv8BnCfL7vFkmiar+VDbxdiJ95OpXZvGKm9OfDjtP0=";
    };
    x86_64-linux = {
      target = "x86_64-unknown-linux-gnu";
      hash = "sha256-Yu8vHivMad6I+uo1iXMIRBVrziVVUZ+7IiXYpbiB/pg=";
    };
  };

  inherit (stdenv.hostPlatform) system;

  archive =
    archives.${system} or (throw "moli: no prebuilt V8 static library is published for ${system}");

  librustyV8 = fetchurl {
    url = "https://github.com/denoland/rusty_v8/releases/download/v${rustyV8Version}/librusty_v8_release_${archive.target}.a.gz";
    inherit (archive) hash;
  };
in

rustPlatform.buildRustPackage {
  pname = "moli";
  inherit version src;

  # `cargoHash` rather than `cargoLock`: moli patches in a fork of curl-rust
  # that carries libcurl as a git submodule, and only the fetchCargoVendor path
  # behind `cargoHash` passes `--fetch-submodules`. With `cargoLock` the
  # submodule arrives empty and the static libcurl build fails.
  cargoHash = "sha256-lNZ7lUE/Xfq8otL2JA6Td3sZ8pNjq2XGE/vSYsN8n7o=";

  nativeBuildInputs = [
    cmake
    perl # openssl-src compiles OpenSSL from source
    pkg-config
    python3 # stylo generates its property tables with mako at build time
    rustPlatform.bindgenHook
  ];

  # A dependency ships CMake files, but the workspace itself is pure Cargo, so
  # the CMake setup hook must not try to configure the source root.
  dontUseCmakeConfigure = true;

  # `moli-protocol` builds its revision string by shelling out to git, and a
  # Nix source tree carries no `.git`. vergen's `fail_on_error` makes that a
  # hard failure that `VERGEN_IDEMPOTENT` cannot soften, so emit the revision
  # the caller already knows rather than pulling a git repository into the
  # sandbox.
  postPatch = ''
    echo 'fn main() { println!("cargo:rustc-env=VERGEN_GIT_SHA=${revision}"); }' \
      > moli-protocol/build.rs
  '';

  # The `lightmount` curl-sys fork builds vendored libcurl with brotli, and
  # finds brotli's C headers by scanning `$CARGO_HOME` and a `vendor/`
  # directory beside its own manifest. Neither exists in the Nix vendor
  # layout, so place the vendored brotlic-sys crate where that scan looks.
  # The cargo setup hook copies the vendor tree into the build directory and
  # makes it writable for exactly this kind of build script.
  preBuild = ''
    curlSys=$(find "$NIX_BUILD_TOP" -maxdepth 4 -type d -name 'curl-sys-*' -print -quit)
    brotliSys=$(find "$NIX_BUILD_TOP" -maxdepth 4 -type d -name 'brotlic-sys-*' -print -quit)
    if [ -z "$curlSys" ] || [ -z "$brotliSys" ]; then
      echo "moli: expected vendored curl-sys and brotlic-sys crates" >&2
      exit 1
    fi
    mkdir -p "$(dirname "$curlSys")/vendor"
    ln -sfn "$brotliSys" "$(dirname "$curlSys")/vendor/$(basename "$brotliSys")"
  '';

  env.RUSTY_V8_ARCHIVE = librustyV8;

  # Only the CLI is an installable artifact; the rest of the workspace is
  # library crates.
  cargoBuildFlags = [
    "--package"
    "moli"
    "--bin"
    "moli"
  ];

  # The suite binds loopback HTTP servers and drives a full browser runtime,
  # neither of which the build sandbox can host.
  doCheck = false;

  meta = {
    description = "Lite, fast, high-compatibility headless browser for AI agents";
    homepage = "https://github.com/lexmount/moli";
    license = with lib.licenses; [
      mit
      asl20
    ];
    mainProgram = "moli";
    platforms = builtins.attrNames archives;
  };
}
