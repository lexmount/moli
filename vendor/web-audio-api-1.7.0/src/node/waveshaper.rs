use std::any::Any;

use rubato::{FftFixedInOut, Resampler as _};

use crate::{
    context::{AudioContextRegistration, BaseAudioContext},
    render::{AudioParamValues, AudioProcessor, AudioRenderQuantum, AudioWorkletGlobalScope},
    RENDER_QUANTUM_SIZE,
};

use super::{AudioNode, AudioNodeOptions, ChannelConfig};

/// enumerates the oversampling rate available for `WaveShaperNode`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// the naming comes from the web audio specification
#[derive(Default)]
pub enum OverSampleType {
    /// No oversampling is applied
    #[default]
    None,
    /// Oversampled by a factor of 2
    X2,
    /// Oversampled by a factor of 4
    X4,
}

impl From<u32> for OverSampleType {
    fn from(i: u32) -> Self {
        match i {
            0 => OverSampleType::None,
            1 => OverSampleType::X2,
            2 => OverSampleType::X4,
            _ => unreachable!(),
        }
    }
}

/// `WaveShaperNode` options
// dictionary WaveShaperOptions : AudioNodeOptions {
//   sequence<float> curve;
//   OverSampleType oversample = "none";
// };
#[derive(Clone, Debug)]
pub struct WaveShaperOptions {
    /// The distortion curve
    pub curve: Option<Vec<f32>>,
    /// Oversampling rate - default to `None`
    pub oversample: OverSampleType,
    /// audio node options
    pub audio_node_options: AudioNodeOptions,
}

impl Default for WaveShaperOptions {
    fn default() -> Self {
        Self {
            oversample: OverSampleType::None,
            curve: None,
            audio_node_options: AudioNodeOptions::default(),
        }
    }
}

/// `WaveShaperNode` allows to apply non-linear distortion effect on a audio
/// signal. Arbitrary non-linear shaping curves may be specified.
///
/// - MDN documentation: <https://developer.mozilla.org/en-US/docs/Web/API/WaveShaperNode>
/// - specification: <https://webaudio.github.io/web-audio-api/#WaveShaperNode>
/// - see also: [`BaseAudioContext::create_wave_shaper`]
///
/// # Usage
///
/// ```no_run
/// use std::fs::File;
/// use web_audio_api::context::{BaseAudioContext, AudioContext};
/// use web_audio_api::node::{AudioNode, AudioScheduledSourceNode};
///
/// # use std::f32::consts::PI;
/// # fn make_distortion_curve(size: usize) -> Vec<f32> {
/// #     let mut curve = vec![0.; size];
/// #     let mut phase = 0.;
/// #     let phase_incr = PI / (size - 1) as f32;
/// #     for i in 0..size {
/// #         curve[i] = (PI + phase).cos();
/// #         phase += phase_incr;
/// #     }
/// #     curve
/// # }
/// let context = AudioContext::default();
///
/// let file = File::open("sample.wav").unwrap();
/// let buffer = context.decode_audio_data_sync(file).unwrap();
/// let curve = make_distortion_curve(2048);
/// let drive = 4.;
///
/// let post_gain = context.create_gain();
/// post_gain.connect(&context.destination());
/// post_gain.gain().set_value(1. / drive);
///
/// let mut shaper = context.create_wave_shaper();
/// shaper.connect(&post_gain);
/// shaper.set_curve(curve);
///
/// let pre_gain = context.create_gain();
/// pre_gain.connect(&shaper);
/// pre_gain.gain().set_value(drive);
///
/// let mut src = context.create_buffer_source();
/// src.connect(&pre_gain);
/// src.set_buffer(buffer);
///
/// src.start();
/// ```
///
/// # Example
///
/// - `cargo run --release --example waveshaper`
#[derive(Debug)]
pub struct WaveShaperNode {
    /// Represents the node instance and its associated audio context
    registration: AudioContextRegistration,
    /// Infos about audio node channel configuration
    channel_config: ChannelConfig,
    /// distortion curve
    curve: Option<Vec<f32>>,
    /// oversample type
    oversample: OverSampleType,
}

