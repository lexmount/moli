use super::{AudioNode, AudioNodeOptions, ChannelConfig, ChannelCountMode, ChannelInterpretation};
use crate::context::{AudioContextRegistration, BaseAudioContext};
use crate::events::{AudioProcessingEvent, EventHandler, EventPayload, EventType};
use crate::render::{
    AudioParamValues, AudioProcessor, AudioRenderQuantum, AudioWorkletGlobalScope,
};
use crate::{AudioBuffer, RENDER_QUANTUM_SIZE};

use std::any::Any;

/// Options for constructing an [`ScriptProcessorNode`]
#[derive(Clone, Debug)]
pub struct ScriptProcessorOptions {
    pub buffer_size: usize,
    pub number_of_input_channels: usize,
    pub number_of_output_channels: usize,
}

/// An AudioNode which can generate, process, or analyse audio directly using a script (deprecated)
#[derive(Debug)]
pub struct ScriptProcessorNode {
    registration: AudioContextRegistration,
    channel_config: ChannelConfig,
    buffer_size: usize,
}

impl AudioNode for ScriptProcessorNode {
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
        assert_eq!(
            mode,
            ChannelCountMode::Explicit,
            "NotSupportedError - ScriptProcessorNode channel count mode must be 'explicit'",
        );
        self.channel_config
            .set_count_mode(mode, self.registration());
    }

    fn set_channel_count(&self, count: usize) {
        assert_eq!(
            count,
            self.channel_config.count(),
            "NotSupportedError - ScriptProcessorNode channel count must equal numberOfInputChannels"
        );
        self.channel_config.set_count(count, self.registration());
    }
}

impl ScriptProcessorNode {
    /// Creates a `ScriptProcessorNode`
    ///
    /// # Arguments
    ///
    /// - `context` - Audio context in which the node will live
    /// - `options` - node options
    ///
    /// # Panics
    ///
    /// This function panics if:
    /// - `buffer_size` is not 0, 256, 512, 1024, 2048, 4096, 8192, or 16384
    /// - the number of input and output channels are both zero
    /// - either of the channel counts exceed [`crate::MAX_CHANNELS`]
    pub fn new<C: BaseAudioContext>(context: &C, options: ScriptProcessorOptions) -> Self {
        let ScriptProcessorOptions {
            buffer_size,
            number_of_input_channels,
            number_of_output_channels,
        } = options;
        let buffer_size = if buffer_size == 0 { 256 } else { buffer_size };

        assert!(
            (buffer_size / 256).is_power_of_two() && buffer_size <= 16384,
            "IndexSizeError - bufferSize must be 0 or one of: 256, 512, 1024, 2048, 4096, 8192, 16384",
        );

        match (number_of_input_channels, number_of_output_channels) {
            (0, 0) => panic!("IndexSizeError - numberOfInputChannels and numberOfOutputChannels cannot both be zero"),
            (0, c) | (c, 0) => crate::assert_valid_number_of_channels(c),
            (c, d) => {
                crate::assert_valid_number_of_channels(c);
                crate::assert_valid_number_of_channels(d);
            }
        };

        context.base().register(move |registration| {
            let number_of_quanta = buffer_size / RENDER_QUANTUM_SIZE;
            let render = ScriptProcessorRenderer {
                input_buffer: Vec::with_capacity(number_of_quanta),
                output_buffer: Vec::with_capacity(number_of_quanta),
                next_output_buffer: Vec::with_capacity(number_of_quanta),
                buffer_size,
                number_of_output_channels,
            };

            let upmix_input_channels = if number_of_input_channels == 0 {
                1 // any value will do, because upmixing is not performed
            } else {
                number_of_input_channels
            };
            let audio_node_options = AudioNodeOptions {
                channel_count: upmix_input_channels,
                channel_count_mode: ChannelCountMode::Explicit,
                channel_interpretation: ChannelInterpretation::Speakers,
            };

            let node = ScriptProcessorNode {
                registration,
                channel_config: audio_node_options.into(),
                buffer_size,
            };

            (node, Box::new(render))
        })
    }

    pub fn buffer_size(&self) -> usize {
        self.buffer_size
    }

