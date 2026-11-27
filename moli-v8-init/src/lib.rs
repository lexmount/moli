//! Process-wide V8 initialization shared by Moli V8 users.
//!
//! V8 platform initialization is process-global. Keep the `Once` boundary
//! here so production and tests do not each grow their own init guards.

use std::sync::Once;

static V8_INIT: Once = Once::new();

pub fn ensure_v8_initialized(create_platform: impl FnOnce() -> v8::SharedRef<v8::Platform>) {
    ensure_v8_initialized_with_flags(None, create_platform);
}

/// Initialize V8 for the current process.
///
/// V8 process-global initialization can run only once. The first caller wins:
/// later calls with different flags or platform factories are ignored by the
/// `Once` guard.
pub fn ensure_v8_initialized_with_flags(
    flags: Option<&'static str>,
    create_platform: impl FnOnce() -> v8::SharedRef<v8::Platform>,
) {
    V8_INIT.call_once(|| {
        if let Some(flags) = flags {
            v8::V8::set_flags_from_string(flags);
        }
        // OHOS shell processes cannot make anonymous memory executable.
        #[cfg(target_env = "ohos")]
        v8::V8::set_flags_from_string("--jitless");
        // iPhoneOS runs Wasm through DrumBrake. Execution tracing is diagnostic only.
        #[cfg(all(target_os = "ios", target_arch = "aarch64", target_abi = ""))]
        v8::V8::set_flags_from_string("--jitless --wasm-jitless");
        let platform = create_platform();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();
    });
}
