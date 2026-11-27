# Moli on iPhoneOS

The `molio-ios` workspace crate builds a static library for arm64 iPhoneOS
(`aarch64-apple-ios`). The `browser` feature exposes a renderer and input
through the C ABI declared in [Molio.h](Molio.h). The `smoke` feature exposes a
small JS and Wasm smoke check. The UIKit application and Xcode packaging live
in the separate Molio iOS repository.

On Apple Silicon with Xcode, Git, Python 3, and curl, run from the Moli root:

```sh
rustup target add aarch64-apple-ios
export MOLI_IOS_BUILD_DIR="$PWD/target/iphoneos"
molio/ios/build-v8.sh
molio/ios/build-engine.sh smoke
molio/ios/build-engine.sh browser
```

The V8 script builds a pinned device archive with JIT disabled, pointer
compression, and the DrumBrake Wasm interpreter with explicit bounds checks.
The archive, generated Rust bindings, and C++ defines are built together.
Set `LIBCLANG_PATH` if using a libclang installation outside Xcode.

Each engine build writes `libmolio_ios.a` under
`$MOLI_IOS_BUILD_DIR/engine/<feature>/aarch64-apple-ios/release/` and writes
the required linker flags to `engine/<feature>/native-static-libs.txt`.
Use the same build directory from the application checkout when packaging.

For Chinese text, the iOS font collection prefers the regional PingFang family
when Fontique can load it and otherwise uses the system fallback.
