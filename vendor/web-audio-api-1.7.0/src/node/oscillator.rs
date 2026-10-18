use std::any::Any;
use std::f32::consts::PI;
use std::fmt::Debug;
use std::sync::OnceLock;

use crate::context::{AudioContextRegistration, AudioParamId, BaseAudioContext};
use crate::param::{AudioParam, AudioParamDescriptor, AutomationRate};
use crate::render::{
    AudioParamValues, AudioProcessor, AudioRenderQuantum, AudioWorkletGlobalScope,
};
use crate::PeriodicWave;
use crate::{assert_valid_time_value, RENDER_QUANTUM_SIZE};

use super::{AudioNode, AudioNodeOptions, AudioScheduledSourceNode, ChannelConfig};

const SINE_TABLE_LENGTH_USIZE: usize = 2048;
const SINE_TABLE_LENGTH_F32: f32 = SINE_TABLE_LENGTH_USIZE as f32;

/// Precomputed sine table
fn precomputed_sine_table() -> &'static [f32] {
    static INSTANCE: OnceLock<Vec<f32>> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        // Compute one period sine wavetable of size SINE_TABLE_LENGTH.
        (0..SINE_TABLE_LENGTH_USIZE)
            .map(|x| ((x as f32) * 2.0 * PI * (1. / (SINE_TABLE_LENGTH_F32))).sin())
            .collect()
    })
}

fn get_computed_freq(freq: f32, detune: f32) -> f64 {
    freq as f64 * (detune as f64 / 1200.).exp2()
}

/// Options for constructing an [`OscillatorNode`]
// dictionary OscillatorOptions : AudioNodeOptions {
//   OscillatorType type = "sine";
//   float frequency = 440;
//   float detune = 0;
//   PeriodicWave periodicWave;
// };
//
// @note - Does extend AudioNodeOptions but they are useless for source nodes as
// they instruct how to upmix the inputs.
// This is a common source of confusion, see e.g. https://github.com/mdn/content/pull/18472, and
// an issue in the spec, see discussion in https://github.com/WebAudio/web-audio-api/issues/2496
#[derive(Clone, Debug)]
pub struct OscillatorOptions {
    /// The shape of the periodic waveform
    pub type_: OscillatorType,
    /// The frequency of the fundamental frequency.
    pub frequency: f32,
    /// A detuning value (in cents) which will offset the frequency by the given amount.
    pub detune: f32,
    /// Optional custom waveform, if specified (set `type` to "custom")
    pub periodic_wave: Option<PeriodicWave>,
    /// channel config options
    pub audio_node_options: AudioNodeOptions,
}

impl Default for OscillatorOptions {
    fn default() -> Self {
        Self {
            type_: OscillatorType::default(),
            frequency: 440.,
            detune: 0.,
            periodic_wave: None,
            audio_node_options: AudioNodeOptions::default(),
        }
    }
}

/// Type of the waveform rendered by an `OscillatorNode`
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum OscillatorType {
    /// Sine wave
    #[default]
    Sine,
    /// Square wave
    Square,
    /// Sawtooth wave
    Sawtooth,
    /// Triangle wave
    Triangle,
    /// type used when periodic_wave is specified
    Custom,
}

impl From<u32> for OscillatorType {
    fn from(i: u32) -> Self {
        match i {
            0 => OscillatorType::Sine,
            1 => OscillatorType::Square,
            2 => OscillatorType::Sawtooth,
            3 => OscillatorType::Triangle,
            4 => OscillatorType::Custom,
            _ => unreachable!(),
        }
    }
}

/// Instructions to start or stop processing
#[derive(Debug, Copy, Clone)]
enum Schedule {
    Start(f64),
    Stop(f64),
}

/// `OscillatorNode` represents an audio source generating a periodic waveform.
/// It can generate a few common waveforms (i.e. sine, square, sawtooth, triangle),
/// or can be set to an arbitrary periodic waveform using a [`PeriodicWave`] object.
///
/// - MDN documentation: <https://developer.mozilla.org/en-US/docs/Web/API/OscillatorNode>
/// - specification: <https://webaudio.github.io/web-audio-api/#OscillatorNode>
/// - see also: [`BaseAudioContext::create_oscillator`]
/// - see also: [`PeriodicWave`]
///
/// # Usage
///
/// ```no_run
/// use web_audio_api::context::{BaseAudioContext, AudioContext};
/// use web_audio_api::node::{AudioNode, AudioScheduledSourceNode};
///
/// let context = AudioContext::default();
///
/// let mut osc = context.create_oscillator();
/// osc.frequency().set_value(200.);
/// osc.connect(&context.destination());
/// osc.start();
/// ```
///
/// # Examples
///
/// - `cargo run --release --example oscillators`
/// - `cargo run --release --example many_oscillators_with_env`
/// - `cargo run --release --example amplitude_modulation`
///
#[derive(Debug)]
pub struct OscillatorNode {
    /// Represents the node instance and its associated audio context
    registration: AudioContextRegistration,
    /// Infos about audio node channel configuration
    channel_config: ChannelConfig,
    /// The frequency of the fundamental frequency.
    frequency: AudioParam,
    /// A detuning value (in cents) which will offset the frequency by the given amount.
    detune: AudioParam,
    /// Waveform of an oscillator
    type_: OscillatorType,
    /// Tracks whether `start` has been called already.
    has_start: bool,
}

impl AudioNode for OscillatorNode {
    fn registration(&self) -> &AudioContextRegistration {
        &self.registration
    }

    fn channel_config(&self) -> &ChannelConfig {
        &self.channel_config
    }

    /// `OscillatorNode` is a source node. A source node is by definition with no input
    fn number_of_inputs(&self) -> usize {
        0
    }

