# Molio engine for HarmonyOS

`molio-ohos` is the native engine adapter for an ARM64 HarmonyOS host app.
It runs Moli's viewport pages in-process and exposes browser commands, state,
and BGRA frames through [the C interface](include/molio_browser.h). The host
app owns the surface, system input method and file picker.

Build this workspace crate from the repository root with Rust's
`aarch64-unknown-linux-ohos` target and the DevEco native SDK installed.
The build uses the
[ARM64 DrumBrake archive](../../vendor/v8-152.2.0/MOLI_VENDORING.md).

```sh
rustup target add aarch64-unknown-linux-ohos
./scripts/build-ohos-arm64.sh --manifest-path client/molio-ohos/Cargo.toml
```

Set `OHOS_SDK_NATIVE` if the SDK is outside the script's default DevEco Studio
location. Link `target/aarch64-unknown-linux-ohos/release/libmolio_ohos.a` from
the repository root into the host app's native library. The host can send
commands defined in [`src/model.rs`](src/model.rs) and poll state and frames
with the most recent revision or generation it has seen. Pass an app-private
writable directory for cookies, storage, sessions and downloaded files.
