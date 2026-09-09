use std::any::Any;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::buffer::AudioBuffer;
use crate::context::{AudioContextRegistration, AudioParamId, BaseAudioContext};
use crate::param::{AudioParam, AudioParamDescriptor, AutomationRate};
use crate::render::{
    AudioParamValues, AudioProcessor, AudioRenderQuantum, AudioWorkletGlobalScope,
};
use crate::{assert_valid_time_value, AtomicF64, RENDER_QUANTUM_SIZE};

use super::{AudioNode, AudioScheduledSourceNode, ChannelConfig};

/// Options for constructing an [`AudioBufferSourceNode`]
// dictionary AudioBufferSourceOptions {
//   AudioBuffer? buffer;
//   float detune = 0;
//   boolean loop = false;
//   double loopEnd = 0;
//   double loopStart = 0;
//   float playbackRate = 1;
// };
//
// @note - Does extend AudioNodeOptions but they are useless for source nodes as
// they instruct how to upmix the inputs.
// This is a common source of confusion, see e.g. https://github.com/mdn/content/pull/18472, and
// an issue in the spec, see discussion in https://github.com/WebAudio/web-audio-api/issues/2496
#[derive(Clone, Debug)]
pub struct AudioBufferSourceOptions {
    pub buffer: Option<AudioBuffer>,
    pub detune: f32,
    pub loop_: bool,
    pub loop_start: f64,
    pub loop_end: f64,
    pub playback_rate: f32,
}

impl Default for AudioBufferSourceOptions {
    fn default() -> Self {
        Self {
            buffer: None,
            detune: 0.,
            loop_: false,
            loop_start: 0.,
            loop_end: 0.,
            playback_rate: 1.,
        }
    }
}

#[derive(Debug, Copy, Clone)]
struct PlaybackInfo {
    prev_frame_index: usize,
    k: f64,
}

#[derive(Debug, Clone, Copy)]
struct LoopState {
    pub is_looping: bool,
    pub start: f64,
    pub end: f64,
}

/// Instructions to start or stop processing
#[derive(Debug, Clone)]
enum ControlMessage {
    StartWithOffsetAndDuration(f64, f64, f64),
    Stop(f64),
    Loop(bool),
    LoopStart(f64),
    LoopEnd(f64),
}

/// `AudioBufferSourceNode` represents an audio source that consists of an
/// in-memory audio source (i.e. an audio file completely loaded in memory),
/// stored in an [`AudioBuffer`].
///
/// - MDN documentation: <https://developer.mozilla.org/en-US/docs/Web/API/AudioBufferSourceNode>
/// - specification: <https://webaudio.github.io/web-audio-api/#AudioBufferSourceNode>
/// - see also: [`BaseAudioContext::create_buffer_source`]
///
/// # Usage
///
/// ```no_run
/// use std::fs::File;
/// use web_audio_api::context::{BaseAudioContext, AudioContext};
/// use web_audio_api::node::{AudioNode, AudioScheduledSourceNode};
///
/// // create an `AudioContext`
/// let context = AudioContext::default();
/// // load and decode a soundfile
/// let file = File::open("samples/sample.wav").unwrap();
/// let audio_buffer = context.decode_audio_data_sync(file).unwrap();
/// // play the sound file
/// let mut src = context.create_buffer_source();
/// src.set_buffer(audio_buffer);
/// src.connect(&context.destination());
/// src.start();
/// ```
///
/// # Examples
///
/// - `cargo run --release --example trigger_soundfile`
/// - `cargo run --release --example granular`
///
#[derive(Debug)]
pub struct AudioBufferSourceNode {
    registration: AudioContextRegistration,
    channel_config: ChannelConfig,
    detune: AudioParam,        // has constraints, no a-rate
    playback_rate: AudioParam, // has constraints, no a-rate
    buffer_time: Arc<AtomicF64>,
    buffer: Option<AudioBuffer>,
    loop_state: LoopState,
    has_start: bool,
}

impl AudioNode for AudioBufferSourceNode {
    fn registration(&self) -> &AudioContextRegistration {
        &self.registration
    }

    fn channel_config(&self) -> &ChannelConfig {
        &self.channel_config
    }

    fn number_of_inputs(&self) -> usize {
        0
    }

    fn number_of_outputs(&self) -> usize {
        1
    }
}

impl AudioScheduledSourceNode for AudioBufferSourceNode {
    fn start(&mut self) {
        let start = self.registration.context().current_time();
        self.start_at_with_offset_and_duration(start, 0., f64::MAX);
    }

    fn start_at(&mut self, when: f64) {
        self.start_at_with_offset_and_duration(when, 0., f64::MAX);
    }

    fn stop(&mut self) {
        let stop = self.registration.context().current_time();
        self.stop_at(stop);
    }

    fn stop_at(&mut self, when: f64) {
        assert_valid_time_value(when);
        assert!(
            self.has_start,
            "InvalidStateError - cannot stop before start"
        );

        self.registration.post_message(ControlMessage::Stop(when));
    }
}

impl AudioBufferSourceNode {
    /// Create a new [`AudioBufferSourceNode`] instance
    pub fn new<C: BaseAudioContext>(context: &C, options: AudioBufferSourceOptions) -> Self {
        let AudioBufferSourceOptions {
            buffer,
            detune,
            loop_,
            loop_start,
            loop_end,
            playback_rate,
        } = options;

        let mut node = context.base().register(move |registration| {
            // these parameters can't be changed to a-rate
            // @see - <https://webaudio.github.io/web-audio-api/#audioparam-automation-rate-constraints>
            let detune_param_options = AudioParamDescriptor {
                name: String::new(),
                min_value: f32::MIN,
                max_value: f32::MAX,
                default_value: 0.,
                automation_rate: AutomationRate::K,
            };
            let (mut d_param, d_proc) =
                context.create_audio_param(detune_param_options, &registration);
            d_param.set_automation_rate_constrained(true);
            d_param.set_value(detune);

            let playback_rate_param_options = AudioParamDescriptor {
                name: String::new(),
                min_value: f32::MIN,
                max_value: f32::MAX,
                default_value: 1.,
                automation_rate: AutomationRate::K,
            };
            let (mut pr_param, pr_proc) =
                context.create_audio_param(playback_rate_param_options, &registration);
            pr_param.set_automation_rate_constrained(true);
            pr_param.set_value(playback_rate);

            let loop_state = LoopState {
                is_looping: loop_,
                start: loop_start,
                end: loop_end,
            };

            let renderer = AudioBufferSourceRenderer {
                start_time: f64::MAX,
                stop_time: f64::MAX,
                duration: f64::MAX,
                offset: 0.,
                buffer: None,
                detune: d_proc,
                playback_rate: pr_proc,
                loop_state,
                render_state: AudioBufferRendererState::default(),
            };

            let node = Self {
                registration,
                channel_config: ChannelConfig::default(),
                detune: d_param,
                playback_rate: pr_param,
                buffer_time: Arc::clone(&renderer.render_state.buffer_time),
                buffer: None,
                loop_state,
                has_start: false,
            };

            (node, Box::new(renderer))
        });

        // renderer has been sent to render thread, we can send it messages
        if let Some(buf) = buffer {
            node.set_buffer(buf);
        }

        node
    }

    /// Start the playback at the given time and with a given offset
    ///
    /// # Panics
    ///
    /// Panics if the source was already started
    pub fn start_at_with_offset(&mut self, start: f64, offset: f64) {
        self.start_at_with_offset_and_duration(start, offset, f64::MAX);
    }

