use crate::context::{AudioContextRegistration, AudioParamId, BaseAudioContext};
use crate::param::{AudioParam, AudioParamDescriptor};
use crate::render::{
    AudioParamValues, AudioProcessor, AudioRenderQuantum, AudioWorkletGlobalScope,
};
use crate::RENDER_QUANTUM_SIZE;

use super::{AudioNode, AudioNodeOptions, ChannelConfig, ChannelInterpretation};

use std::cell::{Cell, RefCell, RefMut};
use std::rc::Rc;

/// Options for constructing a [`DelayNode`]
// dictionary DelayOptions : AudioNodeOptions {
//   double maxDelayTime = 1;
//   double delayTime = 0;
// };
#[derive(Clone, Debug)]
pub struct DelayOptions {
    pub max_delay_time: f64,
    pub delay_time: f64,
    pub audio_node_options: AudioNodeOptions,
}

impl Default for DelayOptions {
    fn default() -> Self {
        Self {
            max_delay_time: 1.,
            delay_time: 0.,
            audio_node_options: AudioNodeOptions::default(),
        }
    }
}

#[derive(Copy, Clone, Debug, Default)]
struct PlaybackInfo {
    prev_block_index: usize,
    prev_frame_index: usize,
    k: f32,
}

/// Node that delays the incoming audio signal by a certain amount
///
/// The current implementation does not allow for zero delay. The minimum delay is one render
/// quantum (e.g. ~2.9ms at 44.1kHz).
///
/// - MDN documentation: <https://developer.mozilla.org/en-US/docs/Web/API/DelayNode>
/// - specification: <https://webaudio.github.io/web-audio-api/#DelayNode>
/// - see also: [`BaseAudioContext::create_delay`]
///
/// # Usage
///
/// ```no_run
/// use std::fs::File;
/// use web_audio_api::context::{BaseAudioContext, AudioContext};
/// use web_audio_api::node::{AudioNode, AudioScheduledSourceNode};
///
/// // create an `AudioContext` and load a sound file
/// let context = AudioContext::default();
/// let file = File::open("samples/sample.wav").unwrap();
/// let audio_buffer = context.decode_audio_data_sync(file).unwrap();
///
/// // create a delay of 0.5s
/// let delay = context.create_delay(1.);
/// delay.delay_time().set_value(0.5);
/// delay.connect(&context.destination());
///
/// let mut src = context.create_buffer_source();
/// src.set_buffer(audio_buffer);
/// // connect to both delay and destination
/// src.connect(&delay);
/// src.connect(&context.destination());
/// src.start();
/// ```
///
/// # Examples
///
/// - `cargo run --release --example simple_delay`
/// - `cargo run --release --example feedback_delay`
///
/*
 * For simplicity in the audio graph rendering, we have made the conscious decision to deviate from
 * the spec and split the delay node up front in a reader and writer node (instead of during the
 * render loop - see https://webaudio.github.io/web-audio-api/#rendering-loop )
 *
 * This has a drawback: a delay of 0 is no longer possible. This would only be possible if the
 * writer end is rendered before the reader end in the graph, but we cannot enforce that here.
 * (The only way would be to connect the writer to the reader, but that would kill the
 * cycle-breaker feature of the delay node.)
 *
 * @note: one possible strategy here would be to create a connection between Reader
 * and Writer in `DelayNode::new` just to guarantee the order of the processing if
 * the delay is not in a loop. In the graph process if the node is found in a cycle,
 * this connection could be removed and the Reader marked as "in_cycle" so that
 * it would clamp the min delay to quantum duration.
 * > no need to make this cancellable, once in a cycle the node behaves like that
 * even if the cycle is broken later (user have to know what they are doing)
 */
#[derive(Debug)]
pub struct DelayNode {
    reader_registration: AudioContextRegistration,
    writer_registration: AudioContextRegistration,
    delay_time: AudioParam,
    channel_config: ChannelConfig,
}

impl AudioNode for DelayNode {
    /*
     * We set the writer node as 'main' registration.  This means other nodes can say
     * `node.connect(delaynode)` and they will connect to the writer.
     * Below, we override the (dis)connect methods as they should operate on the reader node.
     */
    fn registration(&self) -> &AudioContextRegistration {
        &self.writer_registration
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

    /// Connect a specific output of this AudioNode to a specific input of another node.
    fn connect_from_output_to_input<'a>(
        &self,
        dest: &'a dyn AudioNode,
        output: usize,
        input: usize,
    ) -> &'a dyn AudioNode {
        assert!(
            self.context() == dest.context(),
            "InvalidAccessError - Attempting to connect nodes from different contexts",
        );

        assert!(
            self.number_of_outputs() > output,
            "IndexSizeError - output port {} is out of bounds",
            output
        );

