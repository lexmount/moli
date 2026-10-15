# Native video resources

`moli-media` parses bounded WebM resources, retains compressed video packets, and
decodes the frame at a requested container timestamp. Backward seeks reset the
codec and replay reference frames. Snapshots own their I420 planes independently
of subsequent decoder calls and resource retirement.

The current backend supports opaque 8-bit VP8/VP9 video with SDR drawing. Encoded bytes, packet
counts, frame dimensions, and VP9 native frame allocations are bounded. The
renderer also limits the aggregate retained media resources. The backend exports
container and codec color metadata; it does not infer a color profile for an
untagged video. Canvas conversion uses the existing VideoFrame color pipeline.

Linux and macOS build libvpx 1.16.0 from source and require a C/C++ toolchain,
Clang headers, NASM on x86, and `rustup component add llvm-tools`. Cargo's build
job count controls native compilation. Windows x86_64 uses the upstream checked
prebuilt archive. Windows ARM64 uses `libvpx[highbitdepth]:arm64-windows-static-md`
from the vcpkg checkout pinned by the release workflow, with `VCPKG_ROOT` pointing
at that checkout and the matching Rust `llvm-tools` component installed.

The vendored `shiguredo_libvpx` changes are recorded in
`vendor/shiguredo_libvpx-2026.1.0/MOLI-PATCHES.md`. Original upstream licenses are
retained. The native backend does not depend on an installed player or ffmpeg
process at runtime.

Audio decoding, other containers/codecs, alpha video, high bit depth, HDR tone mapping, and complete
media playback lifecycle semantics remain separate work.
