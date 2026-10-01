//! Format limits shared by bounded offline contexts and AudioBuffer storage.

use super::*;

pub(super) fn validate(
    scope: &mut v8::PinScope<'_, '_>,
    channels: u32,
    length: u32,
    sample_rate: f32,
) -> bool {
    // Web Audio 1.1's supported sample-rate range also applies to buffers.
    if !(1..=32).contains(&channels) || length == 0 || !(3000.0..=768000.0).contains(&sample_rate) {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "Unsupported audio channel count, length or sample rate.",
        );
        return false;
    }
    true
}