        assert!(
            dest.number_of_inputs() > input,
            "IndexSizeError - input port {} is out of bounds",
            input
        );

        self.context().connect(
            self.reader_registration.id(),
            dest.registration().id(),
            output,
            input,
        );

        dest
    }

    /// Disconnects all outgoing connections from the AudioNode.
    fn disconnect(&self) {
        self.context()
            .disconnect(self.reader_registration.id(), None, None, None);
    }

    /// Disconnects all outputs of the AudioNode that go to a specific destination AudioNode.
    ///
    /// # Panics
    ///
    /// This function will panic when
    /// - the AudioContext of the source and destination does not match
    /// - the source node was not connected to the destination node
    fn disconnect_dest(&self, dest: &dyn AudioNode) {
        assert!(
            self.context() == dest.context(),
            "InvalidAccessError - Attempting to disconnect nodes from different contexts"
        );

        self.context().disconnect(
            self.reader_registration.id(),
            None,
            Some(dest.registration().id()),
            None,
        );
    }

    /// Disconnects all outgoing connections at the given output port from the AudioNode.
    ///
    /// # Panics
    ///
    /// This function will panic when
    /// - if the output port is out of bounds for this node
    fn disconnect_output(&self, output: usize) {
        assert!(
            self.number_of_outputs() > output,
            "IndexSizeError - output port {} is out of bounds",
            output
        );

        self.context()
            .disconnect(self.reader_registration.id(), Some(output), None, None);
    }

    /// Disconnects a specific output of the AudioNode to a specific destination AudioNode
    ///
    /// # Panics
    ///
    /// This function will panic when
    /// - the AudioContext of the source and destination does not match
    /// - if the output port is out of bounds for the source node
    /// - the source node was not connected to the destination node
    fn disconnect_dest_from_output(&self, dest: &dyn AudioNode, output: usize) {
        assert!(
            self.context() == dest.context(),
            "InvalidAccessError - Attempting to disconnect nodes from different contexts"
        );

        assert!(
            self.number_of_outputs() > output,
            "IndexSizeError - output port {} is out of bounds",
            output
        );

        self.context().disconnect(
            self.reader_registration.id(),
            Some(output),
            Some(dest.registration().id()),
            None,
        );
    }

    /// Disconnects a specific output of the AudioNode to a specific input of some destination
    /// AudioNode
    ///
    /// # Panics
    ///
    /// This function will panic when
    /// - the AudioContext of the source and destination does not match
    /// - if the input port is out of bounds for the destination node
    /// - if the output port is out of bounds for the source node
    /// - the source node was not connected to the destination node
    fn disconnect_dest_from_output_to_input(
        &self,
        dest: &dyn AudioNode,
        output: usize,
        input: usize,
    ) {
        assert!(
            self.context() == dest.context(),
            "InvalidAccessError - Attempting to disconnect nodes from different contexts"
        );

        assert!(
            self.number_of_outputs() > output,
            "IndexSizeError - output port {} is out of bounds",
            output
        );

        assert!(
            dest.number_of_inputs() > input,
            "IndexSizeError - input port {} is out of bounds",
            input
        );

        self.context().disconnect(
            self.reader_registration.id(),
            Some(output),
            Some(dest.registration().id()),
            Some(input),
        );
    }
}

