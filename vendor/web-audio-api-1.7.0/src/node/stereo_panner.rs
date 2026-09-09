//! The stereo panner control and renderer parts
use std::f32::consts::PI;

use crate::context::{AudioContextRegistration, AudioParamId, BaseAudioContext};
use crate::param::{AudioParam, AudioParamDescriptor};
use crate::render::{
    AudioParamValues, AudioProcessor, AudioRenderQuantum, AudioWorkletGlobalScope,
};

use super::{AudioNode, AudioNodeOptions, ChannelConfig, ChannelCountMode, ChannelInterpretation};

/// Options for constructing a [`StereoPannerOptions`]
// dictionary StereoPannerOptions : AudioNodeOptions {
//   float pan = 0;
// };
#[derive(Clone, Debug)]
pub struct StereoPannerOptions {
    /// initial value for the pan parameter
    pub pan: f32,
    /// audio node options
    pub audio_node_options: AudioNodeOptions,
}

impl Default for StereoPannerOptions {
    fn default() -> Self {
        Self {
            pan: 0.,
            audio_node_options: AudioNodeOptions {
                channel_count: 2,
                channel_count_mode: ChannelCountMode::ClampedMax,
                channel_interpretation: ChannelInterpretation::Speakers,
            },
        }
    }
}

/// Assert that the channel count is valid for the StereoPannerNode
/// see <https://webaudio.github.io/web-audio-api/#audionode-channelcount-constraints>
///
/// # Panics
///
/// This function panics if given count is greater than 2
///
#[track_caller]
#[inline(always)]
fn assert_valid_channel_count(count: usize) {
    assert!(
        count <= 2,
        "NotSupportedError - StereoPannerNode channel count cannot be greater than two"
    );
}

/// Assert that the channel count is valid for the StereoPannerNode
/// see <https://webaudio.github.io/web-audio-api/#audionode-channelcountmode-constraints>
///
/// # Panics
///
/// This function panics if given count mode is [`ChannelCountMode::Max`]
///
#[track_caller]
#[inline(always)]
fn assert_valid_channel_count_mode(mode: ChannelCountMode) {
    assert_ne!(
        mode,
        ChannelCountMode::Max,
        "NotSupportedError - StereoPannerNode channel count mode cannot be set to max",
    );
}

/// Generates the stereo gains for a specific x ∈ [0, 1] derived from pan:
/// - `gain_left = (x * PI / 2.).cos()`
/// - `gain_right = (x * PI / 2.).sin()`
#[inline(always)]
fn get_stereo_gains(x: f32) -> [f32; 2] {
    let gain_left = ((1. - x) * PI / 2.).sin(); // more accurate than cos()
    let gain_right = (x * PI / 2.).sin();

    [gain_left, gain_right]
}

/// `StereoPannerNode` positions an incoming audio stream in a stereo image
///
/// It is an audio-processing module that positions an incoming audio stream
/// in a stereo image using a low-cost panning algorithm.
///
/// - MDN documentation: <https://developer.mozilla.org/en-US/docs/Web/API/StereoPannerNode>
/// - specification: <https://webaudio.github.io/web-audio-api/#stereopannernode>
/// - see also: [`BaseAudioContext::create_stereo_panner`]
///
/// # Usage
///
/// ```no_run
/// use web_audio_api::context::{BaseAudioContext, AudioContext};
/// use web_audio_api::node::{AudioNode, AudioScheduledSourceNode};
///
/// // create an `AudioContext`
/// let context = AudioContext::default();
/// // load and decode a soundfile
/// let panner = context.create_stereo_panner();
/// panner.connect(&context.destination());
/// // position source on the left
/// panner.pan().set_value(-1.);
///
/// // pipe an oscillator into the stereo panner
/// let mut osc = context.create_oscillator();
/// osc.frequency().set_value(200.);
/// osc.connect(&panner);
/// osc.start();
/// ```
///
/// # Examples
///
/// - `cargo run --release --example stereo_panner`
///
#[derive(Debug)]
pub struct StereoPannerNode {
    /// Represents the node instance and its associated audio context
    registration: AudioContextRegistration,
    /// Infos about audio node channel configuration
    channel_config: ChannelConfig,
    /// The position of the input in the output’s stereo image. -1 represents
    /// full left, +1 represents full right.
    pan: AudioParam,
}

