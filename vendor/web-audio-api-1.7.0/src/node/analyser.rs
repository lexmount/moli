use crate::analysis::{
    Analyser, AnalyserRingBuffer, DEFAULT_FFT_SIZE, DEFAULT_MAX_DECIBELS, DEFAULT_MIN_DECIBELS,
    DEFAULT_SMOOTHING_TIME_CONSTANT,
};
use crate::context::{AudioContextRegistration, BaseAudioContext};
use crate::render::{
    AudioParamValues, AudioProcessor, AudioRenderQuantum, AudioWorkletGlobalScope,
};

use super::{AudioNode, AudioNodeOptions, ChannelConfig, ChannelInterpretation};

/// Options for constructing an [`AnalyserNode`]
// dictionary AnalyserOptions : AudioNodeOptions {
//   unsigned long fftSize = 2048;
//   double maxDecibels = -30;
//   double minDecibels = -100;
//   double smoothingTimeConstant = 0.8;
// };
#[derive(Clone, Debug)]
pub struct AnalyserOptions {
    pub fft_size: usize,
    pub max_decibels: f64,
    pub min_decibels: f64,
    pub smoothing_time_constant: f64,
    pub audio_node_options: AudioNodeOptions,
}

impl Default for AnalyserOptions {
    fn default() -> Self {
        Self {
            fft_size: DEFAULT_FFT_SIZE,
            max_decibels: DEFAULT_MAX_DECIBELS,
            min_decibels: DEFAULT_MIN_DECIBELS,
            smoothing_time_constant: DEFAULT_SMOOTHING_TIME_CONSTANT,
            audio_node_options: AudioNodeOptions::default(),
        }
    }
}

/// `AnalyserNode` represents a node able to provide real-time frequency and
/// time-domain analysis information.
///
/// It is an AudioNode that passes the audio stream unchanged from the input to
/// the output, but allows you to take the generated data, process it, and create
/// audio visualizations..
///
/// - MDN documentation: <https://developer.mozilla.org/en-US/docs/Web/API/AnalyserNode>
/// - specification: <https://webaudio.github.io/web-audio-api/#AnalyserNode>
/// - see also: [`BaseAudioContext::create_analyser`]
///
/// # Usage
///
/// ```no_run
/// use web_audio_api::context::{BaseAudioContext, AudioContext};
/// use web_audio_api::node::{AudioNode, AudioScheduledSourceNode};
///
/// let context = AudioContext::default();
///
/// let mut analyser = context.create_analyser();
/// analyser.connect(&context.destination());
///
/// let mut osc = context.create_oscillator();
/// osc.frequency().set_value(200.);
/// osc.connect(&analyser);
/// osc.start();
///
/// let mut bins = vec![0.; analyser.frequency_bin_count()];
///
///
/// loop {
///     analyser.get_float_frequency_data(&mut bins);
///     println!("{:?}", &bins[0..20]); // print 20 first bins
///     std::thread::sleep(std::time::Duration::from_millis(1000));
/// }
/// ```
///
/// # Examples
///
/// - `cargo run --release --example analyser`
/// - `cd showcase/mic_playback && cargo run --release`
///
#[derive(Debug)]
pub struct AnalyserNode {
    registration: AudioContextRegistration,
    channel_config: ChannelConfig,
    analyser: Analyser,
}

impl AudioNode for AnalyserNode {
    fn registration(&self) -> &AudioContextRegistration {
        &self.registration
    }

    fn channel_config(&self) -> &ChannelConfig {
        &self.channel_config
    }

    fn number_of_inputs(&self) -> usize {
        1
    }

    fn number_of_outputs(&self) -> usize {
        1
    }
}

impl AnalyserNode {
    pub fn new<C: BaseAudioContext>(context: &C, options: AnalyserOptions) -> Self {
        context.base().register(move |registration| {
            let fft_size = options.fft_size;
            let smoothing_time_constant = options.smoothing_time_constant;
            let min_decibels = options.min_decibels;
            let max_decibels = options.max_decibels;

            let mut analyser = Analyser::new();
            analyser.set_fft_size(fft_size);
            analyser.set_smoothing_time_constant(smoothing_time_constant);
            analyser.set_decibels(min_decibels, max_decibels);

            let render = AnalyserRenderer {
                ring_buffer: analyser.get_ring_buffer_clone(),
            };

            let node = AnalyserNode {
                registration,
                channel_config: options.audio_node_options.into(),
                analyser,
            };

            (node, Box::new(render))
        })
    }

    /// The size of the FFT used for frequency-domain analysis (in sample-frames)
    ///
    /// # Panics
    ///
    /// This method may panic if the lock to the inner analyser is poisoned
    pub fn fft_size(&self) -> usize {
        self.analyser.fft_size()
    }

    /// Set FFT size
    ///
    /// # Panics
    ///
    /// This function panics if fft_size is not a power of two or not in the range [32, 32768]
    pub fn set_fft_size(&mut self, fft_size: usize) {
        self.analyser.set_fft_size(fft_size);
    }

    /// Time averaging parameter with the last analysis frame.
    /// A value from 0 -> 1 where 0 represents no time averaging with the last
    /// analysis frame. The default value is 0.8.
    ///
    /// # Panics
    ///
    /// This method may panic if the lock to the inner analyser is poisoned
    pub fn smoothing_time_constant(&self) -> f64 {
        self.analyser.smoothing_time_constant()
    }