    /// `OscillatorNode` is a mono source node.
    fn number_of_outputs(&self) -> usize {
        1
    }
}

impl AudioScheduledSourceNode for OscillatorNode {
    fn start(&mut self) {
        let when = self.registration.context().current_time();
        self.start_at(when);
    }

    fn start_at(&mut self, when: f64) {
        assert_valid_time_value(when);
        assert!(
            !self.has_start,
            "InvalidStateError - Cannot call `start` twice"
        );

        self.has_start = true;
        self.registration.post_message(Schedule::Start(when));
    }

    fn stop(&mut self) {
        let when = self.registration.context().current_time();
        self.stop_at(when);
    }

    fn stop_at(&mut self, when: f64) {
        assert_valid_time_value(when);
        assert!(
            self.has_start,
            "InvalidStateError - cannot stop before start"
        );

        self.registration.post_message(Schedule::Stop(when));
    }
}

impl OscillatorNode {
    /// Returns an `OscillatorNode`
    ///
    /// # Arguments:
    ///
    /// * `context` - The `AudioContext`
    /// * `options` - The OscillatorOptions
    pub fn new<C: BaseAudioContext>(context: &C, options: OscillatorOptions) -> Self {
        let OscillatorOptions {
            type_,
            frequency,
            detune,
            audio_node_options: channel_config,
            periodic_wave,
        } = options;

        let mut node = context.base().register(move |registration| {
            let sample_rate = context.sample_rate();
            let nyquist = sample_rate / 2.;

            // frequency audio parameter
            let freq_param_options = AudioParamDescriptor {
                name: String::new(),
                min_value: -nyquist,
                max_value: nyquist,
                default_value: 440.,
                automation_rate: AutomationRate::A,
            };
            let (f_param, f_proc) = context.create_audio_param(freq_param_options, &registration);
            f_param.set_value(frequency);

            // detune audio parameter
            let det_param_options = AudioParamDescriptor {
                name: String::new(),
                min_value: -153_600.,
                max_value: 153_600.,
                default_value: 0.,
                automation_rate: AutomationRate::A,
            };
            let (det_param, det_proc) =
                context.create_audio_param(det_param_options, &registration);
            det_param.set_value(detune);

            let renderer = OscillatorRenderer {
                type_,
                frequency: f_proc,
                detune: det_proc,
                phase: 0.,
                start_time: f64::MAX,
                stop_time: f64::MAX,
                started: false,
                periodic_wave: None,
                ended_triggered: false,
                sine_table: precomputed_sine_table(),
            };

            let node = Self {
                registration,
                channel_config: channel_config.into(),
                frequency: f_param,
                detune: det_param,
                type_,
                has_start: false,
            };

            (node, Box::new(renderer))
        });

        // renderer has been sent to render thread, we can send it messages
        if let Some(p_wave) = periodic_wave {
            node.set_periodic_wave(p_wave);
        }

        node
    }

    /// A-rate [`AudioParam`] that defines the fundamental frequency of the
    /// oscillator, expressed in Hz
    ///
    /// The final frequency is calculated as follow: frequency * 2^(detune/1200)
    #[must_use]
    pub fn frequency(&self) -> &AudioParam {
        &self.frequency
    }

    /// A-rate [`AudioParam`] that defines a transposition according to the
    /// frequency, expressed in cents.
    ///
    /// see <https://en.wikipedia.org/wiki/Cent_(music)>
    ///
    /// The final frequency is calculated as follow: frequency * 2^(detune/1200)
    #[must_use]
    pub fn detune(&self) -> &AudioParam {
        &self.detune
    }

    /// Returns the oscillator type
    #[must_use]
    pub fn type_(&self) -> OscillatorType {
        self.type_
    }

    /// Set the oscillator type
    ///
    /// # Arguments
    ///
    /// * `type_` - oscillator type (sine, square, triangle, sawtooth)
    ///
    /// # Panics
    ///
    /// if `type_` is `OscillatorType::Custom`
    pub fn set_type(&mut self, type_: OscillatorType) {
        assert_ne!(
            type_,
            OscillatorType::Custom,
            "InvalidStateError: Custom type cannot be set manually"
        );

        // if periodic wave has been set specified, type_ changes are ignored
        if self.type_ == OscillatorType::Custom {
            return;
        }

        self.type_ = type_;
        self.registration.post_message(type_);
    }

    /// Sets a `PeriodicWave` which describes a waveform to be used by the oscillator.
    ///
    /// Calling this sets the oscillator type to `custom`, once set to `custom`
    /// the oscillator cannot be reverted back to a standard waveform.
    pub fn set_periodic_wave(&mut self, periodic_wave: PeriodicWave) {
        self.type_ = OscillatorType::Custom;
        self.registration.post_message(periodic_wave);
    }
}

/// Rendering component of the oscillator node
struct OscillatorRenderer {
    /// The shape of the periodic waveform
    type_: OscillatorType,
    /// The frequency of the fundamental frequency.
    frequency: AudioParamId,
    /// A detuning value (in cents) which will offset the frequency by the given amount.
    detune: AudioParamId,
    /// current phase of the oscillator
    phase: f64,
    /// start time
    start_time: f64,
    /// end time
    stop_time: f64,
    /// defines if the oscillator has started
    started: bool,
    /// wavetable placeholder for custom oscillators
    periodic_wave: Option<PeriodicWave>,
    /// defines if the `ended` events was already dispatched
    ended_triggered: bool,
    /// Precomputed sine table
    sine_table: &'static [f32],
}

