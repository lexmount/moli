use std::fmt::Debug;

use crate::context::{AudioContextRegistration, BaseAudioContext};
use crate::render::{
    AudioParamValues, AudioProcessor, AudioRenderQuantum, AudioWorkletGlobalScope,
};
use crate::MAX_CHANNELS;

use super::{AudioNode, AudioNodeOptions, ChannelConfig, ChannelCountMode, ChannelInterpretation};

const DEFAULT_NUMBER_OF_OUTPUTS: usize = 6;

/// Assert that the given number of channels is valid for a ChannelMergerNode
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
        "IndexSizeError - Invalid number of channels: {:?} is outside range [1, {:?}]",
        number_of_channels,
        MAX_CHANNELS
    );
}

/// Assert that the channel count is valid for the ChannelSplitterNode
/// see <https://webaudio.github.io/web-audio-api/#audionode-channelcount-constraints>
///
/// # Panics
///
/// This function panics if given count is not equal to number of outputs
///
#[track_caller]
#[inline(always)]
fn assert_valid_channel_count(count: usize, number_of_outputs: usize) {
    assert!(
        count == number_of_outputs,
        "InvalidStateError - channel count of ChannelSplitterNode must be equal to number of outputs"
    );
}

/// Assert that the channel count mode is valid for the ChannelSplitterNode
/// see <https://webaudio.github.io/web-audio-api/#audionode-channelcountmode-constraints>
///
/// # Panics
///
/// This function panics if the mode is not equal to Explicit
///
#[track_caller]
#[inline(always)]
fn assert_valid_channel_count_mode(mode: ChannelCountMode) {
    assert!(
        mode == ChannelCountMode::Explicit,
        "InvalidStateError - channel count of ChannelSplitterNode must be set to Explicit"
    );
}

/// Assert that the channel interpretation is valid for the ChannelSplitterNode
/// see <https://webaudio.github.io/web-audio-api/#audionode-channelinterpretation-constraints>
///
/// # Panics
///
/// This function panics if the mode is not equal to Explicit
///
#[track_caller]
#[inline(always)]
fn assert_valid_channel_interpretation(interpretation: ChannelInterpretation) {
    assert!(
        interpretation == ChannelInterpretation::Discrete,
        "InvalidStateError - channel interpretation of ChannelSplitterNode must be set to Discrete"
    );
}

/// Options for constructing a [`ChannelSplitterNode`]
// dictionary ChannelSplitterOptions : AudioNodeOptions {
//   unsigned long numberOfOutputs = 6;
// };
#[derive(Clone, Debug)]
pub struct ChannelSplitterOptions {
    pub number_of_outputs: usize,
    pub audio_node_options: AudioNodeOptions,
}

impl Default for ChannelSplitterOptions {
    fn default() -> Self {
        Self {
            number_of_outputs: DEFAULT_NUMBER_OF_OUTPUTS,
            audio_node_options: AudioNodeOptions {
                channel_count: DEFAULT_NUMBER_OF_OUTPUTS, // must be same as number_of_outputs
                channel_count_mode: ChannelCountMode::Explicit,
                channel_interpretation: ChannelInterpretation::Discrete,
            },
        }
    }
}

/// AudioNode for accessing the individual channels of an audio stream in the routing graph
#[derive(Debug)]
pub struct ChannelSplitterNode {
    registration: AudioContextRegistration,
    channel_config: ChannelConfig,
    number_of_outputs: usize,
}

impl AudioNode for ChannelSplitterNode {
    fn registration(&self) -> &AudioContextRegistration {
        &self.registration
    }

    fn channel_config(&self) -> &ChannelConfig {
        &self.channel_config
    }

    fn set_channel_count(&self, count: usize) {
        assert_valid_channel_count(count, self.number_of_outputs);
    }

    fn set_channel_count_mode(&self, mode: ChannelCountMode) {
        assert_valid_channel_count_mode(mode);
        self.channel_config
            .set_count_mode(mode, self.registration());
    }

    fn set_channel_interpretation(&self, interpretation: ChannelInterpretation) {
        assert_valid_channel_interpretation(interpretation);
        self.channel_config
            .set_interpretation(interpretation, self.registration());
    }

    fn number_of_inputs(&self) -> usize {
        1
    }