impl AudioNode for WaveShaperNode {
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

impl WaveShaperNode {
    /// returns a `WaveShaperNode` instance
    ///
    /// # Arguments
    ///
    /// * `context` - audio context in which the audio node will live.
    /// * `options` - waveshaper options
    pub fn new<C: BaseAudioContext>(context: &C, options: WaveShaperOptions) -> Self {
        let WaveShaperOptions {
            oversample,
            curve,
            audio_node_options: channel_config,
        } = options;

        let mut node = context.base().register(move |registration| {
            let sample_rate = context.sample_rate() as usize;

            let renderer = WaveShaperRenderer::new(RendererConfig {
                oversample,
                sample_rate,
            });

            let node = Self {
                registration,
                channel_config: channel_config.into(),
                curve: None,
                oversample,
            };

            (node, Box::new(renderer))
        });

        // renderer has been sent to render thread, we can sent it messages
        if let Some(curve) = curve {
            node.set_curve(curve);
        }

        node
    }

    /// Returns the distortion curve
    #[must_use]
    pub fn curve(&self) -> Option<&[f32]> {
        self.curve.as_deref()
    }

    /// Set the distortion `curve` of this node
    ///
    /// # Arguments
    ///
    /// * `curve` - the desired distortion `curve`
    ///
    /// # Panics
    ///
    /// Panics if a curve has already been given to the source (though `new` or through
    /// `set_curve`)
    pub fn set_curve(&mut self, curve: Vec<f32>) {
        assert!(
            self.curve.is_none(),
            "InvalidStateError - cannot assign curve twice",
        );

        let clone = curve.clone();

        self.curve = Some(curve);
        self.registration.post_message(Some(clone));
    }

    /// Returns the `oversample` faactor of this node
    #[must_use]
    pub fn oversample(&self) -> OverSampleType {
        self.oversample
    }