    /// Register callback to run when the AudioProcessingEvent is dispatched
    ///
    /// The event handler processes audio from the input (if any) by accessing the audio data from
    /// the inputBuffer attribute. The audio data which is the result of the processing (or the
    /// synthesized data if there are no inputs) is then placed into the outputBuffer.
    ///
    /// The output buffer is shipped back to the render thread when the AudioProcessingEvent goes
    /// out of scope, so be sure not to store it somewhere.
    ///
    /// Only a single event handler is active at any time. Calling this method multiple times will
    /// override the previous event handler.
    pub fn set_onaudioprocess<F: FnMut(AudioProcessingEvent) + Send + 'static>(
        &self,
        mut callback: F,
    ) {
        // We need these fields to ship the output buffer to the render thread
        let base = self.registration().context().clone();
        let id = self.registration().id();

        let callback = move |v| {
            let mut payload = match v {
                EventPayload::AudioProcessing(v) => v,
                _ => unreachable!(),
            };
            payload.registration = Some((base.clone(), id));
            callback(payload);
        };

        self.context().set_event_handler(
            EventType::AudioProcessing(self.registration().id()),
            EventHandler::Multiple(Box::new(callback)),
        );
    }

    /// Unset the callback to run when the AudioProcessingEvent is dispatched
    pub fn clear_onaudioprocess(&self) {
        self.context()
            .clear_event_handler(EventType::AudioProcessing(self.registration().id()));
    }
}

struct ScriptProcessorRenderer {
    input_buffer: Vec<AudioRenderQuantum>,
    output_buffer: Vec<AudioRenderQuantum>,
    next_output_buffer: Vec<AudioRenderQuantum>,
    buffer_size: usize,
    number_of_output_channels: usize,
}

// SAFETY:
// AudioRenderQuantums are not Send but we promise the `buffer` VecDeque is empty before we ship it
// to the render thread.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for ScriptProcessorRenderer {}

impl AudioProcessor for ScriptProcessorRenderer {
    fn process(
        &mut self,
        inputs: &[AudioRenderQuantum],
        outputs: &mut [AudioRenderQuantum],
        _params: AudioParamValues<'_>,
        scope: &AudioWorkletGlobalScope,
    ) -> bool {
        // single input/output node
        let input = &inputs[0];
        let output = &mut outputs[0];

        // default to silent output
        output.make_silent();
        let silence = output.clone();

        // when there are output buffers lined up, emit the first one
        if !self.output_buffer.is_empty() {
            *output = self.output_buffer.remove(0);
        }

        // buffer inputs
        let number_of_quanta = self.input_buffer.capacity();
        self.input_buffer.push(input.clone());

        // check if we need to emit an event (input buffer is full)
        if self.input_buffer.len() == number_of_quanta {
            // convert self.input_buffer to an AudioBuffer
            let number_of_input_channels = self
                .input_buffer
                .iter()
                .map(|i| i.number_of_channels())
                .max()
                .unwrap();
            let mut input_samples = vec![vec![0.; self.buffer_size]; number_of_input_channels];
            self.input_buffer.iter().enumerate().for_each(|(i, b)| {
                let offset = RENDER_QUANTUM_SIZE * i;
                b.channels()
                    .iter()
                    .zip(input_samples.iter_mut())
                    .for_each(|(c, o)| {
                        o[offset..(offset + RENDER_QUANTUM_SIZE)].copy_from_slice(c);
                    });
            });
            let input_buffer = AudioBuffer::from(input_samples, scope.sample_rate);

            // create a suitable output AudioBuffer
            let output_samples = vec![vec![0.; self.buffer_size]; self.number_of_output_channels];
            let output_buffer = AudioBuffer::from(output_samples, scope.sample_rate);

            // emit event to control thread
            let playback_time =
                scope.current_time + self.buffer_size as f64 / scope.sample_rate as f64;
            scope.send_audio_processing_event(input_buffer, output_buffer, playback_time);

            // clear existing input buffer
            self.input_buffer.clear();

            // move next output buffer into current output buffer
            std::mem::swap(&mut self.output_buffer, &mut self.next_output_buffer);

            // fill next output buffer with silence (with the right channel count)
            let mut silent_quantum = silence;
            silent_quantum.set_number_of_channels(self.number_of_output_channels);
            self.next_output_buffer.clear();
            self.next_output_buffer
                .resize(number_of_quanta, silent_quantum);
        }

        false // node is kept alive as long as the handle in the event loop still exists
    }