    /// Start the playback at the given time, with a given offset, for a given duration
    ///
    /// # Panics
    ///
    /// Panics if the source was already started
    pub fn start_at_with_offset_and_duration(&mut self, start: f64, offset: f64, duration: f64) {
        assert_valid_time_value(start);
        assert_valid_time_value(offset);
        assert_valid_time_value(duration);
        assert!(
            !self.has_start,
            "InvalidStateError - Cannot call `start` twice"
        );

        self.has_start = true;
        let control = ControlMessage::StartWithOffsetAndDuration(start, offset, duration);
        self.registration.post_message(control);
    }

    /// Current buffer value (nullable)
    pub fn buffer(&self) -> Option<&AudioBuffer> {
        self.buffer.as_ref()
    }

    /// Provide an [`AudioBuffer`] as the source of data to be played bask
    ///
    /// # Panics
    ///
    /// Panics if a buffer has already been given to the source (though `new` or through
    /// `set_buffer`)
    pub fn set_buffer(&mut self, audio_buffer: AudioBuffer) {
        let clone = audio_buffer.clone();

        assert!(
            self.buffer.is_none(),
            "InvalidStateError - cannot assign buffer twice",
        );
        self.buffer = Some(audio_buffer);

        self.registration.post_message(clone);
    }

    /// K-rate [`AudioParam`] that defines the speed at which the [`AudioBuffer`]
    /// will be played, e.g.:
    /// - `0.5` will play the file at half speed
    /// - `-1` will play the file in reverse
    ///
    /// Note that playback rate will also alter the pitch of the [`AudioBuffer`]
    pub fn playback_rate(&self) -> &AudioParam {
        &self.playback_rate
    }

    /// Current playhead position in seconds within the [`AudioBuffer`].
    ///
    /// This value is updated at the end of each render quantum.
    ///
    /// Unofficial v2 API extension, not part of the spec yet.
    /// See also: <https://github.com/WebAudio/web-audio-api/issues/2397#issuecomment-709478405>
    pub fn position(&self) -> f64 {
        self.buffer_time.load(Ordering::Relaxed)
    }

    /// K-rate [`AudioParam`] that defines a pitch transposition of the file,
    /// expressed in cents
    ///
    /// see <https://en.wikipedia.org/wiki/Cent_(music)>
    pub fn detune(&self) -> &AudioParam {
        &self.detune
    }

    /// Defines if the playback the [`AudioBuffer`] should be looped
    #[allow(clippy::missing_panics_doc)]
    pub fn loop_(&self) -> bool {
        self.loop_state.is_looping
    }

    pub fn set_loop(&mut self, value: bool) {
        self.loop_state.is_looping = value;
        self.registration.post_message(ControlMessage::Loop(value));
    }

    /// Defines the loop start point, in the time reference of the [`AudioBuffer`]
    pub fn loop_start(&self) -> f64 {
        self.loop_state.start
    }

    pub fn set_loop_start(&mut self, value: f64) {
        self.loop_state.start = value;
        self.registration
            .post_message(ControlMessage::LoopStart(value));
    }

    /// Defines the loop end point, in the time reference of the [`AudioBuffer`]
    pub fn loop_end(&self) -> f64 {
        self.loop_state.end
    }

    pub fn set_loop_end(&mut self, value: f64) {
        self.loop_state.end = value;
        self.registration
            .post_message(ControlMessage::LoopEnd(value));
    }
}

struct AudioBufferRendererState {
    buffer_time: Arc<AtomicF64>,
    started: bool,
    entered_loop: bool,
    buffer_time_elapsed: f64,
    is_aligned: bool,
    ended: bool,
}

impl Default for AudioBufferRendererState {
    fn default() -> Self {
        Self {
            buffer_time: Arc::new(AtomicF64::new(0.)),
            started: false,
            entered_loop: false,
            buffer_time_elapsed: 0.,
            is_aligned: false,
            ended: false,
        }
    }
}

struct AudioBufferSourceRenderer {
    start_time: f64,
    stop_time: f64,
    offset: f64,
    duration: f64,
    buffer: Option<AudioBuffer>,
    detune: AudioParamId,
    playback_rate: AudioParamId,
    loop_state: LoopState,
    render_state: AudioBufferRendererState,
}

impl AudioBufferSourceRenderer {
    fn handle_control_message(&mut self, control: &ControlMessage) {
        match control {
            ControlMessage::StartWithOffsetAndDuration(when, offset, duration) => {
                self.start_time = *when;
                self.offset = *offset;
                self.duration = *duration;
            }
            ControlMessage::Stop(when) => self.stop_time = *when,
            ControlMessage::Loop(is_looping) => self.loop_state.is_looping = *is_looping,
            ControlMessage::LoopStart(loop_start) => self.loop_state.start = *loop_start,
            ControlMessage::LoopEnd(loop_end) => self.loop_state.end = *loop_end,
        }

        self.clamp_loop_boundaries();
    }

    fn clamp_loop_boundaries(&mut self) {
        if let Some(buffer) = &self.buffer {
            let duration = buffer.duration();

            // https://webaudio.github.io/web-audio-api/#dom-audiobuffersourcenode-loopstart
            if self.loop_state.start < 0. {
                self.loop_state.start = 0.;
            } else if self.loop_state.start > duration {
                self.loop_state.start = duration;
            }

            // https://webaudio.github.io/web-audio-api/#dom-audiobuffersourcenode-loopend
            if self.loop_state.end <= 0. || self.loop_state.end > duration {
                self.loop_state.end = duration;
            }
        }
    }
}