impl DelayNode {
    /// Create a new DelayNode
    ///
    /// # Panics
    ///
    /// Panics when the max delay value is smaller than zero or langer than three minutes.
    pub fn new<C: BaseAudioContext>(context: &C, options: DelayOptions) -> Self {
        let sample_rate = context.sample_rate() as f64;

        // Specifies the maximum delay time in seconds allowed for the delay line.
        // If specified, this value MUST be greater than zero and less than three
        // minutes or a NotSupportedError exception MUST be thrown. If not specified,
        // then 1 will be used.
        assert!(
            options.max_delay_time > 0. && options.max_delay_time < 180.,
            "NotSupportedError - maxDelayTime MUST be greater than zero and less than three minutes",
        );

        // Allocate large enough ring buffer to store all delayed samples.
        // We add one extra slot in the ring buffer so that reader never reads the
        // same entry in history as the writer, even if `delay_time == max_delay_time`
        // of if `max_delay_time < quantum duration`
        let max_delay_time = options.max_delay_time;
        let num_quanta =
            (max_delay_time * sample_rate / RENDER_QUANTUM_SIZE as f64).ceil() as usize;
        let ring_buffer = Vec::with_capacity(num_quanta + 1);

        let shared_ring_buffer = Rc::new(RefCell::new(ring_buffer));
        let shared_ring_buffer_clone = Rc::clone(&shared_ring_buffer);

        // shared value set by the writer when it is dropped
        let last_written_index = Rc::new(Cell::<Option<usize>>::new(None));
        let last_written_index_clone = Rc::clone(&last_written_index);

        // shared value for reader/writer to determine who was rendered first,
        // this will indicate if the delay node acts as a cycle breaker
        let latest_frame_written = Rc::new(Cell::new(u64::MAX));
        let latest_frame_written_clone = Rc::clone(&latest_frame_written);

        let node = context.base().register(move |writer_registration| {
            let node = context.base().register(move |reader_registration| {
                let param_opts = AudioParamDescriptor {
                    name: String::new(),
                    min_value: 0.,
                    max_value: max_delay_time as f32,
                    default_value: 0.,
                    automation_rate: crate::param::AutomationRate::A,
                };
                let (param, proc) = context.create_audio_param(param_opts, &reader_registration);

                param.set_value(options.delay_time as f32);

                let reader_render = DelayReader {
                    delay_time: proc,
                    ring_buffer: shared_ring_buffer_clone,
                    index: 0,
                    last_written_index: last_written_index_clone,
                    in_cycle: false,
                    last_written_index_checked: None,
                    latest_frame_written: latest_frame_written_clone,
                };

                let node = DelayNode {
                    reader_registration,
                    writer_registration,
                    channel_config: options.audio_node_options.into(),
                    delay_time: param,
                };

                (node, Box::new(reader_render))
            });

            let writer_render = DelayWriter {
                ring_buffer: shared_ring_buffer,
                index: 0,
                last_written_index,
                latest_frame_written,
            };

            (node, Box::new(writer_render))
        });

        let writer_id = node.writer_registration.id();
        let reader_id = node.reader_registration.id();
        // connect Writer to Reader to guarantee order of processing and enable
        // sub-quantum delay. If found in cycle this connection will be deleted
        // by the graph and the minimum delay clamped to one render quantum
        context.base().mark_cycle_breaker(&node.writer_registration);
        context.base().connect(writer_id, reader_id, 0, 0);

        node
    }

    /// A-rate [`AudioParam`] representing the amount of delay (in seconds) to apply.
    pub fn delay_time(&self) -> &AudioParam {
        &self.delay_time
    }
}

struct DelayWriter {
    ring_buffer: Rc<RefCell<Vec<AudioRenderQuantum>>>,
    index: usize,
    latest_frame_written: Rc<Cell<u64>>,
    last_written_index: Rc<Cell<Option<usize>>>,
}

// SAFETY:
// AudioRenderQuantums are not Send but we promise the `ring_buffer` Vec is
// empty before we ship it to the render thread.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for DelayWriter {}

trait RingBufferChecker {
    fn ring_buffer_mut(&self) -> RefMut<'_, Vec<AudioRenderQuantum>>;

    // This step guarantees the ring buffer is filled with silence buffers,
    // This allow to simplify the code in both Writer and Reader as we know
    // `len() == capacity()` and all inner buffers are initialized with zeros.
    #[inline(always)]
    fn check_ring_buffer_size(&self, render_quantum: &AudioRenderQuantum) {
        let mut ring_buffer = self.ring_buffer_mut();

        if ring_buffer.len() < ring_buffer.capacity() {
            let len = ring_buffer.capacity();
            let mut silence = render_quantum.clone();
            silence.make_silent();

            ring_buffer.resize(len, silence);
        }
    }
}

impl Drop for DelayWriter {
    fn drop(&mut self) {
        let last_written_index = if self.index == 0 {
            self.ring_buffer.borrow().capacity() - 1
        } else {
            self.index - 1
        };

        self.last_written_index.set(Some(last_written_index));
    }
}

impl RingBufferChecker for DelayWriter {
    #[inline(always)]
    fn ring_buffer_mut(&self) -> RefMut<'_, Vec<AudioRenderQuantum>> {
        self.ring_buffer.borrow_mut()
    }
}

impl AudioProcessor for DelayWriter {
    fn process(
        &mut self,
        inputs: &[AudioRenderQuantum],
        outputs: &mut [AudioRenderQuantum],
        _params: AudioParamValues<'_>,
        scope: &AudioWorkletGlobalScope,
    ) -> bool {
        // single input/output node
        let input = inputs[0].clone();
        let output = &mut outputs[0];

        // We must perform this check on both Writer and Reader as the order of
        // the rendering between them is not guaranteed.
        self.check_ring_buffer_size(&input);
        // `check_ring_buffer_up_down_mix` can only be done on the Writer
        // side as Reader do not access the "real" input
        self.check_ring_buffer_up_down_mix(&input);

        // populate ring buffer
        let mut buffer = self.ring_buffer.borrow_mut();
        buffer[self.index] = input;

        // increment cursor and last written frame
        self.index = (self.index + 1) % buffer.capacity();
        self.latest_frame_written.set(scope.current_frame);

        // The writer end does not produce output,
        // clear the buffer so that it can be reused
        output.make_silent();

        // let the node be decommisioned if it has no input left
        false
    }

