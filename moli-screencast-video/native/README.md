# x264 source

Unmodified upstream source from https://github.com/mirror/x264.

Revision: `c24e06c2e184345ceb33eb20a15d1024d9fd3497`.

`x264.tar.gz` is `git archive --format=tar --prefix=x264/ c24e06c2e184345ceb33eb20a15d1024d9fd3497`, gzip-compressed with mtime 0.

SHA-256: `03b672217a6fcf92c6408edd30812bb8c2914a027d7ff8559c90ca17ed87ec9c`.

License: GPL-2.0-or-later; see COPYING.x264 and the upstream source headers.
The resulting binaries containing x264 require the accompanying GPL notices/source distribution.

# NASM source

On x86 targets, an existing NASM >= 2.13 is used. If it is unavailable,
Cargo builds the bundled NASM 2.16.03 for the build host, retaining x264's
assembly optimizations. An explicit `AS` overrides this selection. Native
builds need a C compiler, make and a shell; no download runs in build.rs.

Unmodified release archive from
https://www.nasm.us/pub/nasm/releasebuilds/2.16.03/nasm-2.16.03.tar.xz,
SHA-256 `1412a1c760bbd05db026b6c0d1657affd6631cd0a63cddb6f73cc6d4aa616148`.
Recompressed as gzip with mtime 0; `nasm.tar.gz` SHA-256:
`581ce2840090f6c6d9b9e84c62e6ca8bfe63f1141a9079e6db54ca1efeb54341`.

License: BSD-2-Clause; see LICENSE.nasm. NASM is only a build tool and is not
linked into Moli.

# Windows builds

MSVC builds use the same bundled x264 revision with GNU make, Git for Windows
Bash/Perl, and the MSVC C/assembly tools detected by `cc`. Put Git's `usr/bin`
before Windows' WSL Bash on PATH. Missing NASM is built with MSVC/nmake.
ARM64 uses the gas-preprocessor script already included in the x264 archive.

`MOLI_X264_DIR` may instead point to a matching static prefix containing
`include/x264.h`, `include/x264_config.h` and `lib/libx264.lib` (MSVC), or
`lib/libx264.a` (Unix). The header and library must come from the same build.
