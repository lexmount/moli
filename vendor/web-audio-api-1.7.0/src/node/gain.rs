use crate::context::{AudioContextRegistration, AudioParamId, BaseAudioContext};
use crate::param::{AudioParam, AudioParamDescriptor};
use crate::render::{
    AudioParamValues, AudioProcessor, AudioRenderQuantum, AudioWorkletGlobalScope,
};

use super::{AudioNode, AudioNodeOptions, ChannelConfig};

/// Options for constructing a [`GainNode`]
// dictionary GainOptions : AudioNodeOptions {
//   float gain = 1.0;
// };
#[derive(Clone, Debug)]
pub struct GainOptions {
    pub gain: f32,
    pub audio_node_options: AudioNodeOptions,
}

impl Default for GainOptions {
    fn default() -> Self {
        Self {
            gain: 1.,
            audio_node_options: AudioNodeOptions::default(),
        }
    }
}

/// Applies a single gain (volume) value to its incoming audio signal.
///
/// The value is exposed as an [`AudioParam`] so it can be automated over time.
///
/// - MDN documentation: <https://developer.mozilla.org/en-US/docs/Web/API/GainNode>
/// - specification: <https://webaudio.github.io/web-audio-api/#GainNode>
/// - see also: [`BaseAudioContext::create_gain`]
///
/// # Usage
///
/// ```no_run
/// use web_audio_api::context::{BaseAudioContext, AudioContext};
/// use web_audio_api::node::{AudioNode, AudioScheduledSourceNode};
///
/// let context = AudioContext::default();
///
/// // Build an oscillator that we want to fade in.
/// let mut osc = context.create_oscillator();
/// osc.frequency().set_value(440.);
///
/// // The gain node sits between the source and the destination.
/// let gain = context.create_gain();
/// gain.gain().set_value(0.);
/// gain.gain()
///     .linear_ramp_to_value_at_time(1., context.current_time() + 1.);
///
/// osc.connect(&gain);
/// gain.connect(&context.destination());
/// osc.start();
/// ```
///
/// # Examples
///
/// - `cargo run --release --example amplitude_modulation`
///
#[derive(Debug)]
pub struct GainNode {
    /// Represents the node instance and its associated audio context
    registration: AudioContextRegistration,
    /// Infos about audio node channel configuration
    channel_config: ChannelConfig,
    /// Multiplier applied to every sample. Defaults to `1.0` (pass-through).
    gain: AudioParam,
}

impl AudioNode for GainNode {
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

impl GainNode {
    /// Constructs a new `GainNode` from explicit options.
    ///
    /// [`BaseAudioContext::create_gain`] is an alternative that applies the
    /// spec defaults (`gain = 1.0`).
    ///
    /// # Arguments
    ///
    /// * `context` - audio context in which the audio node will live
    /// * `options` - initial value of the gain parameter and channel config
    pub fn new<C: BaseAudioContext>(context: &C, options: GainOptions) -> Self {
        context.base().register(move |registration| {
            let param_opts = AudioParamDescriptor {
                name: String::new(),
                min_value: f32::MIN,
                max_value: f32::MAX,
                default_value: 1.,
                automation_rate: crate::param::AutomationRate::A,
            };
            let (param, proc) = context.create_audio_param(param_opts, &registration);

            param.set_value(options.gain);

            let render = GainRenderer { gain: proc };

            let node = GainNode {
                registration,
                channel_config: options.audio_node_options.into(),
                gain: param,
            };

            (node, Box::new(render))
        })
    }

    /// Returns the gain `AudioParam`.
    ///
    /// The default value is `1.0` (pass-through). Setting `0.0` mutes the
    /// signal; values greater than `1.0` boost it (and may clip downstream
    /// nodes if uncompensated). Because the parameter is `a-rate`, it can be
    /// scheduled with the full automation API such as
    /// [`AudioParam::linear_ramp_to_value_at_time`] for fades.
    #[must_use]
    pub fn gain(&self) -> &AudioParam {
        &self.gain
    }
}

struct GainRenderer {
    gain: AudioParamId,
}

impl AudioProcessor for GainRenderer {
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

        let gain = params.get(&self.gain);

        // very fast track for mute or pass-through
        if gain.len() == 1 {
            // 1e-6 is -120 dB when close to 0 and ±8.283506e-6 dB when close to 1
            // very probably small enough to not be audible
            let threshold = 1e-6;

            let diff_to_zero = gain[0].abs();
            if diff_to_zero <= threshold {
                output.make_silent();
                return false;
            }

            let diff_to_one = (1. - gain[0]).abs();
            if diff_to_one <= threshold {
                *output = input.clone();
                return false;
            }
        }

        *output = input.clone();

        if gain.len() == 1 {
            let g = gain[0];

            output.channels_mut().iter_mut().for_each(|channel| {
                channel.iter_mut().for_each(|o| *o *= g);
            });
        } else {
            output.channels_mut().iter_mut().for_each(|channel| {
                channel
                    .iter_mut()
                    .zip(gain.iter().cycle())
                    .for_each(|(o, g)| *o *= g);
            });
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::OfflineAudioContext;
    use float_eq::assert_float_eq;

    #[test]
    fn test_audioparam_value_applies_immediately() {
        let context = OfflineAudioContext::new(1, 128, 48000.);
        let options = GainOptions {
            gain: 0.12,
            ..Default::default()
        };
        let src = GainNode::new(&context, options);
        assert_float_eq!(src.gain.value(), 0.12, abs_all <= 0.);
    }
}