    fn has_side_effects(&self) -> bool {
        true // message passing
    }
}

impl DelayWriter {
    #[inline(always)]
    fn check_ring_buffer_up_down_mix(&self, input: &AudioRenderQuantum) {
        // [spec]
        // When the number of channels in a DelayNode's input changes (thus changing
        // the output channel count also), there may be delayed audio samples which
        // have not yet been output by the node and are part of its internal state.
        // If these samples were received earlier with a different channel count,
        // they MUST be upmixed or downmixed before being combined with newly received
        // input so that all internal delay-line mixing takes place using the single
        // prevailing channel layout.
        let mut ring_buffer = self.ring_buffer_mut();
        let buffer_number_of_channels = ring_buffer[0].number_of_channels();
        let input_number_of_channels = input.number_of_channels();

        if buffer_number_of_channels != input_number_of_channels {
            for render_quantum in ring_buffer.iter_mut() {
                render_quantum.mix(input_number_of_channels, ChannelInterpretation::Speakers);
            }
        }
    }
}

struct DelayReader {
    delay_time: AudioParamId,
    ring_buffer: Rc<RefCell<Vec<AudioRenderQuantum>>>,
    index: usize,
    latest_frame_written: Rc<Cell<u64>>,
    in_cycle: bool,
    last_written_index: Rc<Cell<Option<usize>>>,
    // local copy of shared `last_written_index` so as to avoid render ordering issues
    last_written_index_checked: Option<usize>,
}

// SAFETY:
// AudioRenderQuantums are not Send but we promise the `ring_buffer` Vec is
// empty before we ship it to the render thread.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for DelayReader {}

impl RingBufferChecker for DelayReader {
    #[inline(always)]
    fn ring_buffer_mut(&self) -> RefMut<'_, Vec<AudioRenderQuantum>> {
        self.ring_buffer.borrow_mut()
    }
}

impl AudioProcessor for DelayReader {
    fn process(
        &mut self,
        _inputs: &[AudioRenderQuantum], // cannot be used
        outputs: &mut [AudioRenderQuantum],
        params: AudioParamValues<'_>,
        scope: &AudioWorkletGlobalScope,
    ) -> bool {
        // single input/output node
        let output = &mut outputs[0];
        // We must perform the checks (buffer size and up/down mix) on both Writer
        // and Reader as the order of processing between them is not guaranteed.
        self.check_ring_buffer_size(output);

        let ring_buffer = self.ring_buffer.borrow();

        // we need to rely on ring buffer to know the actual number of output channels
        let number_of_channels = ring_buffer[0].number_of_channels();
        output.set_number_of_channels(number_of_channels);

        if !self.in_cycle {
            // check the latest written frame by the delay writer
            let latest_frame_written = self.latest_frame_written.get();
            // if the delay writer has not rendered before us, the cycle breaker has been applied
            self.in_cycle = latest_frame_written != scope.current_frame;
            // once we store in_cycle = true, we do not want to go back to false
            // https://github.com/orottier/web-audio-api-rs/pull/198#discussion_r945326200
        }

        // compute all playback infos for this block
        let delay = params.get(&self.delay_time);
        let sample_rate = scope.sample_rate as f64;
        let dt = 1. / sample_rate;
        let quantum_duration = RENDER_QUANTUM_SIZE as f64 * dt;
        let ring_size = ring_buffer.len() as i32;
        let ring_index = self.index as i32;
        let mut playback_infos = [PlaybackInfo::default(); RENDER_QUANTUM_SIZE];

        if delay.len() == 1 {
            playback_infos[0] = Self::get_playback_infos(
                f64::from(delay[0]),
                self.in_cycle,
                0.,
                quantum_duration,
                sample_rate,
                ring_size,
                ring_index,
            );

            for i in 1..RENDER_QUANTUM_SIZE {
                let PlaybackInfo {
                    prev_block_index,
                    prev_frame_index,
                    k,
                } = playback_infos[i - 1];

                let mut prev_block_index = prev_block_index;
                let mut prev_frame_index = prev_frame_index + 1;

                if prev_frame_index >= RENDER_QUANTUM_SIZE {
                    prev_block_index = (prev_block_index + 1) % ring_buffer.len();
                    prev_frame_index = 0;
                }

                playback_infos[i] = PlaybackInfo {
                    prev_block_index,
                    prev_frame_index,
                    k,
                };
            }
        } else {
            delay
                .iter()
                .zip(playback_infos.iter_mut())
                .enumerate()
                .for_each(|(index, (&d, infos))| {
                    *infos = Self::get_playback_infos(
                        f64::from(d),
                        self.in_cycle,
                        index as f64,
                        quantum_duration,
                        sample_rate,
                        ring_size,
                        ring_index,
                    );
                });
        }

        // [spec] A DelayNode in a cycle is actively processing only when the absolute
        // value of any output sample for the current render quantum is greater
        // than or equal to 2^−126 (smallest f32 value).
        // @note: we use the same strategy even if not in a cycle
        let mut is_actively_processing = false;

        // render channels aligned
        for (channel_number, output_channel) in output.channels_mut().iter_mut().enumerate() {
            // store channel data locally and update pointer only when needed
            let mut block_index = playback_infos[0].prev_block_index;
            let mut channel_data = ring_buffer[block_index].channel_data(channel_number);

            output_channel
                .iter_mut()
                .zip(playback_infos.iter_mut())
                .for_each(|(o, infos)| {
                    let PlaybackInfo {
                        prev_block_index,
                        prev_frame_index,
                        k,
                    } = *infos;

                    // find next sample address
                    let mut next_block_index = prev_block_index;
                    let mut next_frame_index = prev_frame_index + 1;

                    if next_frame_index >= RENDER_QUANTUM_SIZE {
                        next_block_index = (next_block_index + 1) % ring_buffer.len();
                        next_frame_index = 0;
                    }

                    // update pointer to channel_data if needed
                    // @note: most of the time the step is not necessary but can
                    // be in case of an automotation with increasing delay time
                    if block_index != prev_block_index {
                        block_index = prev_block_index;
                        channel_data = ring_buffer[block_index].channel_data(channel_number);
                    }

                    let prev_sample = channel_data[prev_frame_index];

                    // update pointer to channel_data if needed
                    if block_index != next_block_index {
                        block_index = next_block_index;
                        channel_data = ring_buffer[block_index].channel_data(channel_number);
                    }

                    let next_sample = channel_data[next_frame_index];

                    let value = (1. - k).mul_add(prev_sample, k * next_sample);

                    if value.is_normal() {
                        is_actively_processing = true;
                    }

                    *o = value;
                });
        }

        if !is_actively_processing {
            output.make_silent();
        }

        if matches!(self.last_written_index_checked, Some(index) if index == self.index) {
            return false;
        }

        // check if the writer has been decommissioned
        // we need this local copy because if the writer has been processed
        // before the reader, the direct check against `self.last_written_index`
        // would be true earlier than we want
        let last_written_index = self.last_written_index.get();

        if last_written_index.is_some() && self.last_written_index_checked.is_none() {
            self.last_written_index_checked = last_written_index;
        }
        // increment ring buffer cursor
        self.index = (self.index + 1) % ring_buffer.capacity();

        true
    }
}