impl AudioProcessor for OscillatorRenderer {
    fn process(
        &mut self,
        _inputs: &[AudioRenderQuantum],
        outputs: &mut [AudioRenderQuantum],
        params: AudioParamValues<'_>,
        scope: &AudioWorkletGlobalScope,
    ) -> bool {
        // single output node
        let output = &mut outputs[0];
        // 1 channel output
        output.set_number_of_channels(1);

        let sample_rate = scope.sample_rate as f64;
        let dt = 1. / sample_rate;
        let num_frames = RENDER_QUANTUM_SIZE;
        let next_block_time = scope.current_time + dt * num_frames as f64;

        if self.stop_time <= scope.current_time {
            output.make_silent();

            if !self.ended_triggered {
                scope.send_ended_event();
                self.ended_triggered = true;
            }

            return false;
        } else if self.start_time >= next_block_time {
            output.make_silent();

            if self.stop_time <= next_block_time {
                if !self.ended_triggered {
                    scope.send_ended_event();
                    self.ended_triggered = true;
                }

                return false;
            }

            // #462 AudioScheduledSourceNodes that have not been scheduled to start can safely
            // return tail_time false in order to be collected if their control handle drops.
            return self.start_time != f64::MAX;
        }

        let channel_data = output.channel_data_mut(0);
        let frequency_values = params.get(&self.frequency);
        let detune_values = params.get(&self.detune);

        let mut current_time = scope.current_time;

        // Prevent scheduling in the past
        //
        // [spec] If 0 is passed in for this value or if the value is less than
        // currentTime, then the sound will start playing immediately
        // cf. https://webaudio.github.io/web-audio-api/#dom-audioscheduledsourcenode-start-when-when
        if !self.started && self.start_time < current_time {
            self.start_time = current_time;
        }

        let nyquist = sample_rate / 2.;

        // fast path for scalar AudioParam values
        if frequency_values.len() == 1 && detune_values.len() == 1 {
            let freq = frequency_values[0];
            let detune = detune_values[0];
            let computed_freq = get_computed_freq(freq, detune);
            let phase_incr = computed_freq / sample_rate;
            let outside_nyquist = computed_freq.abs() >= nyquist;
            let fully_active = self.started
                && self.start_time <= scope.current_time
                && self.stop_time >= next_block_time;

            if fully_active && !outside_nyquist {
                channel_data.iter_mut().for_each(|output| {
                    *output = self.generate_waveform_sample(phase_incr);
                    self.phase = Self::unroll_phase(self.phase + phase_incr);
                });
            } else {
                channel_data.iter_mut().for_each(|output| {
                    current_time =
                        self.generate_sample(output, outside_nyquist, phase_incr, current_time, dt);
                });
            }
        } else {
            channel_data
                .iter_mut()
                .zip(frequency_values.iter().cycle())
                .zip(detune_values.iter().cycle())
                .for_each(|((output, &freq), &detune)| {
                    let computed_freq = get_computed_freq(freq, detune);
                    let phase_incr = computed_freq / sample_rate;
                    let outside_nyquist = computed_freq.abs() >= nyquist;
                    current_time =
                        self.generate_sample(output, outside_nyquist, phase_incr, current_time, dt)
                });
        }

        if self.stop_time <= next_block_time {
            if !self.ended_triggered {
                scope.send_ended_event();
                self.ended_triggered = true;
            }

            return false;
        }

        true
    }

    fn onmessage(&mut self, msg: &mut dyn Any) {
        if let Some(&type_) = msg.downcast_ref::<OscillatorType>() {
            self.type_ = type_;
            return;
        }

        if let Some(&schedule) = msg.downcast_ref::<Schedule>() {
            match schedule {
                Schedule::Start(v) => self.start_time = v,
                Schedule::Stop(v) => self.stop_time = v,
            }
            return;
        }

        if let Some(periodic_wave) = msg.downcast_mut::<PeriodicWave>() {
            if let Some(current_periodic_wave) = &mut self.periodic_wave {
                // Avoid deallocation in the render thread by swapping the wavetable buffers.
                std::mem::swap(current_periodic_wave, periodic_wave)
            } else {
                // The default wavetable buffer is empty and does not cause allocations.
                self.periodic_wave = Some(std::mem::take(periodic_wave));
            }
            self.type_ = OscillatorType::Custom; // shared type is already updated by control
            return;
        }

        log::warn!("OscillatorRenderer: Dropping incoming message {msg:?}");
    }

    fn before_drop(&mut self, scope: &AudioWorkletGlobalScope) {
        if !self.ended_triggered
            && (scope.current_time >= self.start_time || scope.current_time >= self.stop_time)
        {
            scope.send_ended_event();
            self.ended_triggered = true;
        }
    }
}
impl OscillatorRenderer {
    #[inline]
    fn generate_sample(
        &mut self,
        output: &mut f32,
        outside_nyquist: bool,
        phase_incr: f64,
        current_time: f64,
        dt: f64,
    ) -> f64 {
        if current_time < self.start_time || current_time >= self.stop_time {
            *output = 0.;
            return current_time + dt;
        }

        // first sample to render
        if !self.started {
            // if start time was between last frame and current frame
            // we need to adjust the phase first
            if current_time > self.start_time {
                let ratio = (current_time - self.start_time) / dt;
                self.phase = if outside_nyquist {
                    Self::unroll_phase_unbounded(phase_incr * ratio)
                } else {
                    Self::unroll_phase(phase_incr * ratio)
                };
            }

            self.started = true;
        }

        *output = if outside_nyquist {
            // Output silence when the computed oscillator frequency is outside the
            // nominal [-nyquist, nyquist] range. Timing and phase still advance so
            // automation can re-enter the audible range without resetting phase.
            0.
        } else {
            self.generate_waveform_sample(phase_incr)
        };

        self.phase = if outside_nyquist {
            Self::unroll_phase_unbounded(self.phase + phase_incr)
        } else {
            Self::unroll_phase(self.phase + phase_incr)
        };

        current_time + dt
    }