    /// set the `oversample` factor of this node
    ///
    /// # Arguments
    ///
    /// * `oversample` - the desired `OversampleType` variant
    pub fn set_oversample(&mut self, oversample: OverSampleType) {
        self.oversample = oversample;
        self.registration.post_message(oversample);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResamplerConfig {
    channels: usize,
    chunk_size_in: usize,
    sample_rate_in: usize,
    sample_rate_out: usize,
}

impl ResamplerConfig {
    fn upsample_x2(channels: usize, sample_rate: usize) -> Self {
        let chunk_size_in = RENDER_QUANTUM_SIZE;
        let sample_rate_in = sample_rate;
        let sample_rate_out = sample_rate * 2;
        Self {
            channels,
            chunk_size_in,
            sample_rate_in,
            sample_rate_out,
        }
    }

    fn upsample_x4(channels: usize, sample_rate: usize) -> Self {
        let chunk_size_in = RENDER_QUANTUM_SIZE;
        let sample_rate_in = sample_rate;
        let sample_rate_out = sample_rate * 4;
        Self {
            channels,
            chunk_size_in,
            sample_rate_in,
            sample_rate_out,
        }
    }

    fn downsample_x2(channels: usize, sample_rate: usize) -> Self {
        let chunk_size_in = RENDER_QUANTUM_SIZE * 2;
        let sample_rate_in = sample_rate * 2;
        let sample_rate_out = sample_rate;
        Self {
            channels,
            chunk_size_in,
            sample_rate_in,
            sample_rate_out,
        }
    }

    fn downsample_x4(channels: usize, sample_rate: usize) -> Self {
        let chunk_size_in = RENDER_QUANTUM_SIZE * 4;
        let sample_rate_in = sample_rate * 4;
        let sample_rate_out = sample_rate;
        Self {
            channels,
            chunk_size_in,
            sample_rate_in,
            sample_rate_out,
        }
    }
}

struct Resampler {
    config: ResamplerConfig,
    processor: FftFixedInOut<f32>,
    samples_out: Vec<Vec<f32>>,
}

impl Resampler {
    fn new(config: ResamplerConfig) -> Self {
        let ResamplerConfig {
            channels,
            chunk_size_in,
            sample_rate_in,
            sample_rate_out,
        } = &config;

        let processor =
            FftFixedInOut::new(*sample_rate_in, *sample_rate_out, *chunk_size_in, *channels)
                .unwrap();

        let samples_out = processor.output_buffer_allocate(true);

        Self {
            config,
            processor,
            samples_out,
        }
    }

    fn process<T>(&mut self, samples_in: &[T])
    where
        T: AsRef<[f32]>,
    {
        debug_assert_eq!(self.config.channels, samples_in.len());
        // Processing the output from another resampler directly as input requires this assumption.
        debug_assert!(samples_in
            .iter()
            .all(|channel| channel.as_ref().len() == self.processor.input_frames_next()));
        let (in_len, out_len) = self
            .processor
            .process_into_buffer(samples_in, &mut self.samples_out[..], None)
            .unwrap();
        // All input samples must have been consumed.
        debug_assert_eq!(in_len, samples_in[0].as_ref().len());
        // All output samples must have been initialized.
        debug_assert!(self
            .samples_out
            .iter()
            .all(|channel| channel.len() == out_len));
    }

    fn samples_out(&self) -> &[Vec<f32>] {
        &self.samples_out[..]
    }

    fn samples_out_mut(&mut self) -> &mut [Vec<f32>] {
        &mut self.samples_out[..]
    }
}

/// Helper struct which regroups all parameters
/// required to build `WaveShaperRenderer`
struct RendererConfig {
    /// oversample factor
    oversample: OverSampleType,
    /// Sample rate (equals to audio context sample rate)
    sample_rate: usize,
}

/// `WaveShaperRenderer` represents the rendering part of `WaveShaperNode`
struct WaveShaperRenderer {
    /// oversample factor
    oversample: OverSampleType,
    /// distortion curve
    curve: Option<Vec<f32>>,
    /// Sample rate (equals to audio context sample rate)
    sample_rate: usize,
    /// Number of channels used to build the up/down sampler X2
    channels_x2: usize,
    /// Number of channels used to build the up/down sampler X4
    channels_x4: usize,
    // up sampler configured to multiply by 2 the input signal
    upsampler_x2: Resampler,
    // up sampler configured to multiply by 4 the input signal
    upsampler_x4: Resampler,
    // down sampler configured to divide by 2 the upsampled signal
    downsampler_x2: Resampler,
    // down sampler configured to divide by 4 the upsampled signal
    downsampler_x4: Resampler,
    // check if silence can be propagated, i.e. if curve if None or if
    // it's output value for zero signal is zero (i.e. < 1e-9)
    can_propagate_silence: bool,
}

impl AudioProcessor for WaveShaperRenderer {
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

        if input.is_silent() && self.can_propagate_silence {
            output.make_silent();
            return false;
        }

        *output = input.clone();

        if let Some(curve) = &self.curve {
            match self.oversample {
                OverSampleType::None => {
                    output.modify_channels(|channel| {
                        channel.iter_mut().for_each(|o| *o = apply_curve(curve, *o));
                    });
                }
                OverSampleType::X2 => {
                    let channels = output.channels();

                    // recreate up/down sampler if number of channels changed
                    if channels.len() != self.channels_x2 {
                        self.channels_x2 = channels.len();

                        self.upsampler_x2 = Resampler::new(ResamplerConfig::upsample_x2(
                            self.channels_x2,
                            self.sample_rate,
                        ));

                        self.downsampler_x2 = Resampler::new(ResamplerConfig::downsample_x2(
                            self.channels_x2,
                            self.sample_rate,
                        ));
                    }

                    self.upsampler_x2.process(channels);
                    for channel in self.upsampler_x2.samples_out_mut().iter_mut() {
                        for s in channel.iter_mut() {
                            *s = apply_curve(curve, *s);
                        }
                    }

                    self.downsampler_x2.process(self.upsampler_x2.samples_out());

                    for (processed, output) in self
                        .downsampler_x2
                        .samples_out()
                        .iter()
                        .zip(output.channels_mut())
                    {
                        output.copy_from_slice(&processed[..]);
                    }
                }
                OverSampleType::X4 => {
                    let channels = output.channels();

                    // recreate up/down sampler if number of channels changed
                    if channels.len() != self.channels_x4 {
                        self.channels_x4 = channels.len();

                        self.upsampler_x4 = Resampler::new(ResamplerConfig::upsample_x4(
                            self.channels_x4,
                            self.sample_rate,
                        ));

                        self.downsampler_x4 = Resampler::new(ResamplerConfig::downsample_x4(
                            self.channels_x4,
                            self.sample_rate,
                        ));
                    }

                    self.upsampler_x4.process(channels);

                    for channel in self.upsampler_x4.samples_out_mut().iter_mut() {
                        for s in channel.iter_mut() {
                            *s = apply_curve(curve, *s);
                        }
                    }

                    self.downsampler_x4.process(self.upsampler_x4.samples_out());

                    for (processed, output) in self
                        .downsampler_x4
                        .samples_out()
                        .iter()
                        .zip(output.channels_mut())
                    {
                        output.copy_from_slice(&processed[..]);
                    }
                }
            }
        }

        // @tbc - rubato::FftFixedInOut doesn't seem to introduce any latency
        false
    }

    fn onmessage(&mut self, msg: &mut dyn Any) {
        if let Some(&oversample) = msg.downcast_ref::<OverSampleType>() {
            self.oversample = oversample;
            return;
        }

        if let Some(curve) = msg.downcast_mut::<Option<Vec<f32>>>() {
            std::mem::swap(&mut self.curve, curve);

            // We can propagate silent input only if the center of the curve is zero
            self.can_propagate_silence = if let Some(curve) = &self.curve {
                if curve.len() % 2 == 1 {
                    curve[curve.len() / 2].abs() < 1e-9
                } else {
                    let a = curve[curve.len() / 2 - 1];
                    let b = curve[curve.len() / 2];
                    ((a + b) / 2.).abs() < 1e-9
                }
            } else {
                true
            };

            return;
        }

        log::warn!("WaveShaperRenderer: Dropping incoming message {msg:?}");
    }
}

impl WaveShaperRenderer {
    /// returns an `WaveShaperRenderer` instance
    #[allow(clippy::missing_const_for_fn)]
    fn new(config: RendererConfig) -> Self {
        let RendererConfig {
            sample_rate,
            oversample,
        } = config;

        let channels_x2 = 1;
        let channels_x4 = 1;

        let upsampler_x2 = Resampler::new(ResamplerConfig::upsample_x2(channels_x2, sample_rate));

        let downsampler_x2 =
            Resampler::new(ResamplerConfig::downsample_x2(channels_x2, sample_rate));

        let upsampler_x4 = Resampler::new(ResamplerConfig::upsample_x4(channels_x2, sample_rate));

        let downsampler_x4 =
            Resampler::new(ResamplerConfig::downsample_x4(channels_x2, sample_rate));

        Self {
            oversample,
            curve: None,
            sample_rate,
            channels_x2,
            channels_x4,
            upsampler_x2,
            upsampler_x4,
            downsampler_x2,
            downsampler_x4,
            can_propagate_silence: true,
        }
    }
}

#[inline]
fn apply_curve(curve: &[f32], input: f32) -> f32 {
    if curve.is_empty() {
        return 0.;
    }

    let n = curve.len() as f32;
    let v = (n - 1.) / 2.0 * (input + 1.);

    if v <= 0. {
        curve[0]
    } else if v >= n - 1. {
        curve[(n - 1.) as usize]
    } else {
        let k = v.floor();
        let f = v - k;
        (1. - f) * curve[k as usize] + f * curve[(k + 1.) as usize]
    }
}

#[cfg(test)]
mod tests {
    use float_eq::assert_float_eq;