impl DelayReader {
    #[inline(always)]
    fn get_playback_infos(
        delay: f64,
        in_cycle: bool,
        sample_index: f64,
        quantum_duration: f64,
        sample_rate: f64,
        ring_size: i32,
        ring_index: i32,
    ) -> PlaybackInfo {
        // param is already clamped to max_delay_time internally, so it is
        // safe to only check lower boundary
        let clamped_delay = if in_cycle {
            delay.max(quantum_duration)
        } else {
            delay
        };
        let num_samples = clamped_delay * sample_rate;
        // negative position of the playhead relative to this block start
        let position = sample_index - num_samples;
        let position_floored = position.floor();
        // find address of the frame in the ring buffer just before `position`
        let num_frames = RENDER_QUANTUM_SIZE as i32;

        // offset of the block in which the target sample is recorded
        // we need to be `float` here so that `floor()` behaves as expected
        let block_offset = (position_floored / num_frames as f64).floor();
        // index of the block in which the target sample is recorded
        let mut prev_block_index = ring_index + block_offset as i32;
        // unroll ring buffer is needed
        if prev_block_index < 0 {
            prev_block_index += ring_size;
        }

        // find frame index in the target block
        let mut frame_offset = position_floored as i32 % num_frames;
        // handle special 0 case
        if frame_offset == 0 {
            frame_offset = -num_frames;
        }

        let prev_frame_index = if frame_offset <= 0 {
            num_frames + frame_offset
        } else {
            // sub-quantum delay
            frame_offset
        };

        // as position is negative k will be what we expect
        let k = (position - position_floored) as f32;

        PlaybackInfo {
            prev_block_index: prev_block_index as usize,
            prev_frame_index: prev_frame_index as usize,
            k,
        }
    }
}

#[cfg(test)]
mod tests {
    use float_eq::assert_float_eq;

    use crate::context::OfflineAudioContext;
    use crate::node::AudioScheduledSourceNode;

    use super::*;

    #[test]
    fn test_audioparam_value_applies_immediately() {
        let context = OfflineAudioContext::new(1, 128, 48_000.);
        let options = DelayOptions {
            delay_time: 0.12,
            ..Default::default()
        };
        let src = DelayNode::new(&context, options);
        assert_float_eq!(src.delay_time.value(), 0.12, abs_all <= 0.);
    }