impl AudioProcessor for AudioBufferSourceRenderer {
    fn process(
        &mut self,
        _inputs: &[AudioRenderQuantum], // This node has no input
        outputs: &mut [AudioRenderQuantum],
        params: AudioParamValues<'_>,
        scope: &AudioWorkletGlobalScope,
    ) -> bool {
        // Single output node
        let output = &mut outputs[0];

        if self.render_state.ended {
            output.make_silent();
            return false;
        }

        let sample_rate = scope.sample_rate as f64;
        let dt = 1. / sample_rate;
        let block_duration = dt * RENDER_QUANTUM_SIZE as f64;
        let next_block_time = scope.current_time + block_duration;

        // if start called and buffer is null, should fire ended event and ignore
        // any subsequent buffer assignment.
        // cf. wpt/webaudio/the-audio-api/the-audiobuffersourcenode-interface/audiobuffersource-start-null-buffer.html
        if self.buffer.is_none() && self.start_time != f64::MAX {
            output.make_silent();
            self.render_state.ended = true;
            scope.send_ended_event();
            return false;
        }

        // Return early if start_time is beyond this block
        if self.start_time >= next_block_time {
            output.make_silent();
            // stop before start
            if self.stop_time <= next_block_time {
                self.render_state.ended = true;
                scope.send_ended_event();
                return false;
            }

            // #462 AudioScheduledSourceNodes that have not been scheduled to start can safely
            // return tail_time false in order to be collected if their control handle drops.
            return self.start_time != f64::MAX;
        }

        // If the buffer has not been set wait for it.
        let buffer = match &self.buffer {
            None => {
                output.make_silent();
                // #462 like the above arm, we can safely return tail_time false
                // if this node has no buffer set.
                return false;
            }
            Some(b) => b,
        };

        let LoopState {
            is_looping,
            start: loop_start,
            end: loop_end,
        } = self.loop_state;

        // these will only be used if `loop_` is true, so no need for `Option`
        let mut actual_loop_start = 0.;
        let mut actual_loop_end = 0.;

        // compute compound parameter at k-rate, these parameters have constraints
        // https://webaudio.github.io/web-audio-api/#audioparam-automation-rate-constraints
        let detune = params.get(&self.detune)[0] as f64;
        let playback_rate = params.get(&self.playback_rate)[0] as f64;
        let computed_playback_rate = playback_rate * (detune / 1200.).exp2();

        let buffer_length = buffer.length();
        let buffer_duration = buffer.duration();
        // multiplier to be applied on `position` to tackle possible difference
        // between the context and buffer sample rates. As this is an edge case,
        // we just linearly interpolate, thus favoring performance vs quality
        let sampling_ratio = buffer.sample_rate() as f64 / sample_rate;

        // Load the buffer time from the render state.
        // The render state has to be updated before leaving this method!
        let mut buffer_time = self.render_state.buffer_time.load(Ordering::Relaxed);

        output.set_number_of_channels(buffer.number_of_channels());

        // go through the algorithm described in the spec
        // @see <https://webaudio.github.io/web-audio-api/#playback-AudioBufferSourceNode>
        let block_time = scope.current_time;

        // prevent scheduling in the past
        // If 0 is passed in for this value or if the value is less than
        // currentTime, then the sound will start playing immediately
        // cf. https://webaudio.github.io/web-audio-api/#dom-audioscheduledsourcenode-start-when-when
        if !self.render_state.started && self.start_time < block_time {
            self.start_time = block_time;
        }

        // Define if we can avoid the resampling interpolation in some common cases,
        // basically when:
        // - `src.start()` is called with `audio_context.current_time`,
        //   i.e. start time is aligned with a render quantum block
        // - the AudioBuffer was decoded w/ the right sample rate
        // - no detune or playback_rate changes are made
        // - loop boundaries have not been changed
        if self.start_time == block_time && self.offset == 0. {
            self.render_state.is_aligned = true;
        }

        // these two case imply resampling
        if sampling_ratio != 1. || computed_playback_rate != 1. {
            self.render_state.is_aligned = false;
        }

        // If loop points are not aligned on sample, they can imply resampling.
        // For now we just consider that we can go fast track if loop points are
        // bound to the buffer boundaries.
        //
        // By default, cf. clamp_loop_boundaries, loop_start == 0 && loop_end == buffer_duration,
        if loop_start != 0. || loop_end != buffer_duration {
            self.render_state.is_aligned = false;
        }

        // If some user defined end of rendering, i.e. explicit stop_time or duration,
        // is within this render quantum force slow track as well. It might imply
        // resampling e.g. if stop_time is between 2 samples
        if buffer_time + block_duration > self.duration
            || block_time + block_duration > self.stop_time
        {
            self.render_state.is_aligned = false;
        }

        if self.render_state.is_aligned {
            // ---------------------------------------------------------------
            // Fast track
            // ---------------------------------------------------------------
            if self.start_time == block_time {
                self.render_state.started = true;
            }

            // buffer ends within this block
            if buffer_time + block_duration > buffer_duration {
                let end_index = buffer.length();
                // In case of a loop point in the middle of the block, this value will
                // be used to recompute `buffer_time` according to the actual loop point.
                let mut loop_point_index: Option<usize> = None;

                buffer
                    .channels()
                    .iter()
                    .zip(output.channels_mut().iter_mut())
                    .for_each(|(buffer_channel, output_channel)| {
                        // we need to recompute that for each channel
                        let buffer_channel = buffer_channel.as_slice();
                        let mut start_index = (buffer_time * sample_rate).round() as usize;
                        let mut offset = 0;

                        for (index, o) in output_channel.iter_mut().enumerate() {
                            let mut buffer_index = start_index + index - offset;

                            *o = if buffer_index < end_index {
                                buffer_channel[buffer_index]
                            } else {
                                if is_looping && buffer_index >= end_index {
                                    loop_point_index = Some(index);
                                    // reset values for the rest of the block
                                    start_index = 0;
                                    offset = index;
                                    buffer_index = 0;
                                }

                                if is_looping {
                                    buffer_channel[buffer_index]
                                } else {
                                    0.
                                }
                            };
                        }
                    });

                if let Some(loop_point_index) = loop_point_index {
                    buffer_time = ((RENDER_QUANTUM_SIZE - loop_point_index) as f64 / sample_rate)
                        % buffer_duration;
                } else {
                    buffer_time += block_duration;
                }
            } else {
                let start_index = (buffer_time * sample_rate).round() as usize;
                let end_index = start_index + RENDER_QUANTUM_SIZE;
                // we can do memcopy
                buffer
                    .channels()
                    .iter()
                    .zip(output.channels_mut().iter_mut())
                    .for_each(|(buffer_channel, output_channel)| {
                        let buffer_channel = buffer_channel.as_slice();
                        output_channel.copy_from_slice(&buffer_channel[start_index..end_index]);
                    });

                buffer_time += block_duration;
            }

            self.render_state.buffer_time_elapsed += block_duration;
        } else {
            // ---------------------------------------------------------------
            // Slow track
            // ---------------------------------------------------------------
            if is_looping {
                if loop_start >= 0. && loop_end > 0. && loop_start < loop_end {
                    actual_loop_start = loop_start;
                    actual_loop_end = loop_end;
                } else {
                    actual_loop_start = 0.;
                    actual_loop_end = buffer_duration;
                }
            } else {
                self.render_state.entered_loop = false;
            }

            // internal buffer used to store playback infos to compute the samples
            // according to the source buffer. (prev_sample_index, k)
            let mut playback_infos = [None; RENDER_QUANTUM_SIZE];

            // compute position for each sample and store into `self.positions`
            for (i, playback_info) in playback_infos.iter_mut().enumerate() {
                let current_time = block_time + i as f64 * dt;

                // Sticky behavior to handle floating point errors due to start time computation
                // cf. test_subsample_buffer_stitching
                if !self.render_state.started && almost::equal(current_time, self.start_time) {
                    self.start_time = current_time;
                }

                if almost::equal(self.render_state.buffer_time_elapsed, self.duration) {
                    self.render_state.buffer_time_elapsed = self.duration;
                }

                // Handle following cases:
                // - we are before start time
                // - we are after stop time
                // - explicit duration (in buffer time reference) has been given and we have reached it
                // Note that checking against buffer duration is done below to handle looping
                if current_time < self.start_time
                    || current_time >= self.stop_time
                    || self.render_state.buffer_time_elapsed >= self.duration
                {
                    continue; // nothing more to do for this sample
                }

                // we have now reached start time
                if !self.render_state.started {
                    let delta = current_time - self.start_time;
                    // handle that start time may be between last sample and this one
                    self.offset += delta * computed_playback_rate;
                    // clamp offset to buffer boundaries
                    self.offset = self.offset.max(0.).min(buffer_duration);

                    if is_looping && computed_playback_rate >= 0. && self.offset > actual_loop_end {
                        self.offset = actual_loop_end;
                    }

                    if is_looping && computed_playback_rate < 0. && self.offset < actual_loop_start
                    {
                        self.offset = actual_loop_start;
                    }

                    buffer_time = self.offset;
                    self.render_state.buffer_time_elapsed = (delta * computed_playback_rate).abs();
                    self.render_state.started = true;
                }

                if is_looping {
                    if almost::equal(buffer_time, actual_loop_end) {
                        buffer_time = actual_loop_end;
                    }

                    if almost::equal(buffer_time, actual_loop_start) {
                        buffer_time = actual_loop_start;
                    }

                    if !self.render_state.entered_loop {
                        // playback began before or within loop, and playhead is now past loop start
                        if self.offset < actual_loop_end && buffer_time >= actual_loop_start {
                            self.render_state.entered_loop = true;
                        }

                        // playback began after loop, and playhead is now prior to the loop end
                        if self.offset >= actual_loop_end && buffer_time < actual_loop_end {
                            self.render_state.entered_loop = true;
                        }
                    }

                    // check loop boundaries
                    if self.render_state.entered_loop {
                        while buffer_time >= actual_loop_end {
                            buffer_time -= actual_loop_end - actual_loop_start;
                        }

                        while buffer_time < actual_loop_start {
                            buffer_time += actual_loop_end - actual_loop_start;
                        }
                    }
                }

                if almost::zero(buffer_time) {
                    buffer_time = 0.
                }

                if buffer_time >= 0. && buffer_time < buffer_duration {
                    let position = buffer_time * sampling_ratio;
                    let playhead = position * sample_rate;
                    let playhead_floored = playhead.floor();
                    let prev_frame_index = playhead_floored as usize; // can't be < 0.
                    let k = playhead - playhead_floored;

                    // Due to how buffer_time is computed, we can still run into
                    // floating point errors and try to access a non existing index
                    // cf. test_end_of_file_slow_track_2
                    if prev_frame_index < buffer_length {
                        *playback_info = Some(PlaybackInfo {
                            prev_frame_index,
                            k,
                        });
                    }
                }

                let time_incr = dt * computed_playback_rate;
                buffer_time += time_incr;
                self.render_state.buffer_time_elapsed += time_incr.abs();
            }

            // fill output according to computed positions
            buffer
                .channels()
                .iter()
                .zip(output.channels_mut().iter_mut())
                .for_each(|(buffer_channel, output_channel)| {
                    let buffer_channel = buffer_channel.as_slice();

                    playback_infos
                        .iter()
                        .zip(output_channel.iter_mut())
                        .for_each(|(playhead, o)| {
                            *o = match playhead {
                                Some(PlaybackInfo {
                                    prev_frame_index,
                                    k,
                                }) => {
                                    // `prev_frame_index` cannot be out of bounds
                                    let prev_sample = buffer_channel[*prev_frame_index] as f64;
                                    let next_sample = match buffer_channel.get(prev_frame_index + 1)
                                    {
                                        Some(val) => *val as f64,
                                        // End of buffer
                                        None => {
                                            if is_looping {
                                                if playback_rate >= 0. {
                                                    let start_playhead =
                                                        actual_loop_start * sample_rate;
                                                    let start_index = if start_playhead.floor()
                                                        == start_playhead
                                                    {
                                                        start_playhead as usize
                                                    } else {
                                                        start_playhead as usize + 1
                                                    };

                                                    buffer_channel[start_index] as f64
                                                } else {
                                                    let end_playhead =
                                                        actual_loop_end * sample_rate;
                                                    let end_index = end_playhead as usize;
                                                    buffer_channel[end_index] as f64
                                                }
                                            } else {
                                                // Handle 2 edge cases:
                                                // 1. We are in a case where buffer time is below buffer
                                                // duration due to floating point errors, but where
                                                // prev_frame_index is last index and k is near 1. We can't
                                                // filter this case before, because it might break
                                                // loops logic.
                                                // 2. Buffer contains only one sample
                                                if almost::equal(*k, 1.) || *prev_frame_index == 0 {
                                                    0.
                                                } else {
                                                    // Extrapolate next sample using the last two known samples
                                                    // cf. https://github.com/WebAudio/web-audio-api/issues/2032
                                                    let prev_prev_sample =
                                                        buffer_channel[*prev_frame_index - 1];
                                                    2. * prev_sample - prev_prev_sample as f64
                                                }
                                            }
                                        }
                                    };

                                    (1. - k).mul_add(prev_sample, k * next_sample) as f32
                                }
                                None => 0.,
                            };
                        });
                });
        }

        // Update render state
        self.render_state
            .buffer_time
            .store(buffer_time, Ordering::Relaxed);

        // The buffer has ended within this block, if one of the following conditions holds:
        // 1. the stop time has been reached.
        // 2. the duration has been reached.
        // 3. the end of the buffer has been reached.
        if next_block_time >= self.stop_time
            || self.render_state.buffer_time_elapsed >= self.duration
            || !is_looping
                && (computed_playback_rate > 0. && buffer_time >= buffer_duration
                    || computed_playback_rate < 0. && buffer_time < 0.)
        {
            self.render_state.ended = true;
            scope.send_ended_event();
        }

        true
    }