    #[inline]
    fn generate_waveform_sample(&mut self, phase_incr: f64) -> f32 {
        match self.type_ {
            OscillatorType::Sine => self.generate_sine(),
            OscillatorType::Sawtooth => self.generate_sawtooth(phase_incr),
            OscillatorType::Square => self.generate_square(phase_incr),
            OscillatorType::Triangle => self.generate_triangle(),
            OscillatorType::Custom => self.generate_custom(),
        }
    }

    #[inline]
    fn generate_sine(&mut self) -> f32 {
        let position = self.phase * SINE_TABLE_LENGTH_USIZE as f64;
        let floored = position.floor();

        let prev_index = floored as usize;
        let mut next_index = prev_index + 1;
        if next_index == SINE_TABLE_LENGTH_USIZE {
            next_index = 0;
        }

        // linear interpolation into lookup table
        let k = (position - floored) as f32;
        self.sine_table[prev_index].mul_add(1. - k, self.sine_table[next_index] * k)
    }

    #[inline]
    fn generate_sawtooth(&mut self, phase_incr: f64) -> f32 {
        // offset phase to start at 0. (not -1.)
        let phase = Self::unroll_phase(self.phase + 0.5);
        let mut sample = 2.0 * phase - 1.0;
        sample -= Self::poly_blep(phase, phase_incr, cfg!(test));

        sample as f32
    }

    #[inline]
    fn generate_square(&mut self, phase_incr: f64) -> f32 {
        let mut sample = if self.phase < 0.5 { 1.0 } else { -1.0 };
        sample += Self::poly_blep(self.phase, phase_incr, cfg!(test));

        let shift_phase = Self::unroll_phase(self.phase + 0.5);
        sample -= Self::poly_blep(shift_phase, phase_incr, cfg!(test));

        sample as f32
    }

    #[inline]
    fn generate_triangle(&mut self) -> f32 {
        let mut sample = -4. * self.phase + 2.;

        if sample > 1. {
            sample = 2. - sample;
        } else if sample < -1. {
            sample = -2. - sample;
        }

        sample as f32
    }

    #[inline]
    fn generate_custom(&mut self) -> f32 {
        let periodic_wave = self.periodic_wave.as_ref().unwrap().as_slice();
        let table_length = periodic_wave.len();
        let position = self.phase * table_length as f64;
        let floored = position.floor();

        let prev_index = floored as usize;
        let mut next_index = prev_index + 1;
        if next_index == table_length {
            next_index = 0;
        }

        // linear interpolation into lookup table
        let k = (position - floored) as f32;
        periodic_wave[prev_index].mul_add(1. - k, periodic_wave[next_index] * k)
    }

    // computes the `polyBLEP` corrections to apply to aliasing signal
    // `polyBLEP` stands for `polyBandLimitedstEP`
    // This basically soften the sharp edges in square and sawtooth signals
    // to avoid infinite frequencies impulses (jumps from -1 to 1 or inverse).
    // cf. http://www.martin-finke.de/blog/articles/audio-plugins-018-polyblep-oscillator/
    //
    // @note: do not apply in tests so we can avoid relying on snapshots
    #[inline]
    fn poly_blep(mut t: f64, dt: f64, is_test: bool) -> f64 {
        if is_test {
            0.
        } else if t < dt {
            t /= dt;
            t + t - t * t - 1.0
        } else if t > 1.0 - dt {
            t = (t - 1.0) / dt;
            t.mul_add(t, t) + t + 1.0
        } else {
            0.0
        }
    }

    #[inline]
    fn unroll_phase(phase: f64) -> f64 {
        if phase >= 1. {
            phase - 1.
        } else if phase < 0. {
            phase + 1.
        } else {
            phase
        }
    }

    #[inline]
    fn unroll_phase_unbounded(phase: f64) -> f64 {
        phase.rem_euclid(1.)
    }
}

#[cfg(test)]
mod tests {
    use float_eq::assert_float_eq;
    use std::f64::consts::PI;

    use crate::context::{BaseAudioContext, OfflineAudioContext};
    use crate::node::{AudioNode, AudioScheduledSourceNode};
    use crate::periodic_wave::{PeriodicWave, PeriodicWaveOptions};
    use crate::RENDER_QUANTUM_SIZE;

    use super::{OscillatorNode, OscillatorOptions, OscillatorRenderer, OscillatorType};

    #[test]
    fn assert_osc_default_build_with_factory_func() {
        let default_freq = 440.;
        let default_det = 0.;
        let default_type = OscillatorType::Sine;

        let mut context = OfflineAudioContext::new(2, 1, 44_100.);

        let mut osc = context.create_oscillator();

        let freq = osc.frequency.value();
        assert_float_eq!(freq, default_freq, abs_all <= 0.);

        let det = osc.detune.value();
        assert_float_eq!(det, default_det, abs_all <= 0.);

        assert_eq!(osc.type_(), default_type);

        // should not panic when run
        osc.start();
        osc.connect(&context.destination());
        let _ = context.start_rendering_sync();
    }

    #[test]
    fn assert_osc_default_build() {
        let default_freq = 440.;
        let default_det = 0.;
        let default_type = OscillatorType::Sine;

        let mut context = OfflineAudioContext::new(2, 1, 44_100.);

        let mut osc = OscillatorNode::new(&context, OscillatorOptions::default());

        let freq = osc.frequency.value();
        assert_float_eq!(freq, default_freq, abs_all <= 0.);

        let det = osc.detune.value();
        assert_float_eq!(det, default_det, abs_all <= 0.);

        assert_eq!(osc.type_(), default_type);

        // should not panic when run
        osc.start();
        osc.connect(&context.destination());
        let _ = context.start_rendering_sync();
    }