    fn onmessage(&mut self, msg: &mut dyn Any) {
        if let Some(buffer) = msg.downcast_mut::<AudioBuffer>() {
            buffer.channels().iter().enumerate().for_each(|(i, c)| {
                c.as_slice()
                    .chunks(RENDER_QUANTUM_SIZE)
                    .zip(self.next_output_buffer.iter_mut())
                    .for_each(|(s, o)| o.channel_data_mut(i).copy_from_slice(s))
            });
            return;
        };

        log::warn!("ScriptProcessorRenderer: Dropping incoming message {msg:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::OfflineAudioContext;
    use crate::node::scheduled_source::AudioScheduledSourceNode;
    use float_eq::assert_float_eq;

    #[test]
    fn test_constructor() {
        let mut context = OfflineAudioContext::new(2, 1024, 48000.);
        let node = context.create_script_processor(512, 1, 1);
        node.set_channel_count(1);
        node.set_channel_count_mode(ChannelCountMode::Explicit);
        node.connect(&context.destination());
        let _ = context.start_rendering_sync();
        // TODO - does not work with OfflineAudioContext due to lack of event loop
    }

    #[test]
    fn test_constructor_zero_inputs() {
        let context = OfflineAudioContext::new(2, 1024, 48000.);
        let _ = context.create_script_processor(512, 0, 1); // should not panic
    }

    #[test]
    fn test_constructor_zero_outputs() {
        let context = OfflineAudioContext::new(2, 1024, 48000.);
        let _ = context.create_script_processor(512, 1, 0); // should not panic
    }

    #[test]
    fn test_rendering() {
        const BUFFER_SIZE: usize = 256;

        let mut context = OfflineAudioContext::new(1, BUFFER_SIZE * 3, 48000.);

        let node = context.create_script_processor(BUFFER_SIZE, 0, 1);
        node.connect(&context.destination());
        node.set_onaudioprocess(|mut e| {
            e.output_buffer.get_channel_data_mut(0).fill(1.); // set all samples to 1.
        });

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        // first `2 * BUFFER_SIZE` samples should be silent due to buffering
        assert_float_eq!(
            channel[..2 * BUFFER_SIZE],
            &[0.; 2 * BUFFER_SIZE][..],
            abs_all <= 0.
        );

        // rest of the samples should be 1.
        assert_float_eq!(
            channel[2 * BUFFER_SIZE..],
            &[1.; BUFFER_SIZE][..],
            abs_all <= 0.
        );
    }

    #[test]
    fn test_multiple_channels() {
        const BUFFER_SIZE: usize = 256;

        let mut context = OfflineAudioContext::new(2, BUFFER_SIZE * 3, 48000.);

        // 2 input channels, 2 output channels
        let node = context.create_script_processor(BUFFER_SIZE, 2, 2);
        node.connect(&context.destination());
        node.set_onaudioprocess(|mut e| {
            // left output buffer is left input * 2
            e.output_buffer
                .get_channel_data_mut(0)
                .iter_mut()
                .zip(e.input_buffer.get_channel_data(0))
                .for_each(|(o, i)| *o = *i * 2.);

            // right output buffer is right input * 3
            e.output_buffer
                .get_channel_data_mut(1)
                .iter_mut()
                .zip(e.input_buffer.get_channel_data(1))
                .for_each(|(o, i)| *o = *i * 3.);
        });

        // let the input be a mono constant source, it will be upmixed to two channels
        let mut src = context.create_constant_source();
        src.start();
        src.connect(&node);

        let result = context.start_rendering_sync();
        let channel1 = result.get_channel_data(0);
        let channel2 = result.get_channel_data(1);

        // first `2 * BUFFER_SIZE` samples should be silent due to buffering
        assert_float_eq!(
            channel1[..2 * BUFFER_SIZE],
            &[0.; 2 * BUFFER_SIZE][..],
            abs_all <= 0.
        );
        assert_float_eq!(
            channel2[..2 * BUFFER_SIZE],
            &[0.; 2 * BUFFER_SIZE][..],
            abs_all <= 0.
        );

        // rest of the samples should be 2. for left buffer
        assert_float_eq!(
            channel1[2 * BUFFER_SIZE..],
            &[2.; BUFFER_SIZE][..],
            abs_all <= 0.
        );
        // rest of the samples should be 3. for right buffer
        assert_float_eq!(
            channel2[2 * BUFFER_SIZE..],
            &[3.; BUFFER_SIZE][..],
            abs_all <= 0.
        );
    }
}