    fn number_of_outputs(&self) -> usize {
        self.number_of_outputs
    }
}

impl ChannelSplitterNode {
    pub fn new<C: BaseAudioContext>(context: &C, mut options: ChannelSplitterOptions) -> Self {
        context.base().register(move |registration| {
            assert_valid_number_of_channels(options.number_of_outputs);

            // if channel count has been explicitly set, we need to check
            // its value against number of outputs
            if options.audio_node_options.channel_count != DEFAULT_NUMBER_OF_OUTPUTS {
                assert_valid_channel_count(
                    options.audio_node_options.channel_count,
                    options.number_of_outputs,
                );
            }
            options.audio_node_options.channel_count = options.number_of_outputs;

            assert_valid_channel_count_mode(options.audio_node_options.channel_count_mode);
            assert_valid_channel_interpretation(options.audio_node_options.channel_interpretation);

            let node = ChannelSplitterNode {
                registration,
                channel_config: options.audio_node_options.into(),
                number_of_outputs: options.number_of_outputs,
            };

            let render = ChannelSplitterRenderer {
                number_of_outputs: options.number_of_outputs,
            };

            (node, Box::new(render))
        })
    }
}

#[derive(Debug)]
struct ChannelSplitterRenderer {
    pub number_of_outputs: usize,
}

impl AudioProcessor for ChannelSplitterRenderer {
    fn process(
        &mut self,
        inputs: &[AudioRenderQuantum],
        outputs: &mut [AudioRenderQuantum],
        _params: AudioParamValues<'_>,
        _scope: &AudioWorkletGlobalScope,
    ) -> bool {
        // single input node
        let input = &inputs[0];

        // assert number of outputs was correctly set by renderer
        assert_eq!(self.number_of_outputs, outputs.len());

        for (i, output) in outputs.iter_mut().enumerate() {
            output.set_number_of_channels(1);

            if i < input.number_of_channels() {
                *output.channel_data_mut(0) = input.channel_data(i).clone();
            } else {
                // input does not have this channel filled, emit silence
                output.make_silent();
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use float_eq::assert_float_eq;

    use crate::context::{BaseAudioContext, OfflineAudioContext};
    use crate::node::{AudioNode, AudioScheduledSourceNode};
    use crate::AudioBuffer;

    use super::*;

    #[test]
    fn test_valid_constructor_options() {
        let sample_rate = 48000.;
        let context = OfflineAudioContext::new(1, 128, sample_rate);

        let options = ChannelSplitterOptions {
            number_of_outputs: 2,
            ..Default::default()
        };

        let splitter = ChannelSplitterNode::new(&context, options);
        assert_eq!(splitter.number_of_outputs(), 2);
        assert_eq!(splitter.channel_count(), 2);
    }

    #[test]
    #[should_panic]
    fn test_invalid_constructor_options() {
        let sample_rate = 48000.;
        let context = OfflineAudioContext::new(1, 128, sample_rate);

        let mut options = ChannelSplitterOptions::default();
        options.audio_node_options.channel_count = 7;

        let _splitter = ChannelSplitterNode::new(&context, options);
    }

    #[test]
    #[should_panic]
    fn test_set_channel_count() {
        let sample_rate = 48000.;
        let context = OfflineAudioContext::new(1, 128, sample_rate);

        let options = ChannelSplitterOptions::default();
        let splitter = ChannelSplitterNode::new(&context, options);
        splitter.set_channel_count(3);
    }

    #[test]
    fn test_splitter() {
        let sample_rate = 48000.;
        let mut context = OfflineAudioContext::new(1, 128, sample_rate);

        let splitter = context.create_channel_splitter(2);

        // connect the 2nd output to the destination
        splitter.connect_from_output_to_input(&context.destination(), 1, 0);

        // create buffer with sample value 1. left, value -1. right
        let audio_buffer = AudioBuffer::from(vec![vec![1.], vec![-1.]], 48000.);
        let mut src = context.create_buffer_source();
        src.set_buffer(audio_buffer);
        src.set_loop(true);
        src.start();
        src.connect(&splitter);

        let buffer = context.start_rendering_sync();

        let mono = buffer.get_channel_data(0);
        assert_float_eq!(mono, &[-1.; 128][..], abs_all <= 0.);
    }
}