    #[test]
    #[should_panic]
    fn set_type_to_custom_should_panic() {
        let context = OfflineAudioContext::new(2, 1, 44_100.);
        let mut osc = OscillatorNode::new(&context, OscillatorOptions::default());
        osc.set_type(OscillatorType::Custom);
    }

    #[test]
    fn type_is_custom_when_periodic_wave_is_some() {
        let expected_type = OscillatorType::Custom;

        let mut context = OfflineAudioContext::new(2, 1, 44_100.);

        let periodic_wave = PeriodicWave::new(&context, PeriodicWaveOptions::default());

        let options = OscillatorOptions {
            periodic_wave: Some(periodic_wave),
            ..OscillatorOptions::default()
        };

        let mut osc = OscillatorNode::new(&context, options);

        assert_eq!(osc.type_(), expected_type);

        // should not panic when run
        osc.start();
        osc.connect(&context.destination());
        let _ = context.start_rendering_sync();
    }

    #[test]
    fn set_type_is_ignored_when_periodic_wave_is_some() {
        let expected_type = OscillatorType::Custom;

        let mut context = OfflineAudioContext::new(2, 1, 44_100.);

        let periodic_wave = PeriodicWave::new(&context, PeriodicWaveOptions::default());

        let options = OscillatorOptions {
            periodic_wave: Some(periodic_wave),
            ..OscillatorOptions::default()
        };

        let mut osc = OscillatorNode::new(&context, options);

        osc.set_type(OscillatorType::Sine);
        assert_eq!(osc.type_(), expected_type);

        // should not panic when run
        osc.start();
        osc.connect(&context.destination());
        let _ = context.start_rendering_sync();
    }

    // # Test waveforms
    //
    // - for `square`, `triangle` and `sawtooth` the tests may appear a bit
    //   tautological (and they actually are) as the code from the test is the
    //   mostly as same as in the renderer, just written in a more compact way.
    //   However they should help to prevent regressions, and/or allow testing
    //   against trusted and simple implementation in case of future changes
    //   in the renderer impl, e.g. performance improvements or spec compliance:
    //   https://webaudio.github.io/web-audio-api/#oscillator-coefficients.
    //
    // - PolyBlep is not applied on `square` and `triangle` for tests, so we can
    //   compare according to a crude waveforms

    #[test]
    fn sine_raw() {
        // 1, 10, 100, 1_000, 10_000 Hz
        for i in 0..5 {
            let freq = 10_f32.powf(i as f32);
            let sample_rate = 44_100;

            let mut context = OfflineAudioContext::new(1, sample_rate, sample_rate as f32);

            let mut osc = context.create_oscillator();
            osc.connect(&context.destination());
            osc.frequency().set_value(freq);
            osc.start_at(0.);

            let output = context.start_rendering_sync();
            let result = output.get_channel_data(0);

            let mut expected = Vec::<f32>::with_capacity(sample_rate);
            let mut phase: f64 = 0.;
            let phase_incr = freq as f64 / sample_rate as f64;

            for _i in 0..sample_rate {
                let sample = (phase * 2. * PI).sin();

                expected.push(sample as f32);

                phase += phase_incr;
                if phase >= 1. {
                    phase -= 1.;
                }
            }

            assert_float_eq!(result[..], expected[..], abs_all <= 1e-5);
        }
    }

    #[test]
    fn sine_raw_exact_phase() {
        // 1, 10, 100, 1_000, 10_000 Hz
        for i in 0..5 {
            let freq = 10_f32.powf(i as f32);
            let sample_rate = 44_100;

            let mut context = OfflineAudioContext::new(1, sample_rate, sample_rate as f32);

            let mut osc = context.create_oscillator();
            osc.connect(&context.destination());
            osc.frequency().set_value(freq);
            osc.start_at(0.);

            let output = context.start_rendering_sync();
            let result = output.get_channel_data(0);
            let mut expected = Vec::<f32>::with_capacity(sample_rate);

            for i in 0..sample_rate {
                let phase = freq as f64 * i as f64 / sample_rate as f64;
                let sample = (phase * 2. * PI).sin();
                // phase += phase_incr;
                expected.push(sample as f32);
            }

            assert_float_eq!(result[..], expected[..], abs_all <= 1e-5);
        }
    }

    #[test]
    fn square_raw() {
        // 1, 10, 100, 1_000, 10_000 Hz
        for i in 0..5 {
            let freq = 10_f32.powf(i as f32);
            let sample_rate = 44100;

            let mut context = OfflineAudioContext::new(1, sample_rate, sample_rate as f32);

            let mut osc = context.create_oscillator();
            osc.connect(&context.destination());
            osc.frequency().set_value(freq);
            osc.set_type(OscillatorType::Square);
            osc.start_at(0.);

            let output = context.start_rendering_sync();
            let result = output.get_channel_data(0);

            let mut expected = Vec::<f32>::with_capacity(sample_rate);
            let mut phase: f64 = 0.;
            let phase_incr = freq as f64 / sample_rate as f64;

            for _i in 0..sample_rate {
                // 0.5 belongs to the second half of the waveform
                let sample = if phase < 0.5 { 1. } else { -1. };

                expected.push(sample as f32);

                phase += phase_incr;
                if phase >= 1. {
                    phase -= 1.;
                }
            }

            assert_float_eq!(result[..], expected[..], abs_all <= 1e-10);
        }
    }