    fn onmessage(&mut self, msg: &mut dyn Any) {
        if let Some(control) = msg.downcast_ref::<ControlMessage>() {
            self.handle_control_message(control);
            return;
        };

        if let Some(buffer) = msg.downcast_mut::<AudioBuffer>() {
            if let Some(current_buffer) = &mut self.buffer {
                // Avoid deallocation in the render thread by swapping the buffers.
                std::mem::swap(current_buffer, buffer);
            } else {
                // Creating the tombstone buffer does not cause allocations.
                let tombstone_buffer = AudioBuffer {
                    channels: Default::default(),
                    sample_rate: Default::default(),
                };
                self.buffer = Some(std::mem::replace(buffer, tombstone_buffer));
                self.clamp_loop_boundaries();
            }
            return;
        };

        log::warn!("AudioBufferSourceRenderer: Dropping incoming message {msg:?}");
    }

    fn before_drop(&mut self, scope: &AudioWorkletGlobalScope) {
        if !self.render_state.ended
            && (scope.current_time >= self.start_time || scope.current_time >= self.stop_time)
        {
            scope.send_ended_event();
            self.render_state.ended = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use float_eq::assert_float_eq;
    use std::f32::consts::PI;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use crate::context::{BaseAudioContext, OfflineAudioContext};
    use crate::AudioBufferOptions;
    use crate::RENDER_QUANTUM_SIZE;

    use super::*;

    #[test]
    fn test_construct_with_options_and_run() {
        let sample_rate = 44100.;
        let length = RENDER_QUANTUM_SIZE;
        let mut context = OfflineAudioContext::new(1, length, sample_rate);

        let buffer = AudioBuffer::from(vec![vec![1.; RENDER_QUANTUM_SIZE]], sample_rate);
        let options = AudioBufferSourceOptions {
            buffer: Some(buffer),
            ..Default::default()
        };
        let mut src = AudioBufferSourceNode::new(&context, options);
        src.connect(&context.destination());
        src.start();
        let res = context.start_rendering_sync();

        assert_float_eq!(
            res.channel_data(0).as_slice()[..],
            &[1.; RENDER_QUANTUM_SIZE][..],
            abs_all <= 0.
        );
    }

    #[test]
    fn test_playing_some_file() {
        let context = OfflineAudioContext::new(2, RENDER_QUANTUM_SIZE, 44_100.);

        let file = std::fs::File::open("samples/sample.wav").unwrap();
        let expected = context.decode_audio_data_sync(file).unwrap();

        // 44100 will go through fast track
        // 48000 will go through slow track
        [44100, 48000].iter().for_each(|sr| {
            let decoding_context = OfflineAudioContext::new(2, RENDER_QUANTUM_SIZE, *sr as f32);

            let mut filename = "samples/sample-".to_owned();
            filename.push_str(&sr.to_string());
            filename.push_str(".wav");

            let file = std::fs::File::open("samples/sample.wav").unwrap();
            let audio_buffer = decoding_context.decode_audio_data_sync(file).unwrap();

            assert_eq!(audio_buffer.sample_rate(), *sr as f32);

            let mut context = OfflineAudioContext::new(2, RENDER_QUANTUM_SIZE, 44_100.);

            let mut src = context.create_buffer_source();
            src.set_buffer(audio_buffer);
            src.connect(&context.destination());
            src.start_at(context.current_time());
            src.stop_at(context.current_time() + 128.);

            let res = context.start_rendering_sync();
            let diff_abs = if *sr == 44100 {
                0. // fast track
            } else {
                5e-3 // slow track w/ linear interpolation
            };

            // asserting length() is meaningless as this is controlled by the context
            assert_eq!(res.number_of_channels(), expected.number_of_channels());

            // check first 128 samples in left and right channels
            assert_float_eq!(
                res.channel_data(0).as_slice()[..],
                expected.get_channel_data(0)[0..128],
                abs_all <= diff_abs
            );

            assert_float_eq!(
                res.channel_data(1).as_slice()[..],
                expected.get_channel_data(1)[0..128],
                abs_all <= diff_abs
            );
        });
    }

    // slow track
    #[test]
    fn test_sub_quantum_start_1() {
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, RENDER_QUANTUM_SIZE, sample_rate);

        let mut dirac = context.create_buffer(1, 1, sample_rate);
        dirac.copy_to_channel(&[1.], 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(dirac);
        src.start_at(1. / sample_rate as f64);

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; RENDER_QUANTUM_SIZE];
        expected[1] = 1.;

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }

    // adapted from the-audio-api/the-audiobuffersourcenode-interface/sample-accurate-scheduling.html
    #[test]
    fn test_sub_quantum_start_2() {
        let sample_rate = 44_100.;
        let length_in_seconds = 4.;
        let mut context =
            OfflineAudioContext::new(2, (length_in_seconds * sample_rate) as usize, sample_rate);

        let mut dirac = context.create_buffer(2, 512, sample_rate);
        dirac.copy_to_channel(&[1.], 0);
        dirac.copy_to_channel(&[1.], 1);

        let sample_offsets = [0, 3, 512, 517, 1000, 1005, 20000, 21234, 37590];

        sample_offsets.iter().for_each(|index| {
            let time_in_seconds = *index as f64 / sample_rate as f64;

            let mut src = context.create_buffer_source();
            src.set_buffer(dirac.clone());
            src.connect(&context.destination());
            src.start_at(time_in_seconds);
        });

        let res = context.start_rendering_sync();

        let channel_left = res.get_channel_data(0);
        let channel_right = res.get_channel_data(1);
        // assert lef and right channels are equal
        assert_float_eq!(channel_left[..], channel_right[..], abs_all <= 0.);
        // assert we got our dirac at each defined offsets

        sample_offsets.iter().for_each(|index| {
            assert_ne!(
                channel_left[*index], 0.,
                "non zero sample at index {:?}",
                index
            );
        });
    }

    #[test]
    fn test_sub_sample_start() {
        // sub sample
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, RENDER_QUANTUM_SIZE, sample_rate);

        let mut dirac = context.create_buffer(1, 1, sample_rate);
        dirac.copy_to_channel(&[1.], 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(dirac);
        src.start_at(1.5 / sample_rate as f64);

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; RENDER_QUANTUM_SIZE];
        expected[2] = 0.5;

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }

