# Local changes to shiguredo_libvpx 2026.1.0

Upstream: https://github.com/shiguredo/libvpx-rs ; crates.io package SHA-256 e056f7ad0caee584d906681f671f347b306950dc7169e0879cd71941940d5d37.

Moli exposes native frame color metadata and bounded VP9 frame-buffer allocation, and peeks allocation dimensions before decoding. Windows ARM64 uses the pinned vcpkg libvpx 1.16.0 port, since the upstream package has neither a prebuilt artifact nor a source build for that target. Other source and prebuilt paths retain the upstream behavior. Native C symbols keep the upstream shiguredo_vpx prefix.