    #[test]
    fn triangle_raw() {
        // 1, 10, 100, 1_000, 10_000 Hz
        for i in 0..5 {
            let freq = 10_f32.powf(i as f32);
            let sample_rate = 44_100;

            let mut context = OfflineAudioContext::new(1, sample_rate, sample_rate as f32);

            let mut osc = context.create_oscillator();
            osc.connect(&context.destination());
            osc.frequency().set_value(freq);
            osc.set_type(OscillatorType::Triangle);
            osc.start_at(0.);

            let output = context.start_rendering_sync();
            let result = output.get_channel_data(0);

            let mut expected = Vec::<f32>::with_capacity(sample_rate);
            let mut phase: f64 = 0.;
            let phase_incr = freq as f64 / sample_rate as f64;

            for _i in 0..sample_rate {
                // triangle starts a 0.
                // [0., 1.]  between [0, 0.25]
                // [1., -1.] between [0.25, 0.75]
                // [-1., 0.] between [0.75, 1]
                let mut sample = -4. * phase + 2.;

                if sample > 1. {
                    sample = 2. - sample;
                } else if sample < -1. {
                    sample = -2. - sample;
                }

                expected.push(sample as f32);

                phase += phase_incr;
                if phase >= 1. {
                    phase -= 1.;
                }
            }

            assert_float_eq!(result[..], expected[..], abs_all <= 1e-10);
        }
    }

    #[test]
    fn sawtooth_raw() {
        // 1, 10, 100, 1_000, 10_000 Hz
        for i in 0..5 {
            let freq = 10_f32.powf(i as f32);
            let sample_rate = 44_100;

            let mut context = OfflineAudioContext::new(1, sample_rate, sample_rate as f32);

            let mut osc = context.create_oscillator();
            osc.connect(&context.destination());
            osc.frequency().set_value(freq);
            osc.set_type(OscillatorType::Sawtooth);
            osc.start_at(0.);

            let output = context.start_rendering_sync();
            let result = output.get_channel_data(0);

            let mut expected = Vec::<f32>::with_capacity(sample_rate);
            let mut phase: f64 = 0.;
            let phase_incr = freq as f64 / sample_rate as f64;

            for _i in 0..sample_rate {
                // triangle starts a 0.
                // [0, 1] between [0, 0.5]
                // [-1, 0] between [0.5, 1]
                let mut offset_phase = phase + 0.5;
                if offset_phase >= 1. {
                    offset_phase -= 1.;
                }
                let sample = 2. * offset_phase - 1.;

                expected.push(sample as f32);

                phase += phase_incr;
                if phase >= 1. {
                    phase -= 1.;
                }
            }

            assert_float_eq!(result[..], expected[..], abs_all <= 1e-10);
        }
    }

    #[test]
    // this one should output exactly the same thing as sine_raw
    fn periodic_wave_1f() {
        // 1, 10, 100, 1_000, 10_000 Hz
        for i in 0..5 {
            let freq = 10_f32.powf(i as f32);
            let sample_rate = 44_100;

            let mut context = OfflineAudioContext::new(1, sample_rate, sample_rate as f32);

            let options = PeriodicWaveOptions {
                real: Some(vec![0., 0.]),
                imag: Some(vec![0., 1.]), // sine is in imaginary component
                disable_normalization: false,
            };

            let periodic_wave = context.create_periodic_wave(options);

            let mut osc = context.create_oscillator();
            osc.connect(&context.destination());
            osc.set_periodic_wave(periodic_wave);
            osc.frequency().set_value(freq);
            osc.set_type(OscillatorType::Sawtooth);
            osc.start_at(0.);

            let output = context.start_rendering_sync();
            let result = output.get_channel_data(0);

            let mut expected = Vec::<f32>::with_capacity(sample_rate);
            let mut phase: f64 = 0.;
            let phase_incr = freq as f64 / sample_rate as f64;

            for _i in 0..sample_rate {
                let sample = (phase * 2. * PI).sin();

                expected.push(sample as f32);

                phase += phase_incr;
                if phase >= 1. {
                    phase -= 1.;
                }
            }

            assert_float_eq!(result[..], expected[..], abs_all <= 1e-5);
        }
    }

    #[test]
    fn periodic_wave_2f() {
        // 1, 10, 100, 1_000, 10_000 Hz
        for i in 0..5 {
            let freq = 10_f32.powf(i as f32);
            let sample_rate = 44_100;

            let mut context = OfflineAudioContext::new(1, sample_rate, sample_rate as f32);

            let options = PeriodicWaveOptions {
                real: Some(vec![0., 0., 0.]),
                imag: Some(vec![0., 0.5, 0.5]),
                // disable norm, is already tested in `PeriodicWave`
                disable_normalization: true,
            };

            let periodic_wave = context.create_periodic_wave(options);

            let mut osc = context.create_oscillator();
            osc.connect(&context.destination());
            osc.set_periodic_wave(periodic_wave);
            osc.frequency().set_value(freq);
            osc.start_at(0.);

            let output = context.start_rendering_sync();
            let result = output.get_channel_data(0);

            let mut expected = Vec::<f32>::with_capacity(sample_rate);
            let mut phase: f64 = 0.;
            let phase_incr = freq as f64 / sample_rate as f64;

            for _i in 0..sample_rate {
                let mut sample = 0.;
                sample += 0.5 * (1. * phase * 2. * PI).sin();
                sample += 0.5 * (2. * phase * 2. * PI).sin();

                expected.push(sample as f32);

                phase += phase_incr;
                if phase >= 1. {
                    phase -= 1.;
                }
            }

            assert_float_eq!(result[..], expected[..], abs_all <= 1e-5);
        }
    }