    use crate::context::OfflineAudioContext;
    use crate::node::AudioScheduledSourceNode;

    use super::*;

    const LENGTH: usize = 555;

    #[test]
    fn build_with_new() {
        let context = OfflineAudioContext::new(2, LENGTH, 44_100.);
        let _shaper = WaveShaperNode::new(&context, WaveShaperOptions::default());
    }

    #[test]
    fn build_with_factory_func() {
        let context = OfflineAudioContext::new(2, LENGTH, 44_100.);
        let _shaper = context.create_wave_shaper();
    }

    #[test]
    fn test_default_options() {
        let context = OfflineAudioContext::new(2, LENGTH, 44_100.);
        let shaper = WaveShaperNode::new(&context, WaveShaperOptions::default());

        assert_eq!(shaper.curve(), None);
        assert_eq!(shaper.oversample(), OverSampleType::None);
    }

    #[test]
    fn test_user_defined_options() {
        let mut context = OfflineAudioContext::new(2, LENGTH, 44_100.);

        let options = WaveShaperOptions {
            curve: Some(vec![1.0]),
            oversample: OverSampleType::X2,
            ..Default::default()
        };

        let shaper = WaveShaperNode::new(&context, options);

        let _ = context.start_rendering_sync();

        assert_eq!(shaper.curve(), Some(&[1.0][..]));
        assert_eq!(shaper.oversample(), OverSampleType::X2);
    }