impl AudioNode for StereoPannerNode {
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

    fn set_channel_count_mode(&self, mode: ChannelCountMode) {
        assert_valid_channel_count_mode(mode);
        self.channel_config
            .set_count_mode(mode, self.registration());
    }

    fn set_channel_count(&self, count: usize) {
        assert_valid_channel_count(count);
        self.channel_config.set_count(count, self.registration());
    }
}

impl StereoPannerNode {
    /// returns a `StereoPannerNode` instance
    ///
    /// # Arguments
    ///
    /// * `context` - audio context in which the audio node will live.
    /// * `options` - stereo panner options
    ///
    /// # Panics
    ///
    /// Will panic if:
    ///
    /// * `options.channel_config.count` is greater than 2
    /// * `options.channel_config.mode` is `ChannelCountMode::Max`
    ///
    pub fn new<C: BaseAudioContext>(context: &C, options: StereoPannerOptions) -> Self {
        context.base().register(move |registration| {
            assert_valid_channel_count_mode(options.audio_node_options.channel_count_mode);
            assert_valid_channel_count(options.audio_node_options.channel_count);

            let pan_options = AudioParamDescriptor {
                name: String::new(),
                min_value: -1.,
                max_value: 1.,
                default_value: 0.,
                automation_rate: crate::param::AutomationRate::A,
            };
            let (pan_param, pan_proc) = context.create_audio_param(pan_options, &registration);

            pan_param.set_value(options.pan);

            let renderer = StereoPannerRenderer::new(pan_proc);

            let node = Self {
                registration,
                channel_config: options.audio_node_options.into(),
                pan: pan_param,
            };

            (node, Box::new(renderer))
        })
    }

    /// Returns the pan audio parameter
    #[must_use]
    pub fn pan(&self) -> &AudioParam {
        &self.pan
    }
}

/// `StereoPannerRenderer` represents the rendering part of `StereoPannerNode`
struct StereoPannerRenderer {
    /// Position of the input in the output’s stereo image.
    /// -1 represents full left, +1 represents full right.
    pan: AudioParamId,
}

impl StereoPannerRenderer {
    fn new(pan: AudioParamId) -> Self {
        Self { pan }
    }
}