    #[test]
    fn polyblep_isolated() {
        // @note: Only first branch of the polyblep seems to be used here.
        // May be due on the simplicity of the test itself where everything is
        // well aligned.

        // square
        {
            let mut signal = [1., 1., 1., 1., -1., -1., -1., -1.];
            let len = signal.len() as f64;
            let dt = 1. / len;

            for (index, s) in signal.iter_mut().enumerate() {
                let phase = index as f64 / len;

                *s += OscillatorRenderer::poly_blep(phase, dt, false);
                *s -= OscillatorRenderer::poly_blep((phase + 0.5) % 1., dt, false);
            }

            let expected = [0., 1., 1., 1., 0., -1., -1., -1.];

            assert_float_eq!(signal[..], expected[..], abs_all <= 0.);
        }

        // sawtooth
        {
            let mut signal = [0., 0.25, 0.75, 1., -1., -0.75, -0.5, -0.25];
            let len = signal.len() as f64;
            let dt = 1. / len;

            for (index, s) in signal.iter_mut().enumerate() {
                let phase = index as f64 / len;
                *s -= OscillatorRenderer::poly_blep((phase + 0.5) % 1., dt, false);
            }

            let expected = [0., 0.25, 0.75, 1., 0., -0.75, -0.5, -0.25];
            assert_float_eq!(signal[..], expected[..], abs_all <= 0.);
        }
    }

    #[test]
    fn osc_sub_quantum_start() {
        let freq = 1.25;
        let sample_rate = 44_100;

        let mut context = OfflineAudioContext::new(1, sample_rate, sample_rate as f32);
        let mut osc = context.create_oscillator();
        osc.connect(&context.destination());
        osc.frequency().set_value(freq);
        osc.start_at(2. / sample_rate as f64);

        let output = context.start_rendering_sync();
        let result = output.get_channel_data(0);

        let mut expected = Vec::<f32>::with_capacity(sample_rate);
        let mut phase: f64 = 0.;
        let phase_incr = freq as f64 / sample_rate as f64;

        expected.push(0.);
        expected.push(0.);

        for _i in 2..sample_rate {
            let sample = (phase * 2. * PI).sin();
            phase += phase_incr;
            expected.push(sample as f32);
        }

        assert_float_eq!(result[..], expected[..], abs_all <= 1e-5);
    }

    // # Test scheduling

    #[test]
    fn osc_sub_sample_start() {
        let freq = 1.;
        let sample_rate = 96000;

        let mut context = OfflineAudioContext::new(1, sample_rate, sample_rate as f32);
        let mut osc = context.create_oscillator();
        osc.connect(&context.destination());
        osc.frequency().set_value(freq);
        // start between second and third sample
        osc.start_at(1.3 / sample_rate as f64);

        let output = context.start_rendering_sync();
        let result = output.get_channel_data(0);

        let mut expected = Vec::<f32>::with_capacity(sample_rate);
        let phase_incr = freq as f64 / sample_rate as f64;
        // on first computed sample, phase is 0.7 (e.g. 2. - 1.3) * phase_incr
        let mut phase: f64 = 0.7 * phase_incr;

        expected.push(0.);
        expected.push(0.);

        for _i in 2..sample_rate {
            let sample = (phase * 2. * PI).sin();
            phase += phase_incr;
            expected.push(sample as f32);
        }

        assert_float_eq!(result[..], expected[..], abs_all <= 1e-5);
    }

    #[test]
    fn osc_sub_quantum_stop() {
        let freq = 2345.6;
        let sample_rate = 44_100;

        let mut context = OfflineAudioContext::new(1, sample_rate, sample_rate as f32);
        let mut osc = context.create_oscillator();
        osc.connect(&context.destination());
        osc.frequency().set_value(freq);
        osc.start_at(0.);
        osc.stop_at(6. / sample_rate as f64);

        let output = context.start_rendering_sync();
        let result = output.get_channel_data(0);

        let mut expected = Vec::<f32>::with_capacity(sample_rate);
        let mut phase: f64 = 0.;
        let phase_incr = freq as f64 / sample_rate as f64;

        for i in 0..sample_rate {
            if i < 6 {
                let sample = (phase * 2. * PI).sin();
                phase += phase_incr;
                expected.push(sample as f32);
            } else {
                expected.push(0.);
            }
        }

        assert_float_eq!(result[..], expected[..], abs_all <= 1e-5);
    }

    #[test]
    fn osc_stop_disarms_future_start() {
        let sample_rate = 44_100;
        let future_start = 2. / sample_rate as f64;

        let mut context = OfflineAudioContext::new(1, 128, sample_rate as f32);
        let mut osc = context.create_oscillator();
        osc.connect(&context.destination());
        osc.start_at(future_start);
        osc.stop();

        let output = context.start_rendering_sync();
        let result = output.get_channel_data(0);

        assert_float_eq!(result[..], vec![0.; 128][..], abs_all <= 0.);
    }

    #[test]
    fn osc_stop_before_start_triggers_onended_without_waiting_for_start_time() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let sample_rate = 44_100.;
        let future_start = 2. * RENDER_QUANTUM_SIZE as f64 / sample_rate;
        let suspend_at = RENDER_QUANTUM_SIZE as f64 / sample_rate;

        let ended = Arc::new(AtomicBool::new(false));
        let ended_in_callback = Arc::clone(&ended);
        let ended_after_render = Arc::clone(&ended);

        let mut context = OfflineAudioContext::new(1, RENDER_QUANTUM_SIZE * 4, sample_rate as f32);
        let mut osc = context.create_oscillator();
        osc.connect(&context.destination());
        osc.start_at(future_start);
        osc.set_onended(move |_| {
            ended_in_callback.store(true, Ordering::Relaxed);
        });
        osc.stop();

        context.suspend_sync(suspend_at, move |_| {
            assert!(ended_after_render.load(Ordering::Relaxed));
        });

