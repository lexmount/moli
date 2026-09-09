#![doc = include_str!("../README.md")]
#![warn(rust_2018_idioms)]
#![warn(rust_2021_compatibility)]
#![warn(clippy::missing_panics_doc)]
#![warn(clippy::clone_on_ref_ptr)]
#![warn(clippy::todo)]
#![warn(clippy::dbg_macro)]
#![warn(clippy::unimplemented)]
#![deny(trivial_numeric_casts)]
#![deny(missing_debug_implementations)]
#![deny(unsafe_op_in_unsafe_fn)]

use std::error::Error;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Render quantum size, the audio graph is rendered in blocks of RENDER_QUANTUM_SIZE samples
/// see. <https://webaudio.github.io/web-audio-api/#render-quantum>
pub(crate) const RENDER_QUANTUM_SIZE: usize = 128;

/// Maximum number of channels for audio processing
pub const MAX_CHANNELS: usize = 32;

mod buffer;
pub use buffer::*;

mod capacity;
pub use capacity::*;

mod playback_stats;
pub use playback_stats::*;

pub mod context;

pub mod media_devices;
pub mod media_recorder;
pub mod media_streams;

pub mod node;

mod events;
pub use events::*;

mod message_port;
pub use message_port::MessagePort;

mod param;
pub use param::*;

mod periodic_wave;
pub use periodic_wave::*;

mod render;

mod stats;

mod spatial;
pub use spatial::AudioListener;

mod io;

mod analysis;
mod message;

mod decoding;

mod media_element;
pub use media_element::MediaElement;

mod resampling;
pub mod worklet;

#[repr(transparent)]
pub(crate) struct AtomicF32 {
    bits: AtomicU32,
}

impl std::fmt::Debug for AtomicF32 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{}", self.load(Ordering::Relaxed)))
    }
}

impl AtomicF32 {
    #[must_use]
    pub fn new(value: f32) -> Self {
        Self {
            bits: AtomicU32::new(value.to_bits()),
        }
    }

    #[must_use]
    pub fn load(&self, ordering: Ordering) -> f32 {
        f32::from_bits(self.bits.load(ordering))
    }

    pub fn store(&self, value: f32, ordering: Ordering) {
        self.bits.store(value.to_bits(), ordering);
    }
}

/// Atomic float 64, only `load` and `store` are supported, no arithmetic
#[repr(transparent)]
pub(crate) struct AtomicF64 {
    bits: AtomicU64,
}

impl std::fmt::Debug for AtomicF64 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{}", self.load(Ordering::Relaxed)))
    }
}

impl AtomicF64 {
    #[must_use]
    pub fn new(value: f64) -> Self {
        Self {
            bits: AtomicU64::new(value.to_bits()),
        }
    }

    #[must_use]
    pub fn load(&self, ordering: Ordering) -> f64 {
        f64::from_bits(self.bits.load(ordering))
    }

    pub fn store(&self, value: f64, ordering: Ordering) {
        self.bits.store(value.to_bits(), ordering);
    }
}

/// Assert that the given sample rate is valid.
///
/// Note that in practice sample rates should stand between 8000Hz (lower bound for
/// voice based applications, e.g. see phone bandwidth) and 96000Hz (for very high
/// quality audio applications and spectrum manipulation).
/// Most common sample rates for musical applications are 44100 and 48000.
///
/// The Web Audio API specification requires implementations to support every
/// sample rate in the inclusive range `[3000, 768000]`.
///
/// - see <https://webaudio.github.io/web-audio-api/#sample-rates>
///
/// # Panics
///
/// This function will panic if the given sample rate is outside the inclusive
/// range `[3000, 768000]`.
///
/// The minimum sample rate supported by the Web Audio API specification, in Hz.
pub(crate) const MIN_SAMPLE_RATE: f32 = 3_000.;

/// The maximum sample rate supported by the Web Audio API specification, in Hz.
pub(crate) const MAX_SAMPLE_RATE: f32 = 768_000.;

/// Returns whether the given sample rate falls in the inclusive range
/// `[MIN_SAMPLE_RATE, MAX_SAMPLE_RATE]` defined by the Web Audio API specification.
///
/// - see <https://webaudio.github.io/web-audio-api/#sample-rates>
#[inline(always)]
pub(crate) fn is_valid_sample_rate(sample_rate: f32) -> bool {
    (MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&sample_rate)
}