    #[test]
    #[should_panic]
    fn change_a_curve_for_another_curve_should_panic() {
        let mut context = OfflineAudioContext::new(2, LENGTH, 44_100.);

        let options = WaveShaperOptions {
            curve: Some(vec![1.0]),
            oversample: OverSampleType::X2,
            ..Default::default()
        };

        let mut shaper = WaveShaperNode::new(&context, options);
        assert_eq!(shaper.curve(), Some(&[1.0][..]));
        assert_eq!(shaper.oversample(), OverSampleType::X2);

        shaper.set_curve(vec![2.0]);
        shaper.set_oversample(OverSampleType::X4);

        let _ = context.start_rendering_sync();

        assert_eq!(shaper.curve(), Some(&[2.0][..]));
        assert_eq!(shaper.oversample(), OverSampleType::X4);
    }

    #[test]
    fn change_none_for_curve_after_build() {
        let mut context = OfflineAudioContext::new(2, LENGTH, 44_100.);

        let options = WaveShaperOptions {
            curve: None,
            oversample: OverSampleType::X2,
            ..Default::default()
        };

        let mut shaper = WaveShaperNode::new(&context, options);
        assert_eq!(shaper.curve(), None);
        assert_eq!(shaper.oversample(), OverSampleType::X2);

        shaper.set_curve(vec![2.0]);
        shaper.set_oversample(OverSampleType::X4);

        let _ = context.start_rendering_sync();

        assert_eq!(shaper.curve(), Some(&[2.0][..]));
        assert_eq!(shaper.oversample(), OverSampleType::X4);
    }

    #[test]
    fn test_shape_boundaries() {
        let sample_rate = 44100.;
        let mut context = OfflineAudioContext::new(1, 3 * RENDER_QUANTUM_SIZE, sample_rate);

        let mut shaper = context.create_wave_shaper();
        let curve = vec![-0.5, 0., 0.5];
        shaper.set_curve(curve);
        shaper.connect(&context.destination());

        let mut data = vec![0.; 3 * RENDER_QUANTUM_SIZE];
        let mut expected = vec![0.; 3 * RENDER_QUANTUM_SIZE];
        for i in 0..(3 * RENDER_QUANTUM_SIZE) {
            if i < RENDER_QUANTUM_SIZE {
                data[i] = -1.;
                expected[i] = -0.5;
            } else if i < 2 * RENDER_QUANTUM_SIZE {
                data[i] = 0.;
                expected[i] = 0.;
            } else {
                data[i] = 1.;
                expected[i] = 0.5;
            }
        }
        let mut buffer = context.create_buffer(1, 3 * RENDER_QUANTUM_SIZE, sample_rate);
        buffer.copy_to_channel(&data, 0);

        let mut src = context.create_buffer_source();
        src.connect(&shaper);
        src.set_buffer(buffer);
        src.start_at(0.);

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }

    #[test]
    fn test_shape_interpolation() {
        let sample_rate = 44100.;
        let mut context = OfflineAudioContext::new(1, RENDER_QUANTUM_SIZE, sample_rate);

        let mut shaper = context.create_wave_shaper();
        let curve = vec![-0.5, 0., 0.5];
        shaper.set_curve(curve);
        shaper.connect(&context.destination());

        let mut data = vec![0.; RENDER_QUANTUM_SIZE];
        let mut expected = vec![0.; RENDER_QUANTUM_SIZE];

        for i in 0..RENDER_QUANTUM_SIZE {
            let sample = i as f32 / (RENDER_QUANTUM_SIZE as f32) * 2. - 1.;
            data[i] = sample;
            expected[i] = sample / 2.;
        }

        let mut buffer = context.create_buffer(1, 3 * RENDER_QUANTUM_SIZE, sample_rate);
        buffer.copy_to_channel(&data, 0);

        let mut src = context.create_buffer_source();
        src.connect(&shaper);
        src.set_buffer(buffer);
        src.start_at(0.);

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }
}