        let _ = context.start_rendering_sync();
        assert!(ended.load(Ordering::Relaxed));
    }

    #[test]
    fn osc_sub_sample_stop() {
        let freq = 8910.1;
        let sample_rate = 44_100;

        let mut context = OfflineAudioContext::new(1, sample_rate, sample_rate as f32);
        let mut osc = context.create_oscillator();
        osc.connect(&context.destination());
        osc.frequency().set_value(freq);
        osc.start_at(0.);
        osc.stop_at(19.4 / sample_rate as f64);

        let output = context.start_rendering_sync();
        let result = output.get_channel_data(0);

        let mut expected = Vec::<f32>::with_capacity(sample_rate);
        let mut phase: f64 = 0.;
        let phase_incr = freq as f64 / sample_rate as f64;

        for i in 0..sample_rate {
            if i < 20 {
                let sample = (phase * 2. * PI).sin();
                phase += phase_incr;
                expected.push(sample as f32);
            } else {
                expected.push(0.);
            }
        }

        assert_float_eq!(result[..], expected[..], abs_all <= 1e-5);
    }

    #[test]
    fn test_start_in_the_past() {
        let freq = 8910.1;
        let sample_rate = 44_100;

        let mut context = OfflineAudioContext::new(1, sample_rate, sample_rate as f32);

        context.suspend_sync(128. / sample_rate as f64, move |context| {
            let mut osc = context.create_oscillator();
            osc.connect(&context.destination());
            osc.frequency().set_value(freq);
            osc.start_at(0.);
        });

        let output = context.start_rendering_sync();
        let result = output.get_channel_data(0);

        let mut expected = Vec::<f32>::with_capacity(sample_rate);
        let mut phase: f64 = 0.;
        let phase_incr = freq as f64 / sample_rate as f64;

        for i in 0..sample_rate {
            if i < 128 {
                expected.push(0.);
            } else {
                let sample = (phase * 2. * PI).sin();
                expected.push(sample as f32);
                phase += phase_incr;
            }
        }

        assert_float_eq!(result[..], expected[..], abs_all <= 1e-5);
    }

    #[test]
    fn compute_freq_above_nyquist_outputs_zero() {
        let freq = 20000.;
        let detune = 1200.; // one octave upper, then computed feq is 40000Hz
        let sample_rate = 44_100;

        let mut context = OfflineAudioContext::new(1, 128, sample_rate as f32);

        let mut osc = context.create_oscillator();
        osc.connect(&context.destination());
        osc.frequency().set_value(freq);
        osc.detune().set_value(detune);
        osc.start_at(0.);

        let output = context.start_rendering_sync();
        let result = output.get_channel_data(0);

        assert_float_eq!(result[..], [0.; 128], abs_all <= 1e-5);
    }

    #[test]
    fn compute_freq_below_negative_nyquist_outputs_zero() {
        let freq = -20000.;
        let detune = 1200.; // one octave lower, then computed feq is -40000Hz
        let sample_rate = 44_100;

        let mut context = OfflineAudioContext::new(1, 128, sample_rate as f32);

        let mut osc = context.create_oscillator();
        osc.connect(&context.destination());
        osc.frequency().set_value(freq);
        osc.detune().set_value(detune);
        osc.start_at(0.);

        let output = context.start_rendering_sync();
        let result = output.get_channel_data(0);

        assert_float_eq!(result[..], [0.; 128], abs_all <= 1e-5);
    }

    #[test]
    fn oscillator_can_reenter_audible_range_after_large_phase_increments() {
        let sample_rate = 44_100;
        let mut context = OfflineAudioContext::new(1, 256, sample_rate as f32);

        let mut osc = context.create_oscillator();
        osc.connect(&context.destination());
        osc.frequency().set_value(20_000.);
        osc.detune().set_value(2400.); // computed frequency is 80_000Hz
        osc.detune()
            .set_value_at_time(0., RENDER_QUANTUM_SIZE as f64 / sample_rate as f64);
        osc.start_at(0.);

        let output = context.start_rendering_sync();
        let result = output.get_channel_data(0);

        assert_float_eq!(
            result[..RENDER_QUANTUM_SIZE],
            [0.; RENDER_QUANTUM_SIZE],
            abs_all <= 1e-5
        );
        assert!(result[RENDER_QUANTUM_SIZE..].iter().all(|v| v.is_finite()));
        assert!(result[RENDER_QUANTUM_SIZE..].iter().any(|&v| v != 0.));
    }

    #[test]
    fn oscillator_delayed_start_renders_first_fully_active_block() {
        let sample_rate = 44_100;
        let start_time = RENDER_QUANTUM_SIZE as f64 / sample_rate as f64;
        let mut context = OfflineAudioContext::new(1, RENDER_QUANTUM_SIZE * 2, sample_rate as f32);

        let mut osc = context.create_oscillator();
        osc.connect(&context.destination());
        osc.start_at(start_time);

        let output = context.start_rendering_sync();
        let result = output.get_channel_data(0);

        assert_float_eq!(
            result[..RENDER_QUANTUM_SIZE],
            [0.; RENDER_QUANTUM_SIZE],
            abs_all <= 1e-5
        );
        assert!(result[RENDER_QUANTUM_SIZE..].iter().any(|&v| v != 0.));
    }

    #[test]
    fn sine_negative_frequency() {
        let freq = -100.;
        let sample_rate = 44_100;
        let length = sample_rate as usize;

        let mut context = OfflineAudioContext::new(1, length, sample_rate as f32);

        let mut osc = context.create_oscillator();
        osc.connect(&context.destination());
        osc.frequency().set_value(freq);
        osc.start_at(0.);

        let output = context.start_rendering_sync();
        let result = output.get_channel_data(0);
        let mut expected = Vec::<f32>::with_capacity(length);

        for i in 0..length {
            let phase = freq as f64 * i as f64 / sample_rate as f64;
            let sample = (phase * 2. * PI).sin();
            // phase += phase_incr;
            expected.push(sample as f32);
        }

        assert_float_eq!(result[..], expected[..], abs_all <= 1e-5);
    }
}