#[track_caller]
#[inline(always)]
pub(crate) fn assert_valid_sample_rate(sample_rate: f32) {
    assert!(
        is_valid_sample_rate(sample_rate),
        "NotSupportedError - Invalid sample rate: {:?}, should be in the range [{:?}, {:?}]",
        sample_rate,
        MIN_SAMPLE_RATE,
        MAX_SAMPLE_RATE,
    );
}

/// Assert that the given number of channels is valid.
///
/// # Panics
///
/// This function will panic if:
/// - the given number of channels is outside the [1, 32] range,
///   32 being defined by the MAX_CHANNELS constant.
///
#[track_caller]
#[inline(always)]
pub(crate) fn assert_valid_number_of_channels(number_of_channels: usize) {
    assert!(
        number_of_channels > 0 && number_of_channels <= MAX_CHANNELS,
        "NotSupportedError - Invalid number of channels: {:?} is outside range [1, {:?}]",
        number_of_channels,
        MAX_CHANNELS
    );
}

/// Assert that the given channel number is valid according to the number of channels
/// of an Audio asset (e.g. [`AudioBuffer`]).
///
/// # Panics
///
/// This function will panic if:
/// - the given channel number is greater than or equal to the given number of channels.
///
#[track_caller]
#[inline(always)]
pub(crate) fn assert_valid_channel_number(channel_number: usize, number_of_channels: usize) {
    assert!(
        channel_number < number_of_channels,
        "IndexSizeError - Invalid channel number {:?} (number of channels: {:?})",
        channel_number,
        number_of_channels
    );
}

/// Assert that the given value number is a valid buffer length, i.e. greater than zero
///
/// # Panics
///
/// This function will panic if:
/// - the given value is not lower than or equal to zero
///
#[track_caller]
#[inline(always)]
pub(crate) fn assert_valid_buffer_length(length: usize) {
    assert!(
        length > 0,
        "NotSupportedError - Invalid length: {:?} is less than or equal to minimum bound (0)",
        length,
    );
}

/// Assert that the given value number is a valid time information, i.e. greater
/// than or equal to zero and finite.
///
/// # Panics
///
/// This function will panic if:
/// - the given value is not finite and lower than zero
///
#[track_caller]
#[inline(always)]
pub(crate) fn assert_valid_time_value(value: f64) {
    assert!(
        value.is_finite(),
        "TypeError - The provided time value is non-finite.",
    );

    assert!(
        value >= 0.,
        "RangeError - The provided time value ({:?}) cannot be negative",
        value
    );
}

pub(crate) trait AudioBufferIter: Iterator<Item = FallibleBuffer> + Send + 'static {}

impl<M: Iterator<Item = FallibleBuffer> + Send + 'static> AudioBufferIter for M {}

type FallibleBuffer = Result<AudioBuffer, Box<dyn Error + Send + Sync>>;

#[cfg(test)]
mod tests {
    use float_eq::assert_float_eq;

    use super::*;

    #[test]
    fn test_atomic_f64() {
        let f = AtomicF64::new(2.0);
        assert_float_eq!(f.load(Ordering::SeqCst), 2.0, abs <= 0.);

        f.store(3.0, Ordering::SeqCst);
        assert_float_eq!(f.load(Ordering::SeqCst), 3.0, abs <= 0.);
    }

    #[test]
    fn test_valid_sample_rate() {
        assert_valid_sample_rate(48000.);
        assert_valid_sample_rate(3_000.);
        assert_valid_sample_rate(768_000.);
    }

    #[test]
    #[should_panic]
    fn test_invalid_sample_rate_too_small() {
        assert_valid_sample_rate(2_999.);
    }

    #[test]
    #[should_panic]
    fn test_invalid_sample_rate_too_big() {
        assert_valid_sample_rate(768_001.);
    }

    #[test]
    #[should_panic]
    fn test_invalid_number_of_channels_min() {
        assert_valid_number_of_channels(0);
    }

    #[test]
    #[should_panic]
    fn test_invalid_number_of_channels_max() {
        assert_valid_number_of_channels(33);
    }

    #[test]
    fn test_valid_number_of_channels() {
        assert_valid_number_of_channels(1);
        assert_valid_number_of_channels(32);
    }

    #[test]
    #[should_panic]
    fn test_invalid_time_value_non_finite() {
        assert_valid_time_value(f64::NAN);
    }

    #[test]
    #[should_panic]
    fn test_invalid_time_value_negative() {
        assert_valid_time_value(-1.);
    }

    #[test]
    fn test_valid_time_value() {
        assert_valid_time_value(0.);
        assert_valid_time_value(1.);
    }
}