impl AudioProcessor for StereoPannerRenderer {
    fn process(
        &mut self,
        inputs: &[AudioRenderQuantum],
        outputs: &mut [AudioRenderQuantum],
        params: AudioParamValues<'_>,
        _scope: &AudioWorkletGlobalScope,
    ) -> bool {
        // single input/output node
        let input = &inputs[0];
        let output = &mut outputs[0];

        if input.is_silent() {
            output.make_silent();
            return false;
        }

        output.set_number_of_channels(2);

        // a-rate param
        let pan_values = params.get(&self.pan);

        let [left, right] = output.stereo_mut();

        match input.number_of_channels() {
            0 => (),
            1 => {
                if pan_values.len() == 1 {
                    let pan = pan_values[0];
                    let x = (pan + 1.) * 0.5;
                    let [gain_left, gain_right] = get_stereo_gains(x);

                    left.iter_mut()
                        .zip(right.iter_mut())
                        .zip(input.channel_data(0).iter())
                        .for_each(|((l, r), input)| {
                            *l = input * gain_left;
                            *r = input * gain_right;
                        });
                } else {
                    left.iter_mut()
                        .zip(right.iter_mut())
                        .zip(pan_values.iter())
                        .zip(input.channel_data(0).iter())
                        .for_each(|(((l, r), pan), input)| {
                            let x = (pan + 1.) * 0.5;
                            let [gain_left, gain_right] = get_stereo_gains(x);

                            *l = input * gain_left;
                            *r = input * gain_right;
                        });
                }
            }
            2 => {
                if pan_values.len() == 1 {
                    let pan = pan_values[0];
                    let x = if pan <= 0. { pan + 1. } else { pan };
                    let [gain_left, gain_right] = get_stereo_gains(x);

                    left.iter_mut()
                        .zip(right.iter_mut())
                        .zip(input.channel_data(0).iter())
                        .zip(input.channel_data(1).iter())
                        .for_each(|(((l, r), &input_left), &input_right)| {
                            if pan <= 0. {
                                *l = input_right.mul_add(gain_left, input_left);
                                *r = input_right * gain_right;
                            } else {
                                *l = input_left * gain_left;
                                *r = input_left.mul_add(gain_right, input_right);
                            }
                        });
                } else {
                    left.iter_mut()
                        .zip(right.iter_mut())
                        .zip(pan_values.iter())
                        .zip(input.channel_data(0).iter())
                        .zip(input.channel_data(1).iter())
                        .for_each(|((((l, r), &pan), &input_left), &input_right)| {
                            if pan <= 0. {
                                let x = pan + 1.;
                                let [gain_left, gain_right] = get_stereo_gains(x);

                                *l = input_right.mul_add(gain_left, input_left);
                                *r = input_right * gain_right;
                            } else {
                                let x = pan;
                                let [gain_left, gain_right] = get_stereo_gains(x);

                                *l = input_left * gain_left;
                                *r = input_left.mul_add(gain_right, input_right);
                            }
                        });
                }
            }
            _ => panic!("StereoPannerNode should not have more than 2 channels to process"),
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use float_eq::assert_float_eq;

    use crate::context::{BaseAudioContext, OfflineAudioContext};
    use crate::node::AudioScheduledSourceNode;

    use super::*;

    #[test]
    fn test_constructor() {
        {
            let context = OfflineAudioContext::new(2, 1, 44_100.);
            let _panner = StereoPannerNode::new(&context, StereoPannerOptions::default());
        }

        {
            let context = OfflineAudioContext::new(2, 1, 44_100.);
            let _panner = context.create_stereo_panner();
        }

        {
            let context = OfflineAudioContext::new(2, 1, 44_100.);
            let panner = StereoPannerNode::new(&context, StereoPannerOptions::default());

            let default_pan = 0.;
            let pan = panner.pan.value();
            assert_float_eq!(pan, default_pan, abs_all <= 0.);
        }
    }

    #[test]
    fn test_init_with_channel_count_mode() {
        let context = OfflineAudioContext::new(2, 1, 44_100.);
        let options = StereoPannerOptions {
            audio_node_options: AudioNodeOptions {
                channel_count_mode: ChannelCountMode::Explicit,
                ..AudioNodeOptions::default()
            },
            ..StereoPannerOptions::default()
        };
        let panner = StereoPannerNode::new(&context, options);
        assert_eq!(
            panner.channel_config().count_mode(),
            ChannelCountMode::Explicit
        );
        assert_eq!(panner.channel_count_mode(), ChannelCountMode::Explicit);
    }

    #[test]
    fn test_mono_panning() {
        let sample_rate = 44_100.;

        let context = OfflineAudioContext::new(2, 128, 44_100.);

        let mut buffer = context.create_buffer(1, 128, sample_rate);
        buffer.copy_to_channel(&[1.; 128], 0);

        // left
        {
            let mut context = OfflineAudioContext::new(2, 128, 44_100.);
            // force channel count to mono
            let panner = StereoPannerNode::new(
                &context,
                StereoPannerOptions {
                    audio_node_options: AudioNodeOptions {
                        channel_count: 1,
                        channel_count_mode: ChannelCountMode::ClampedMax,
                        ..AudioNodeOptions::default()
                    },
                    pan: -1.,
                },
            );
            panner.connect(&context.destination());

            let mut src = context.create_buffer_source();
            src.connect(&panner);
            src.set_buffer(buffer.clone());
            src.start();

            let res = context.start_rendering_sync();

            assert_float_eq!(res.get_channel_data(0)[..], [1.; 128], abs_all <= 0.);
            assert_float_eq!(res.get_channel_data(1)[..], [0.; 128], abs_all <= 0.);
        }

        // right
        {
            let mut context = OfflineAudioContext::new(2, 128, 44_100.);
            // force channel count to mono
            let panner = StereoPannerNode::new(
                &context,
                StereoPannerOptions {
                    audio_node_options: AudioNodeOptions {
                        channel_count: 1,
                        channel_count_mode: ChannelCountMode::ClampedMax,
                        ..AudioNodeOptions::default()
                    },
                    pan: 1.,
                },
            );
            panner.connect(&context.destination());

            let mut src = context.create_buffer_source();
            src.connect(&panner);
            src.set_buffer(buffer.clone());
            src.start();

            let res = context.start_rendering_sync();

            assert_float_eq!(res.get_channel_data(0)[..], [0.; 128], abs_all <= 1e-7);
            assert_float_eq!(res.get_channel_data(1)[..], [1.; 128], abs_all <= 0.);
        }

        // equal power
        {
            let mut context = OfflineAudioContext::new(2, 128, 44_100.);
            // force channel count to mono
            let panner = StereoPannerNode::new(
                &context,
                StereoPannerOptions {
                    audio_node_options: AudioNodeOptions {
                        channel_count: 1,
                        channel_count_mode: ChannelCountMode::ClampedMax,
                        ..AudioNodeOptions::default()
                    },
                    pan: 0.,
                },
            );
            panner.connect(&context.destination());

            let mut src = context.create_buffer_source();
            src.connect(&panner);
            src.set_buffer(buffer.clone());
            src.start();

            let res = context.start_rendering_sync();

            let mut power = [0.; 128];
            power
                .iter_mut()
                .zip(res.get_channel_data(0).iter())
                .zip(res.get_channel_data(1).iter())
                .for_each(|((p, l), r)| {
                    *p = l * l + r * r;
                });

            assert_float_eq!(power, [1.; 128], abs_all <= 1.2e-7);
        }
    }

    #[test]
    fn test_stereo_panning() {
        let sample_rate = 44_100.;

        let context = OfflineAudioContext::new(2, 128, 44_100.);

        let mut buffer = context.create_buffer(2, 128, sample_rate);
        buffer.copy_to_channel(&[1.; 128], 0);
        buffer.copy_to_channel(&[1.; 128], 1);

        // left
        {
            let mut context = OfflineAudioContext::new(2, 128, 44_100.);
            // force channel count to mono
            let panner = StereoPannerNode::new(
                &context,
                StereoPannerOptions {
                    pan: -1.,
                    ..StereoPannerOptions::default()
                },
            );
            panner.connect(&context.destination());

            let mut src = context.create_buffer_source();
            src.connect(&panner);
            src.set_buffer(buffer.clone());
            src.start();

            let res = context.start_rendering_sync();

            assert_float_eq!(res.get_channel_data(0)[..], [2.; 128], abs_all <= 0.);
            assert_float_eq!(res.get_channel_data(1)[..], [0.; 128], abs_all <= 0.);
        }

        // right
        {
            let mut context = OfflineAudioContext::new(2, 128, 44_100.);
            // force channel count to mono
            let panner = StereoPannerNode::new(
                &context,
                StereoPannerOptions {
                    pan: 1.,
                    ..StereoPannerOptions::default()
                },
            );
            panner.connect(&context.destination());

            let mut src = context.create_buffer_source();
            src.connect(&panner);
            src.set_buffer(buffer.clone());
            src.start();

            let res = context.start_rendering_sync();

            assert_float_eq!(res.get_channel_data(0)[..], [0.; 128], abs_all <= 1e-7);
            assert_float_eq!(res.get_channel_data(1)[..], [2.; 128], abs_all <= 0.);
        }

        // middle
        {
            let mut context = OfflineAudioContext::new(2, 128, 44_100.);
            // force channel count to mono
            let panner = StereoPannerNode::new(
                &context,
                StereoPannerOptions {
                    pan: 0.,
                    ..StereoPannerOptions::default()
                },
            );
            panner.connect(&context.destination());

            let mut src = context.create_buffer_source();
            src.connect(&panner);
            src.set_buffer(buffer.clone());
            src.start();

            let res = context.start_rendering_sync();

            assert_float_eq!(res.get_channel_data(0)[..], [1.; 128], abs_all <= 1e-7);
            assert_float_eq!(res.get_channel_data(1)[..], [1.; 128], abs_all <= 0.);
        }
    }
}