    #[test]
    fn test_sample_accurate() {
        for delay_in_samples in [128., 131., 197.].iter() {
            let sample_rate = 48_000.;
            let mut context = OfflineAudioContext::new(1, 256, sample_rate);

            let delay = context.create_delay(2.);
            delay.delay_time.set_value(delay_in_samples / sample_rate);
            delay.connect(&context.destination());

            let mut dirac = context.create_buffer(1, 1, sample_rate);
            dirac.copy_to_channel(&[1.], 0);

            let mut src = context.create_buffer_source();
            src.connect(&delay);
            src.set_buffer(dirac);
            src.start_at(0.);

            let result = context.start_rendering_sync();
            let channel = result.get_channel_data(0);

            let mut expected = vec![0.; 256];
            expected[*delay_in_samples as usize] = 1.;

            assert_float_eq!(channel[..], expected[..], abs_all <= 0.00001);
        }
    }

    #[test]
    fn test_sub_sample_accurate_1() {
        let delay_in_samples = 128.5;
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, 256, sample_rate);

        let delay = context.create_delay(2.);
        delay.delay_time.set_value(delay_in_samples / sample_rate);
        delay.connect(&context.destination());

        let mut dirac = context.create_buffer(1, 1, sample_rate);
        dirac.copy_to_channel(&[1.], 0);