    #[test]
    fn test_sub_quantum_stop_fast_track() {
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, RENDER_QUANTUM_SIZE, sample_rate);

        let mut dirac = context.create_buffer(1, RENDER_QUANTUM_SIZE, sample_rate);
        dirac.copy_to_channel(&[0., 0., 0., 0., 1.], 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(dirac);
        src.start_at(0. / sample_rate as f64);
        // stop at time of dirac, should not be played
        src.stop_at(4. / sample_rate as f64);

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);
        let expected = vec![0.; RENDER_QUANTUM_SIZE];

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }

    #[test]
    fn test_sub_quantum_stop_slow_track() {
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, RENDER_QUANTUM_SIZE, sample_rate);

        let mut dirac = context.create_buffer(1, RENDER_QUANTUM_SIZE, sample_rate);
        dirac.copy_to_channel(&[0., 0., 0., 1.], 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(dirac);

        src.start_at(1. / sample_rate as f64);
        src.stop_at(4. / sample_rate as f64);

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);
        let expected = vec![0.; RENDER_QUANTUM_SIZE];

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }

    #[test]
    fn test_sub_sample_stop_fast_track() {
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, RENDER_QUANTUM_SIZE, sample_rate);

        let mut dirac = context.create_buffer(1, RENDER_QUANTUM_SIZE, sample_rate);
        dirac.copy_to_channel(&[0., 0., 0., 0., 1., 1.], 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(dirac);
        src.start_at(0. / sample_rate as f64);
        // stop at between two diracs, only first one should be played
        src.stop_at(4.5 / sample_rate as f64);

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; 128];
        expected[4] = 1.;

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }

    #[test]
    fn test_sub_sample_stop_slow_track() {
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, RENDER_QUANTUM_SIZE, sample_rate);

        let mut dirac = context.create_buffer(1, RENDER_QUANTUM_SIZE, sample_rate);
        dirac.copy_to_channel(&[0., 0., 0., 0., 1., 1.], 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(dirac);
        src.start_at(1. / sample_rate as f64);
        // stop at between two diracs, only first one should be played
        src.stop_at(5.5 / sample_rate as f64);

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; 128];
        expected[5] = 1.;

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }

    #[test]
    fn test_start_in_the_past() {
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, 2 * RENDER_QUANTUM_SIZE, sample_rate);

        let mut dirac = context.create_buffer(1, 1, sample_rate);
        dirac.copy_to_channel(&[1.], 0);

        context.suspend_sync((128. / sample_rate).into(), |context| {
            let mut src = context.create_buffer_source();
            src.connect(&context.destination());
            src.set_buffer(dirac);
            src.start_at(0.);
        });

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; 2 * RENDER_QUANTUM_SIZE];
        expected[128] = 1.;

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }

    #[test]
    fn test_audio_buffer_resampling() {
        [22_500, 38_000, 43_800, 48_000, 96_000]
            .iter()
            .for_each(|sr| {
                let freq = 1.;
                let base_sr = 44_100;
                let mut context = OfflineAudioContext::new(1, base_sr, base_sr as f32);

                // 1Hz sine at different sample rates
                let buf_sr = *sr;
                // safe cast for sample rate, see discussion at #113
                let sample_rate = buf_sr as f32;
                let mut buffer = context.create_buffer(1, buf_sr, sample_rate);
                let mut sine = vec![];

                for i in 0..buf_sr {
                    let phase = freq * i as f32 / buf_sr as f32 * 2. * PI;
                    let sample = phase.sin();
                    sine.push(sample);
                }

                buffer.copy_to_channel(&sine[..], 0);

                let mut src = context.create_buffer_source();
                src.connect(&context.destination());
                src.set_buffer(buffer);
                src.start_at(0. / sample_rate as f64);

                let result = context.start_rendering_sync();
                let channel = result.get_channel_data(0);

                // 1Hz sine at audio context sample rate
                let mut expected = vec![];

                for i in 0..base_sr {
                    let phase = freq * i as f32 / base_sr as f32 * 2. * PI;
                    let sample = phase.sin();
                    expected.push(sample);
                }

                assert_float_eq!(channel[..], expected[..], abs_all <= 1e-6);
            });
    }

    #[test]
    fn test_playback_rate() {
        let sample_rate = 44_100;
        let mut context = OfflineAudioContext::new(1, sample_rate, sample_rate as f32);

        let mut buffer = context.create_buffer(1, sample_rate, sample_rate as f32);
        let mut sine = vec![];

        // 1 Hz sine
        for i in 0..sample_rate {
            let phase = i as f32 / sample_rate as f32 * 2. * PI;
            let sample = phase.sin();
            sine.push(sample);
        }

        buffer.copy_to_channel(&sine[..], 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(buffer);
        src.playback_rate.set_value(0.5);
        src.start();

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        // 0.5 Hz sine
        let mut expected = vec![];

        for i in 0..sample_rate {
            let phase = i as f32 / sample_rate as f32 * PI;
            let sample = phase.sin();
            expected.push(sample);
        }

        assert_float_eq!(channel[..], expected[..], abs_all <= 1e-6);
    }

    #[test]
    fn test_negative_playback_rate() {
        let sample_rate = 44_100;
        let mut context = OfflineAudioContext::new(1, sample_rate, sample_rate as f32);

        let mut buffer = context.create_buffer(1, sample_rate, sample_rate as f32);
        let mut sine = vec![];

        // 1 Hz sine
        for i in 0..sample_rate {
            let phase = i as f32 / sample_rate as f32 * 2. * PI;
            let sample = phase.sin();
            sine.push(sample);
        }

        buffer.copy_to_channel(&sine[..], 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(buffer.clone());
        src.playback_rate.set_value(-1.);
        src.start_at_with_offset(context.current_time(), buffer.duration());

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        // -1 Hz sine
        let mut expected: Vec<f32> = sine.into_iter().rev().collect();
        // offset is at duration (after last sample), then result will start
        // with a zero value
        expected.pop();
        expected.insert(0, 0.);

        assert_float_eq!(channel[..], expected[..], abs_all <= 1e-6);
    }

    #[test]
    fn test_detune() {
        let sample_rate = 44_100;
        let mut context = OfflineAudioContext::new(1, sample_rate, sample_rate as f32);

        let mut buffer = context.create_buffer(1, sample_rate, sample_rate as f32);
        let mut sine = vec![];

        // 1 Hz sine
        for i in 0..sample_rate {
            let phase = i as f32 / sample_rate as f32 * 2. * PI;
            let sample = phase.sin();
            sine.push(sample);
        }

        buffer.copy_to_channel(&sine[..], 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(buffer);
        src.detune.set_value(-1200.);
        src.start();

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        // 0.5 Hz sine
        let mut expected = vec![];

        for i in 0..sample_rate {
            let phase = i as f32 / sample_rate as f32 * PI;
            let sample = phase.sin();
            expected.push(sample);
        }

        assert_float_eq!(channel[..], expected[..], abs_all <= 1e-6);
    }

    #[test]
    fn test_end_of_file_fast_track() {
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, RENDER_QUANTUM_SIZE * 2, sample_rate);

        let mut buffer = context.create_buffer(1, 129, sample_rate);
        let mut data = vec![0.; 129];
        data[0] = 1.;
        data[128] = 1.;
        buffer.copy_to_channel(&data, 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(buffer);
        src.start_at(0. / sample_rate as f64);

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; 256];
        expected[0] = 1.;
        expected[128] = 1.;

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }

    #[test]
    fn test_end_of_file_slow_track_1() {
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, RENDER_QUANTUM_SIZE * 2, sample_rate);

        let mut buffer = context.create_buffer(1, 129, sample_rate);
        let mut data = vec![0.; 129];
        data[0] = 1.;
        data[128] = 1.;
        buffer.copy_to_channel(&data, 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(buffer);
        src.start_at(1. / sample_rate as f64);

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; 256];
        expected[1] = 1.;
        expected[129] = 1.;

        assert_float_eq!(channel[..], expected[..], abs_all <= 1e-10);
    }

    #[test]
    fn test_with_duration_0() {
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, RENDER_QUANTUM_SIZE, sample_rate);

        let mut dirac = context.create_buffer(1, RENDER_QUANTUM_SIZE, sample_rate);
        dirac.copy_to_channel(&[0., 0., 0., 0., 1., 1.], 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(dirac);
        // duration is between two diracs, only first one should be played
        src.start_at_with_offset_and_duration(0., 0., 4.5 / sample_rate as f64);

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; 128];
        expected[4] = 1.;

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }

    #[test]
    fn test_with_duration_1() {
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, RENDER_QUANTUM_SIZE, sample_rate);

        let mut dirac = context.create_buffer(1, RENDER_QUANTUM_SIZE, sample_rate);
        dirac.copy_to_channel(&[0., 0., 0., 0., 1., 1.], 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(dirac);
        // duration is between two diracs, only first one should be played
        // as we force slow track with start == 1. / sample_rate as f64
        // the expected dirac will be at index 5 instead of 4
        src.start_at_with_offset_and_duration(
            1. / sample_rate as f64,
            0. / sample_rate as f64,
            4.5 / sample_rate as f64,
        );

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; 128];
        expected[5] = 1.;

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }

    #[test]
    // port from wpt - sub-sample-scheduling.html / sub-sample-grain
    fn test_with_duration_2() {
        let sample_rate = 32_768.;
        let mut context = OfflineAudioContext::new(1, RENDER_QUANTUM_SIZE, sample_rate);

        let mut buffer = context.create_buffer(1, RENDER_QUANTUM_SIZE, sample_rate);
        buffer.copy_to_channel(&[1.; RENDER_QUANTUM_SIZE], 0);

        let start_grain_index = 3.1;
        let end_grain_index = 37.2;

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(buffer);

        src.start_at_with_offset_and_duration(
            start_grain_index / sample_rate as f64,
            0.,
            (end_grain_index - start_grain_index) / sample_rate as f64,
        );

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = [1.; RENDER_QUANTUM_SIZE];
        for s in expected
            .iter_mut()
            .take(start_grain_index.floor() as usize + 1)
        {
            *s = 0.;
        }
        for s in expected
            .iter_mut()
            .take(RENDER_QUANTUM_SIZE)
            .skip(end_grain_index.ceil() as usize)
        {
            *s = 0.;
        }

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }

    #[test]
    fn test_with_offset() {
        // offset always bypass slow track
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, RENDER_QUANTUM_SIZE, sample_rate);

        let mut dirac = context.create_buffer(1, RENDER_QUANTUM_SIZE, sample_rate);
        dirac.copy_to_channel(&[0., 0., 0., 0., 1., 1.], 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(dirac);
        // duration is between two diracs, only first one should be played
        // as we force slow track with start == 1. / sample_rate as f64
        // the expected dirac will be at index 5 instead of 4
        src.start_at_with_offset_and_duration(
            0. / sample_rate as f64,
            1. / sample_rate as f64,
            3.5 / sample_rate as f64,
        );

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; 128];
        expected[3] = 1.;

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }

    #[test]
    fn test_null_buffer_start_ends_before_start_time() {
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, sample_rate as usize, sample_rate);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());

        let ended = Arc::new(AtomicBool::new(false));
        let ended_clone = Arc::clone(&ended);
        src.set_onended(move |_| {
            ended_clone.store(true, Ordering::Relaxed);
        });

        src.start_at(0.75);
        context.suspend_sync(0.5, move |context| {
            assert!(ended.load(Ordering::Relaxed));
            src.set_buffer(context.create_buffer(1, 1, sample_rate));
        });

        let result = context.start_rendering_sync();
        assert_float_eq!(
            result.get_channel_data(0)[..],
            vec![0.; sample_rate as usize][..],
            abs_all <= 0.
        );
    }

    #[test]
    fn test_reverse_playback_with_duration() {
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, RENDER_QUANTUM_SIZE, sample_rate);

        let mut buffer = context.create_buffer(1, 5, sample_rate);
        buffer.copy_to_channel(&[1., 2., 3., 4., 5.], 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(buffer.clone());
        src.playback_rate().set_value(-1.);
        src.start_at_with_offset_and_duration(0., buffer.duration(), 2. / sample_rate as f64);

        let result = context.start_rendering_sync();
        let mut expected = vec![0.; RENDER_QUANTUM_SIZE];
        expected[1] = 5.;

        assert_float_eq!(result.get_channel_data(0)[..], expected[..], abs_all <= 0.);
    }

    #[test]
    fn test_offset_larger_than_buffer_duration() {
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, RENDER_QUANTUM_SIZE, sample_rate);
        let mut buffer = context.create_buffer(1, 13, sample_rate);
        buffer.copy_to_channel(&[1.; 13], 0);

        let mut src = context.create_buffer_source();
        src.set_buffer(buffer);
        src.start_at_with_offset(0., 64. / sample_rate as f64); // offset larger than buffer size

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let expected = [0.; RENDER_QUANTUM_SIZE];
        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }

    #[test]
    fn test_fast_track_loop_mono() {
        let sample_rate = 48_000.;
        let len = RENDER_QUANTUM_SIZE * 4;

        for buffer_len in [
            RENDER_QUANTUM_SIZE / 2 - 1,
            RENDER_QUANTUM_SIZE / 2,
            RENDER_QUANTUM_SIZE / 2 + 1,
            RENDER_QUANTUM_SIZE - 1,
            RENDER_QUANTUM_SIZE,
            RENDER_QUANTUM_SIZE + 1,
            RENDER_QUANTUM_SIZE * 2 - 1,
            RENDER_QUANTUM_SIZE * 2,
            RENDER_QUANTUM_SIZE * 2 + 1,
        ] {
            let mut context = OfflineAudioContext::new(1, len, sample_rate);

            let mut dirac = context.create_buffer(1, buffer_len, sample_rate);
            dirac.copy_to_channel(&[1.], 0);

            let mut src = context.create_buffer_source();
            src.connect(&context.destination());
            src.set_loop(true);
            src.set_buffer(dirac);
            src.start();

            let result = context.start_rendering_sync();
            let channel = result.get_channel_data(0);

            let mut expected = vec![0.; len];
            for i in (0..len).step_by(buffer_len) {
                expected[i] = 1.;
            }

            assert_float_eq!(channel[..], expected[..], abs_all <= 1e-10);
        }
    }

    #[test]
    fn test_slow_track_loop_mono() {
        let sample_rate = 48_000.;
        let len = RENDER_QUANTUM_SIZE * 4;

        for buffer_len in [
            RENDER_QUANTUM_SIZE / 2 - 1,
            RENDER_QUANTUM_SIZE / 2,
            RENDER_QUANTUM_SIZE / 2 + 1,
            RENDER_QUANTUM_SIZE - 1,
            RENDER_QUANTUM_SIZE,
            RENDER_QUANTUM_SIZE + 1,
            RENDER_QUANTUM_SIZE * 2 - 1,
            RENDER_QUANTUM_SIZE * 2,
            RENDER_QUANTUM_SIZE * 2 + 1,
        ] {
            let mut context = OfflineAudioContext::new(1, len, sample_rate);

            let mut dirac = context.create_buffer(1, buffer_len, sample_rate);
            dirac.copy_to_channel(&[1.], 0);

            let mut src = context.create_buffer_source();
            src.connect(&context.destination());
            src.set_loop(true);
            src.set_buffer(dirac);
            src.start_at(1. / sample_rate as f64);

            let result = context.start_rendering_sync();
            let channel = result.get_channel_data(0);

            let mut expected = vec![0.; len];
            for i in (1..len).step_by(buffer_len) {
                expected[i] = 1.;
            }

            assert_float_eq!(channel[..], expected[..], abs_all <= 1e-9);
        }
    }

    #[test]
    fn test_fast_track_loop_stereo() {
        let sample_rate = 48_000.;
        let len = RENDER_QUANTUM_SIZE * 4;

        for buffer_len in [
            RENDER_QUANTUM_SIZE / 2 - 1,
            RENDER_QUANTUM_SIZE / 2,
            RENDER_QUANTUM_SIZE / 2 + 1,
            RENDER_QUANTUM_SIZE - 1,
            RENDER_QUANTUM_SIZE,
            RENDER_QUANTUM_SIZE + 1,
            RENDER_QUANTUM_SIZE * 2 - 1,
            RENDER_QUANTUM_SIZE * 2,
            RENDER_QUANTUM_SIZE * 2 + 1,
        ] {
            let mut context = OfflineAudioContext::new(2, len, sample_rate);
            let mut dirac = context.create_buffer(2, buffer_len, sample_rate);
            dirac.copy_to_channel(&[1.], 0);
            dirac.copy_to_channel(&[0., 1.], 1);

            let mut src = context.create_buffer_source();
            src.connect(&context.destination());
            src.set_loop(true);
            src.set_buffer(dirac);
            src.start();

            let result = context.start_rendering_sync();

            let mut expected_left: Vec<f32> = vec![0.; len];
            let mut expected_right = vec![0.; len];
            for i in (0..len).step_by(buffer_len) {
                expected_left[i] = 1.;

                if i < expected_right.len() - 1 {
                    expected_right[i + 1] = 1.;
                }
            }

            assert_float_eq!(
                result.get_channel_data(0)[..],
                expected_left[..],
                abs_all <= 1e-10
            );
            assert_float_eq!(
                result.get_channel_data(1)[..],
                expected_right[..],
                abs_all <= 1e-10
            );
        }
    }

    #[test]
    fn test_slow_track_loop_stereo() {
        let sample_rate = 48_000.;
        let len = RENDER_QUANTUM_SIZE * 4;

        for buffer_len in [
            RENDER_QUANTUM_SIZE / 2 - 1,
            RENDER_QUANTUM_SIZE / 2,
            RENDER_QUANTUM_SIZE / 2 + 1,
            RENDER_QUANTUM_SIZE - 1,
            RENDER_QUANTUM_SIZE,
            RENDER_QUANTUM_SIZE + 1,
            RENDER_QUANTUM_SIZE * 2 - 1,
            RENDER_QUANTUM_SIZE * 2,
            RENDER_QUANTUM_SIZE * 2 + 1,
        ] {
            let mut context = OfflineAudioContext::new(2, len, sample_rate);
            let mut dirac = context.create_buffer(2, buffer_len, sample_rate);
            dirac.copy_to_channel(&[1.], 0);
            dirac.copy_to_channel(&[0., 1.], 1);

            let mut src = context.create_buffer_source();
            src.connect(&context.destination());
            src.set_loop(true);
            src.set_buffer(dirac);
            src.start_at(1. / sample_rate as f64);

            let result = context.start_rendering_sync();

            let mut expected_left: Vec<f32> = vec![0.; len];
            let mut expected_right = vec![0.; len];
            for i in (1..len).step_by(buffer_len) {
                expected_left[i] = 1.;

                if i < expected_right.len() - 1 {
                    expected_right[i + 1] = 1.;
                }
            }

            assert_float_eq!(
                result.get_channel_data(0)[..],
                expected_left[..],
                abs_all <= 1e-9
            );
            assert_float_eq!(
                result.get_channel_data(1)[..],
                expected_right[..],
                abs_all <= 1e-9
            );
        }
    }

    #[test]
    fn test_reverse_loop_boundaries() {
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, RENDER_QUANTUM_SIZE, sample_rate);

        let mut buffer = context.create_buffer(1, 5, sample_rate);
        buffer.copy_to_channel(&[1., 2., 3., 4., 5.], 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(buffer);
        src.set_loop(true);
        src.set_loop_start(1. / sample_rate as f64);
        src.set_loop_end(4. / sample_rate as f64);
        src.playback_rate().set_value(-1.);
        src.start_at_with_offset(0., 3. / sample_rate as f64);

        let result = context.start_rendering_sync();
        let expected = [4., 3., 2., 4., 3., 2., 4., 3.];
        assert_float_eq!(result.get_channel_data(0)[..8], expected[..], abs_all <= 0.);
    }

    #[test]
    fn test_loop_out_of_bounds() {
        [
            // these will go in fast track
            (-2., -1., 0.),
            (-1., -2., 0.),
            (0., 0., 0.),
            (-1., 2., 0.),
            // these will go in slow track
            (2., -1., 1e-10),
            (1., 1., 1e-10),
            (2., 3., 1e-10),
            (3., 2., 1e-10),
        ]
        .iter()
        .for_each(|(loop_start, loop_end, error)| {
            let sample_rate = 48_000.;
            let length = sample_rate as usize / 10;
            let mut context = OfflineAudioContext::new(1, length, sample_rate);

            let buffer_size = 500;
            let mut buffer = context.create_buffer(1, buffer_size, sample_rate);
            let data = vec![1.; 1];
            buffer.copy_to_channel(&data, 0);

            let mut src = context.create_buffer_source();
            src.connect(&context.destination());
            src.set_buffer(buffer);

            src.set_loop(true);
            src.set_loop_start(*loop_start); // outside of buffer duration
            src.set_loop_end(*loop_end); // outside of buffer duration
            src.start();

            let result = context.start_rendering_sync(); // should terminate
            let channel = result.get_channel_data(0);

            // Both loop points will be clamped to buffer duration due to rules defined at
            // https://webaudio.github.io/web-audio-api/#dom-audiobuffersourcenode-loopstart
            // https://webaudio.github.io/web-audio-api/#dom-audiobuffersourcenode-loopend
            // Thus it violates the rule defined in
            // https://webaudio.github.io/web-audio-api/#playback-AudioBufferSourceNode
            // `loopStart >= 0 && loopEnd > 0 && loopStart < loopEnd`
            // Hence the whole buffer should be looped

            let mut expected = vec![0.; length];
            for i in (0..length).step_by(buffer_size) {
                expected[i] = 1.;
            }

            assert_float_eq!(channel[..], expected[..], abs_all <= error);
        });
    }

    #[test]
    // regression test for #452
    // - duration not set so `self.duration` is `f64::MAX`
    // - stop time is > buffer length
    fn test_end_of_file_fast_track_2() {
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, RENDER_QUANTUM_SIZE, sample_rate);

        let mut buffer = context.create_buffer(1, 5, sample_rate);
        let data = vec![1.; 1];
        buffer.copy_to_channel(&data, 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(buffer);
        // play in fast track
        src.start_at(0.);
        // stop after end of buffer but before the end of render quantum
        src.stop_at(125. / sample_rate as f64);

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; 128];
        expected[0] = 1.;

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }

    #[test]
    // regression test for #452
    // - duration not set so `self.duration` is `f64::MAX`
    // - stop time is > buffer length
    fn test_end_of_file_slow_track_2() {
        let sample_rate = 48_000.;
        let mut context = OfflineAudioContext::new(1, RENDER_QUANTUM_SIZE, sample_rate);

        let mut buffer = context.create_buffer(1, 5, sample_rate);
        let data = vec![1.; 1];
        buffer.copy_to_channel(&data, 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(buffer);
        // play in fast track
        src.start_at(1. / sample_rate as f64);
        // stop after end of buffer but before the end of render quantum
        src.stop_at(125. / sample_rate as f64);

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; 128];
        expected[1] = 1.;

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }

    #[test]
    fn test_loop_no_restart_suspend() {
        let sample_rate = 48_000.;
        let result_size = RENDER_QUANTUM_SIZE * 2;
        let mut context = OfflineAudioContext::new(1, result_size, sample_rate);

        let mut buffer = context.create_buffer(1, 1, sample_rate);
        let data = vec![1.; 1];
        buffer.copy_to_channel(&data, 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(buffer);
        src.start_at(0.);

        context.suspend_sync(RENDER_QUANTUM_SIZE as f64 / sample_rate as f64, move |_| {
            src.set_loop(true);
        });

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; result_size];
        expected[0] = 1.;

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }

    #[test]
    fn test_loop_no_restart_onended_fast_track() {
        let sample_rate = 48_000.;
        // ended event is send on second render quantum, let's take a few more to be sure
        let result_size = RENDER_QUANTUM_SIZE * 4;
        let mut context = OfflineAudioContext::new(1, result_size, sample_rate);

        let mut buffer = context.create_buffer(1, 1, sample_rate);
        let data = vec![1.; 1];
        buffer.copy_to_channel(&data, 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(buffer);
        // play in fast track
        src.start_at(0.);

        let src = Arc::new(Mutex::new(src));
        let clone = Arc::clone(&src);
        src.lock().unwrap().set_onended(move |_| {
            clone.lock().unwrap().set_loop(true);
        });

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; result_size];
        expected[0] = 1.;

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }

    #[test]
    fn test_loop_no_restart_onended_slow_track() {
        let sample_rate = 48_000.;
        // ended event is send on second render quantum, let's take a few more to be sure
        let result_size = RENDER_QUANTUM_SIZE * 4;
        let mut context = OfflineAudioContext::new(1, result_size, sample_rate);

        let mut buffer = context.create_buffer(1, 1, sample_rate);
        let data = vec![1.; 1];
        buffer.copy_to_channel(&data, 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(buffer);
        // play in slow track
        src.start_at(1. / sample_rate as f64);

        let src = Arc::new(Mutex::new(src));
        let clone = Arc::clone(&src);
        src.lock().unwrap().set_onended(move |_| {
            clone.lock().unwrap().set_loop(true);
        });

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; result_size];
        expected[1] = 1.;

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
    }

    #[test]
    // Ported from wpt: the-audiobuffersourcenode-interface/sub-sample-buffer-stitching.html
    // Note that in wpt, results are tested against an oscillator node, which fails
    // in the (44_100., 43_800., 3.8986e-3) condition for some (yet) unknown reason
    fn test_subsample_buffer_stitching() {
        [(44_100., 44_100., 9.0957e-5), (44_100., 43_800., 3.8986e-3)]
            .iter()
            .for_each(|(sample_rate, buffer_rate, error_threshold)| {
                let sample_rate = *sample_rate;
                let buffer_rate = *buffer_rate;
                let buffer_length = 30;
                let frequency = 440.;

                // let length = sample_rate as usize;
                let length = buffer_length * 15;
                let mut context = OfflineAudioContext::new(2, length, sample_rate);

                let mut wave_signal = vec![0.; context.length()];
                let omega = 2. * PI / buffer_rate * frequency;
                wave_signal.iter_mut().enumerate().for_each(|(i, s)| {
                    *s = (omega * i as f32).sin();
                });

                // Slice the sine wave into many little buffers to be assigned to ABSNs
                // that are started at the appropriate times to produce a final sine
                // wave.
                for k in (0..context.length()).step_by(buffer_length) {
                    let mut buffer = AudioBuffer::new(AudioBufferOptions {
                        number_of_channels: 1,
                        length: buffer_length,
                        sample_rate: buffer_rate,
                    });
                    buffer.copy_to_channel(&wave_signal[k..k + buffer_length], 0);

                    let mut src = AudioBufferSourceNode::new(
                        &context,
                        AudioBufferSourceOptions {
                            buffer: Some(buffer),
                            ..Default::default()
                        },
                    );
                    src.connect(&context.destination());
                    src.start_at(k as f64 / buffer_rate as f64);
                }

                let mut expected = vec![0.; context.length()];
                let omega = 2. * PI / sample_rate * frequency;
                expected.iter_mut().enumerate().for_each(|(i, s)| {
                    *s = (omega * i as f32).sin();
                });

                let result = context.start_rendering_sync();
                let actual = result.get_channel_data(0);

                assert_float_eq!(actual[..], expected[..], abs_all <= error_threshold);
            });
    }

    #[test]
    fn test_onended_before_drop() {
        let sample_rate = 48_000.;
        let result_size = RENDER_QUANTUM_SIZE;
        let mut context = OfflineAudioContext::new(1, result_size, sample_rate);
        // buffer is larger than context output so it never goes into the ended check condition
        let mut buffer = context.create_buffer(1, result_size * 2, sample_rate);
        let data = vec![1.; 1];
        buffer.copy_to_channel(&data, 0);

        let mut src = context.create_buffer_source();
        src.connect(&context.destination());
        src.set_buffer(buffer);
        src.start();

        let onended_called = Arc::new(AtomicBool::new(false));
        let onended_called_clone = Arc::clone(&onended_called);

        src.set_onended(move |_| {
            onended_called_clone.store(true, Ordering::SeqCst);
        });

        let result = context.start_rendering_sync();
        let channel = result.get_channel_data(0);

        let mut expected = vec![0.; result_size];
        expected[0] = 1.;

        assert_float_eq!(channel[..], expected[..], abs_all <= 0.);
        assert!(onended_called.load(Ordering::SeqCst));
    }
}