    /// Set smoothing time constant
    ///
    /// # Panics
    ///
    /// This function panics if the value is set to a value less than 0 or more than 1.
    pub fn set_smoothing_time_constant(&mut self, value: f64) {
        self.analyser.set_smoothing_time_constant(value);
    }

    /// Minimum power value in the scaling range for the FFT analysis data for
    /// conversion to unsigned byte values. The default value is -100.
    ///
    /// # Panics
    ///
    /// This method may panic if the lock to the inner analyser is poisoned
    pub fn min_decibels(&self) -> f64 {
        self.analyser.min_decibels()
    }

    /// Set min decibels
    ///
    /// # Panics
    ///
    /// This function panics if the value is set to a value more than or equal
    /// to max decibels.
    pub fn set_min_decibels(&mut self, value: f64) {
        self.analyser.set_decibels(value, self.max_decibels());
    }

    /// Maximum power value in the scaling range for the FFT analysis data for
    /// conversion to unsigned byte values. The default value is -30.
    ///
    /// # Panics
    ///
    /// This method may panic if the lock to the inner analyser is poisoned
    pub fn max_decibels(&self) -> f64 {
        self.analyser.max_decibels()
    }

    /// Set max decibels
    ///
    /// # Panics
    ///
    /// This function panics if the value is set to a value less than or equal
    /// to min decibels.
    pub fn set_max_decibels(&mut self, value: f64) {
        self.analyser.set_decibels(self.min_decibels(), value);
    }

    /// Number of bins in the FFT results, is half the FFT size
    ///
    /// # Panics
    ///
    /// This method may panic if the lock to the inner analyser is poisoned
    pub fn frequency_bin_count(&self) -> usize {
        self.analyser.frequency_bin_count()
    }

    /// Copy the current time domain data as f32 values into the provided buffer
    ///
    /// # Panics
    ///
    /// This method may panic if the lock to the inner analyser is poisoned
    pub fn get_float_time_domain_data(&mut self, buffer: &mut [f32]) {
        self.analyser.get_float_time_domain_data(buffer);
    }

    /// Copy the current time domain data as u8 values into the provided buffer
    ///
    /// # Panics
    ///
    /// This method may panic if the lock to the inner analyser is poisoned
    pub fn get_byte_time_domain_data(&mut self, buffer: &mut [u8]) {
        self.analyser.get_byte_time_domain_data(buffer);
    }

    /// Copy the current frequency data into the provided buffer
    ///
    /// # Panics
    ///
    /// This method may panic if the lock to the inner analyser is poisoned
    pub fn get_float_frequency_data(&mut self, buffer: &mut [f32]) {
        let current_time = self.registration.context().current_time();
        self.analyser.get_float_frequency_data(buffer, current_time);
    }

    /// Copy the current frequency data scaled between min_decibels and
    /// max_decibels into the provided buffer
    ///
    /// # Panics
    ///
    /// This method may panic if the lock to the inner analyser is poisoned
    pub fn get_byte_frequency_data(&mut self, buffer: &mut [u8]) {
        let current_time = self.registration.context().current_time();
        self.analyser.get_byte_frequency_data(buffer, current_time);
    }
}

struct AnalyserRenderer {
    ring_buffer: AnalyserRingBuffer,
}

impl AudioProcessor for AnalyserRenderer {
    fn process(
        &mut self,
        inputs: &[AudioRenderQuantum],
        outputs: &mut [AudioRenderQuantum],
        _params: AudioParamValues<'_>,
        _scope: &AudioWorkletGlobalScope,
    ) -> bool {
        // single input/output node
        let input = &inputs[0];
        let output = &mut outputs[0];

        // pass through input
        *output = input.clone();

        // down mix to mono
        let mut mono = input.clone();
        mono.mix(1, ChannelInterpretation::Speakers);

        // add current input to ring buffer
        let data = mono.channel_data(0).as_ref();
        self.ring_buffer.write(data);

        // no tail-time
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{
        AudioContext, AudioContextOptions, BaseAudioContext, OfflineAudioContext,
    };
    use crate::node::{AudioNode, AudioScheduledSourceNode};
    use float_eq::assert_float_eq;

    #[test]
    fn test_analyser_after_closed() {
        let options = AudioContextOptions {
            sink_id: "none".into(),
            ..AudioContextOptions::default()
        };
        let context = AudioContext::new(options);

        let mut src = context.create_constant_source();
        src.start();

        let mut analyser = context.create_analyser();
        src.connect(&analyser);

        // allow buffer to fill
        std::thread::sleep(std::time::Duration::from_millis(20));

        let mut buffer = vec![0.; 128];
        analyser.get_float_time_domain_data(&mut buffer);
        assert_float_eq!(&buffer[..], &[1.; 128][..], abs_all <= 0.); // constant source of 1.

        // close context
        context.close_sync();
        std::thread::sleep(std::time::Duration::from_millis(50));

        let mut buffer = vec![0.; 128];
        analyser.get_float_time_domain_data(&mut buffer); // should not crash or hang

        // should contain the most recent frames available
        assert_float_eq!(&buffer[..], &[1.; 128][..], abs_all <= 0.);
    }

    #[test]
    fn test_construct_decibels() {
        let context = OfflineAudioContext::new(1, 128, 44_100.);
        let options = AnalyserOptions {
            min_decibels: -10.,
            max_decibels: 20.,
            ..AnalyserOptions::default()
        };
        let _ = AnalyserNode::new(&context, options);
    }
}