        let mut src = context.create_buffer_source();
        src.connect(&delay);
        src.set_buffer(dirac);
        src.start_at(0.);

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; 256];
        expected[128] = 0.5;
        expected[129] = 0.5;

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.00001);
    }

    #[test]
    fn test_sub_sample_accurate_2() {
        let delay_in_samples = 128.8;
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, 256, sample_rate);

        let delay = context.create_delay(2.);
        delay.delay_time.set_value(delay_in_samples / sample_rate);
        delay.connect(&context.destination());

        let mut dirac = context.create_buffer(1, 1, sample_rate);
        dirac.copy_to_channel(&[1.], 0);

        let mut src = context.create_buffer_source();
        src.connect(&delay);
        src.set_buffer(dirac);
        src.start_at(0.);

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; 256];
        expected[128] = 0.2;
        expected[129] = 0.8;

        assert_float_eq!(channel[..], expected[..], abs_all <= 1e-5);
    }

    #[test]
    fn test_multichannel() {
        let delay_in_samples = 128.;
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(2, 2 * 128, sample_rate);

        let delay = context.create_delay(2.);
        delay.delay_time.set_value(delay_in_samples / sample_rate);
        delay.connect(&context.destination());

        let mut two_chan_dirac = context.create_buffer(2, 256, sample_rate);
        // different channels
        two_chan_dirac.copy_to_channel(&[1.], 0);
        two_chan_dirac.copy_to_channel(&[0., 1.], 1);

        let mut src = context.create_buffer_source();
        src.connect(&delay);
        src.set_buffer(two_chan_dirac);
        src.start_at(0.);

        let result = context.start_rendering_sync();

        let channel_left = result.get_channel_data(0);
        let mut expected_left = vec![0.; 256];
        expected_left[128] = 1.;
        assert_float_eq!(channel_left[..], expected_left[..], abs_all <= 1e-5);

        let channel_right = result.get_channel_data(1);
        let mut expected_right = vec![0.; 256];
        expected_right[128 + 1] = 1.;
        assert_float_eq!(channel_right[..], expected_right[..], abs_all <= 1e-5);
    }

    #[test]
    fn test_input_number_of_channels_change() {
        let delay_in_samples = 128.;
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(2, 3 * 128, sample_rate);

        let delay = context.create_delay(2.);
        delay.delay_time.set_value(delay_in_samples / sample_rate);
        delay.connect(&context.destination());

        let mut one_chan_dirac = context.create_buffer(1, 128, sample_rate);
        one_chan_dirac.copy_to_channel(&[1.], 0);

        let mut src1 = context.create_buffer_source();
        src1.connect(&delay);
        src1.set_buffer(one_chan_dirac);
        src1.start_at(0.);

        let mut two_chan_dirac = context.create_buffer(2, 256, sample_rate);
        // the two channels are different
        two_chan_dirac.copy_to_channel(&[1.], 0);
        two_chan_dirac.copy_to_channel(&[0., 1.], 1);
        // start second buffer at next block
        let mut src2 = context.create_buffer_source();
        src2.connect(&delay);
        src2.set_buffer(two_chan_dirac);
        src2.start_at(delay_in_samples as f64 / sample_rate as f64);

        let result = context.start_rendering_sync();

        let channel_left = result.get_channel_data(0);
        let mut expected_left = vec![0.; 3 * 128];
        expected_left[128] = 1.;
        expected_left[256] = 1.;
        assert_float_eq!(channel_left[..], expected_left[..], abs_all <= 1e-5);

        let channel_right = result.get_channel_data(1);
        let mut expected_right = vec![0.; 3 * 128];
        expected_right[128] = 1.;
        expected_right[256 + 1] = 1.;
        assert_float_eq!(channel_right[..], expected_right[..], abs_all <= 1e-5);
    }

    #[test]
    fn test_node_stays_alive_long_enough() {
        // make sure there are no hidden order problem
        for _ in 0..10 {
            let sample_rate = 48_000.;
            let mut context = OfflineAudioContext::new(1, 5 * 128, sample_rate);

            // Set up a source that starts only after 5 render quanta.
            // The delay writer and reader should stay alive in this period of silence.
            // We set up the nodes in a separate block {} so they are dropped in the control thread,
            // otherwise the lifecycle rules do not kick in
            {
                let delay = context.create_delay(1.);
                delay.delay_time.set_value(128. / sample_rate);
                delay.connect(&context.destination());

                let mut dirac = context.create_buffer(1, 1, sample_rate);
                dirac.copy_to_channel(&[1.], 0);

                let mut src = context.create_buffer_source();
                src.connect(&delay);
                src.set_buffer(dirac);
                // 3rd block - play buffer
                // 4th block - play silence and dropped in render thread
                src.start_at(128. * 3. / sample_rate as f64);
            } // src and delay nodes are dropped

            let result = context.start_rendering_sync();
            let mut expected = vec![0.; 5 * 128];
            // source starts after 2 * 128 samples, then is delayed another 128
            expected[4 * 128] = 1.;

            assert_float_eq!(result.get_channel_data(0), &expected[..], abs_all <= 1e-5);
        }
    }

    #[test]
    fn test_subquantum_delay() {
        for i in 0..128 {
            let sample_rate = 48_000.;
            let mut context = OfflineAudioContext::new(1, 128, sample_rate);

            let delay = context.create_delay(1.);
            delay.delay_time.set_value(i as f32 / sample_rate);
            delay.connect(&context.destination());

            let mut dirac = context.create_buffer(1, 1, sample_rate);
            dirac.copy_to_channel(&[1.], 0);

            let mut src = context.create_buffer_source();
            src.connect(&delay);
            src.set_buffer(dirac);
            src.start_at(0.);

            let result = context.start_rendering_sync();
            let channel = result.get_channel_data(0);

            let mut expected = vec![0.; 128];
            expected[i] = 1.;

            assert_float_eq!(channel[..], expected[..], abs_all <= 1e-5);
        }
    }

    #[test]
    fn test_min_delay_when_in_loop() {
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, 256, sample_rate);

        let delay = context.create_delay(1.);
        delay.delay_time.set_value(1. / sample_rate);
        delay.connect(&context.destination());
        // create a loop with a gain at 0 to avoid feedback
        // therefore delay_time will be clamped to 128 * sample_rate by the Reader
        let gain = context.create_gain();
        gain.gain().set_value(0.);
        delay.connect(&gain);
        gain.connect(&delay);

        let mut dirac = context.create_buffer(1, 1, sample_rate);
        dirac.copy_to_channel(&[1.], 0);

        let mut src = context.create_buffer_source();
        src.connect(&delay);
        src.set_buffer(dirac);
        src.start_at(0.);

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; 256];
        expected[128] = 1.;

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }

    // reproduce wpt tests from
    // - the-delaynode-interface/delaynode-max-default-delay.html
    // - the-delaynode-interface/delaynode-max-nondefault-delay.html
    #[test]
    fn test_max_delay() {
        use std::f32::consts::PI;

        for &delay_time_seconds in [1., 1.5].iter() {
            let sample_rate = 44100.0;
            let render_length = 4 * sample_rate as usize;

            let mut context = OfflineAudioContext::new(1, render_length, sample_rate);

            // create 2 seconds tone buffer at 20Hz
            let tone_frequency = 20.;
            let tone_length_seconds = 2.;
            let tone_length = tone_length_seconds as usize * sample_rate as usize;
            let mut tone_buffer = context.create_buffer(1, tone_length, sample_rate);
            let tone_data = tone_buffer.get_channel_data_mut(0);

            for (i, s) in tone_data.iter_mut().enumerate() {
                *s = (tone_frequency * 2.0 * PI * i as f32 / sample_rate).sin();
            }

            let mut buffer_source = context.create_buffer_source();
            buffer_source.set_buffer(tone_buffer.clone());

            let delay = context.create_delay(delay_time_seconds); // max delay defaults to 1 second
            delay.delay_time.set_value(delay_time_seconds as f32);

            buffer_source.connect(&delay);
            delay.connect(&context.destination());
            buffer_source.start_at(0.);

            let output = context.start_rendering_sync();
            let source = tone_buffer.get_channel_data(0);
            let rendered = output.get_channel_data(0);

            let delay_time_frames = (delay_time_seconds * sample_rate as f64) as usize;
            let tone_length_frames = (tone_length_seconds * sample_rate as f64) as usize;

            for (i, s) in rendered.iter().enumerate() {
                if i < delay_time_frames {
                    assert_eq!(*s, 0.);
                } else if i >= delay_time_frames && i < delay_time_frames + tone_length_frames {
                    let j = i - delay_time_frames;
                    assert_eq!(*s, source[j]);
                } else {
                    assert_eq!(*s, 0.);
                }
            }
        }
    }

    #[test]
    fn test_max_delay_smaller_than_quantum_size() {
        // regression test that even if the declared max_delay_time is smaller than
        // a quantum duration, the node internally clamps it to quantum duration so
        // that everything works even if order of processing is not guaranteed
        // (i.e. when delay is in a loop)
        for _ in 0..10 {
            let sample_rate = 48_000.;
            let mut context = OfflineAudioContext::new(1, 256, sample_rate);

            // this will be internally clamped to 128 * sample_rate
            let delay = context.create_delay((64. / sample_rate).into());
            // this will be clamped to 128 * sample_rate by the Reader
            delay.delay_time.set_value(64. / sample_rate);
            delay.connect(&context.destination());

            // create a loop with a gain at 0 to avoid feedback
            let gain = context.create_gain();
            gain.gain().set_value(0.);
            delay.connect(&gain);
            gain.connect(&delay);

            let mut dirac = context.create_buffer(1, 1, sample_rate);
            dirac.copy_to_channel(&[1.], 0);

            let mut src = context.create_buffer_source();
            src.connect(&delay);
            src.set_buffer(dirac);
            src.start_at(0.);

            let result = context.start_rendering_sync();
            let channel = result.get_channel_data(0);

            let mut expected = vec![0.; 256];
            expected[128] = 1.;

            assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
        }
    }

    // test_max_delay_multiple_of_quantum_size_x
    // are regression test that delay node has always enough internal buffer size
    // when max_delay is a multiple of quantum size and delay == max_delay.
    // This bug only occurs when the Writer is called before than the Reader,
    // which is the case when not in a loop
    #[test]
    fn test_max_delay_multiple_of_quantum_size_1() {
        // set delay and max delay time exactly 1 render quantum
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, 256, sample_rate);

        let max_delay = 128. / sample_rate;
        let delay = context.create_delay(max_delay.into());
        delay.delay_time.set_value(max_delay);
        delay.connect(&context.destination());

        let mut dirac = context.create_buffer(1, 1, sample_rate);
        dirac.copy_to_channel(&[1.], 0);

        let mut src = context.create_buffer_source();
        src.connect(&delay);
        src.set_buffer(dirac);
        src.start_at(0.);

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; 256];
        expected[128] = 1.;

        assert_float_eq!(channel[..], expected[..], abs_all <= 1e-5);
    }

    #[test]
    fn test_max_delay_multiple_of_quantum_size_2() {
        // set delay and max delay time exactly 2 render quantum
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, 3 * 128, sample_rate);

        let max_delay = 128. * 2. / sample_rate;
        let delay = context.create_delay(max_delay.into());
        delay.delay_time.set_value(max_delay);
        delay.connect(&context.destination());

        let mut dirac = context.create_buffer(1, 1, sample_rate);
        dirac.copy_to_channel(&[1.], 0);

        let mut src = context.create_buffer_source();
        src.connect(&delay);
        src.set_buffer(dirac);
        src.start_at(0.);

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; 3 * 128];
        expected[256] = 1.;

        assert_float_eq!(channel[..], expected[..], abs_all <= 1e-5);
    }

    #[test]
    fn test_subquantum_delay_dynamic_lifetime() {
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, 3 * 128, sample_rate);

        // Setup a source that emits for 120 frames, so it deallocates after the first render
        // quantum. Delay the signal with 64 frames. Deallocation of the delay writer might trick
        // the delay reader into thinking it is part of a cycle, and would clamp the delay to a
        // full render quantum.
        {
            let delay = context.create_delay(1.);
            delay.delay_time.set_value(64_f32 / sample_rate);
            delay.connect(&context.destination());

            // emit 120 samples
            let mut src = context.create_constant_source();
            src.connect(&delay);
            src.start_at(0.);
            src.stop_at(120. / sample_rate as f64);
        } // drop all nodes, trigger dynamic lifetimes

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; 3 * 128];
        expected[64..64 + 120].fill(1.);

        assert_float_eq!(channel[..], expected[..], abs_all <= 1e-5);
    }
}
