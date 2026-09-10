//! AudioParam interface

use std::any::Any;
use std::slice::{Iter, IterMut};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};

use arrayvec::ArrayVec;

use crate::context::AudioContextRegistration;
use crate::node::{
    AudioNode, AudioNodeOptions, ChannelConfig, ChannelCountMode, ChannelInterpretation,
};
use crate::render::{
    AudioParamValues, AudioProcessor, AudioRenderQuantum, AudioWorkletGlobalScope,
};
use crate::{assert_valid_time_value, AtomicF32, RENDER_QUANTUM_SIZE};

/// For SetTargetAtTime event, that theoretically cannot end, if the diff between
/// the current value and the target is below this threshold, the value is set
/// to target value and the event is considered ended.
const SNAP_TO_TARGET: f32 = 1e-10;

#[track_caller]
fn assert_is_finite(value: f32) {
    assert!(
        value.is_finite(),
        "TypeError - The provided value is non-finite."
    );
}

#[track_caller]
fn assert_strictly_positive(value: f64) {
    assert!(
        value.is_finite(),
        "TypeError - The provided value is non-finite."
    );
    assert!(
        value > 0.,
        "RangeError - duration ({:?}) should be strictly positive",
        value
    );
}

#[track_caller]
fn assert_not_zero(value: f32) {
    assert_is_finite(value);
    assert_ne!(
        value, 0.,
        "RangeError - value ({:?}) should not be equal to zero",
        value
    );
}

#[track_caller]
fn assert_sequence_length(values: &[f32]) {
    assert!(
        values.len() >= 2,
        "InvalidStateError - sequence length ({:?}) should not be less than 2",
        values.len()
    );
}

// 𝑣(𝑡) = 𝑉0 + (𝑉1−𝑉0) * ((𝑡−𝑇0) / (𝑇1−𝑇0))
#[inline(always)]
fn compute_linear_ramp_sample(
    start_time: f64,
    duration: f64,
    start_value: f32,
    diff: f32, // end_value - start_value
    time: f64,
) -> f32 {
    let phase = (time - start_time) / duration;
    diff.mul_add(phase as f32, start_value)
}

// v(t) = v1 * (v2/v1)^((t-t1) / (t2-t1))
#[inline(always)]
fn compute_exponential_ramp_sample(
    start_time: f64,
    duration: f64,
    start_value: f32,
    ratio: f32, // end_value / start_value
    time: f64,
) -> f32 {
    let phase = (time - start_time) / duration;
    start_value * ratio.powf(phase as f32)
}

// 𝑣(𝑡) = 𝑉1 + (𝑉0 − 𝑉1) * 𝑒^−((𝑡−𝑇0) / 𝜏)
#[inline(always)]
fn compute_set_target_sample(
    start_time: f64,
    time_constant: f64,
    end_value: f32,
    diff: f32, // start_value - end_value
    time: f64,
) -> f32 {
    let exponent = -((time - start_time) / time_constant);
    diff.mul_add(exponent.exp() as f32, end_value)
}

// 𝑘=⌊𝑁−1 / 𝑇𝐷 * (𝑡−𝑇0)⌋
// Then 𝑣(𝑡) is computed by linearly interpolating between 𝑉[𝑘] and 𝑉[𝑘+1],
#[inline(always)]
fn compute_set_value_curve_sample(
    start_time: f64,
    duration: f64,
    values: &[f32],
    time: f64,
) -> f32 {
    if time - start_time >= duration {
        return values[values.len() - 1];
    }

    let position = (values.len() - 1) as f64 * (time - start_time) / duration;
    let k = position as usize;
    let phase = (position - position.floor()) as f32;
    (values[k + 1] - values[k]).mul_add(phase, values[k])
}

/// Precision of AudioParam value calculation per render quantum
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum AutomationRate {
    /// Audio Rate - sampled for each sample-frame of the block
    A,
    /// Control Rate - sampled at the time of the very first sample-frame,
    /// then used for the entire block
    K,
}

impl AutomationRate {
    fn is_a_rate(self) -> bool {
        match self {
            AutomationRate::A => true,
            AutomationRate::K => false,
        }
    }
}

/// Options for constructing an [`AudioParam`]
#[derive(Clone, Debug)]
pub struct AudioParamDescriptor {
    pub name: String,
    pub automation_rate: AutomationRate,
    pub default_value: f32,
    pub min_value: f32,
    pub max_value: f32,
}

#[derive(PartialEq, Eq, Debug, Copy, Clone)]
enum AudioParamEventType {
    SetValue,
    SetValueAtTime,
    LinearRampToValueAtTime,
    ExponentialRampToValueAtTime,
    CancelScheduledValues,
    SetTargetAtTime,
    CancelAndHoldAtTime,
    SetValueCurveAtTime,
}

#[derive(Debug)]
pub(crate) struct AudioParamEvent {
    event_type: AudioParamEventType,
    value: f32,
    time: f64,
    time_constant: Option<f64>, // populated by `SetTargetAtTime` events
    cancel_time: Option<f64>,   // populated by `CancelAndHoldAtTime` events
    duration: Option<f64>,      // populated by `SetValueCurveAtTime` events
    values: Option<Box<[f32]>>, // populated by `SetValueCurveAtTime` events
}

// Event queue that contains `AudioParamEvent`s, most of the time, events must be
// ordered (using stable sort), some operation may break this ordering (e.g. `push`)
// in which cases `sort` must be called explicitly.
// In the current implementation of the param rendering, `sort` is called once after
// all events have been inserted in the queue at each `tick` (with the exception
// `CancelAndHoldAtTime` which needs a clean queue to find its neighbors, but this
// occurs during the insertion of events)
// After this point, the queue should be considered sorted and no operations that
// breaks the ordering should be done.
#[derive(Debug, Default)]
struct AudioParamEventTimeline {
    inner: Vec<AudioParamEvent>,
    dirty: bool,
}

impl AudioParamEventTimeline {
    fn new() -> Self {
        Self {
            inner: Vec::with_capacity(32),
            dirty: false,
        }
    }

    fn push(&mut self, item: AudioParamEvent) {
        self.dirty = true;
        self.inner.push(item);
    }

    // `pop` and `retain` preserve order so they don't make the queue dirty
    fn pop(&mut self) -> Option<AudioParamEvent> {
        if !self.inner.is_empty() {
            Some(self.inner.remove(0))
        } else {
            None
        }
    }

    fn retain<F>(&mut self, func: F)
    where
        F: Fn(&AudioParamEvent) -> bool,
    {
        self.inner.retain(func);
    }

    // Only used to handle special cases in `ExponentialRampToValueAtTime`:
    // as the replaced item has the same time, order is preserved.
    // If the method turned out to be used elsewhere, this could maybe
    // become wrong, be careful here.
    fn replace_peek(&mut self, item: AudioParamEvent) {
        self.inner[0] = item;
    }

    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    fn unsorted_peek(&self) -> Option<&AudioParamEvent> {
        self.inner.first()
    }

    // panic if dirty, we are doing something wrong here
    fn peek(&self) -> Option<&AudioParamEvent> {
        assert!(
            !self.dirty,
            "`AudioParamEventTimeline`: Invalid `.peek()` call, the queue is dirty"
        );
        self.inner.first()
    }

    fn next(&self) -> Option<&AudioParamEvent> {
        assert!(
            !self.dirty,
            "`AudioParamEventTimeline`: Invalid `.next()` call, the queue is dirty"
        );
        self.inner.get(1)
    }

    fn sort(&mut self) {
        self.inner
            .sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap());
        self.dirty = false;
    }

    fn iter(&mut self) -> Iter<'_, AudioParamEvent> {
        self.inner.iter()
    }

    fn iter_mut(&mut self) -> IterMut<'_, AudioParamEvent> {
        self.inner.iter_mut()
    }
}

/// AudioParam controls an individual aspect of an AudioNode's functionality, such as volume.
#[derive(Clone)] // `Clone` for the node bindings, see #378
pub struct AudioParam {
    registration: Arc<AudioContextRegistration>,
    raw_parts: AudioParamInner,
}

impl std::fmt::Debug for AudioParam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioParam")
            .field("registration", &self.registration())
            .field("automation_rate", &self.automation_rate())
            .field(
                "automation_rate_constrained",
                &self.raw_parts.automation_rate_constrained,
            )
            .field("default_value", &self.default_value())
            .field("min_value", &self.min_value())
            .field("max_value", &self.max_value())
            .finish()
    }
}

// helper struct to attach / detach to context (for borrow reasons)
#[derive(Debug, Clone)]
pub(crate) struct AudioParamInner {
    default_value: f32,                          // immutable
    min_value: f32,                              // immutable
    max_value: f32,                              // immutable
    automation_rate_constrained: bool,           // effectively immutable
    automation_rate: Arc<Mutex<AutomationRate>>, // shared with clones
    current_value: Arc<AtomicF32>,               // shared with clones and with render thread
}

impl AudioNode for AudioParam {
    fn registration(&self) -> &AudioContextRegistration {
        &self.registration
    }

    fn channel_config(&self) -> &'static ChannelConfig {
        static INSTANCE: OnceLock<ChannelConfig> = OnceLock::new();
        INSTANCE.get_or_init(|| {
            AudioNodeOptions {
                channel_count: 1,
                channel_count_mode: ChannelCountMode::Explicit,
                channel_interpretation: ChannelInterpretation::Discrete,
            }
            .into()
        })
    }

    fn number_of_inputs(&self) -> usize {
        1
    }

    fn number_of_outputs(&self) -> usize {
        1
    }

    fn set_channel_count(&self, _v: usize) {
        panic!("NotSupportedError - AudioParam has channel count constraints");
    }
    fn set_channel_count_mode(&self, _v: ChannelCountMode) {
        panic!("NotSupportedError - AudioParam has channel count mode constraints");
    }
    fn set_channel_interpretation(&self, _v: ChannelInterpretation) {
        panic!("NotSupportedError - AudioParam has channel interpretation constraints");
    }
}

impl AudioParam {
    /// Current value of the automation rate of the AudioParam
    #[allow(clippy::missing_panics_doc)]
    pub fn automation_rate(&self) -> AutomationRate {
        *self.raw_parts.automation_rate.lock().unwrap()
    }

    /// Update the current value of the automation rate of the AudioParam
    ///
    /// # Panics
    ///
    /// Some nodes have automation rate constraints and may panic when updating the value.
    pub fn set_automation_rate(&self, value: AutomationRate) {
        assert!(
            !self.raw_parts.automation_rate_constrained || value == self.automation_rate(),
            "InvalidStateError - automation rate cannot be changed for this param"
        );

        let mut guard = self.raw_parts.automation_rate.lock().unwrap();
        *guard = value;
        self.registration().post_message(value);
        drop(guard); // drop guard after sending message to prevent out of order arrivals on
                     // concurrent access
    }

    pub(crate) fn set_automation_rate_constrained(&mut self, value: bool) {
        self.raw_parts.automation_rate_constrained = value;
    }

    pub fn default_value(&self) -> f32 {
        self.raw_parts.default_value
    }

    pub fn min_value(&self) -> f32 {
        self.raw_parts.min_value
    }

    pub fn max_value(&self) -> f32 {
        self.raw_parts.max_value
    }

    /// Retrieve the current value of the `AudioParam`.
    //
    // @note: the choice here is to have this coherent with the first sample of
    // the last rendered block, which means `intrinsic_value` must be calculated
    // for next_block_time at each tick.
    // @note - maybe check with spec editors that it is correct
    //
    // see. test_linear_ramp_arate_multiple_blocks
    //      test_linear_ramp_krate_multiple_blocks
    //      test_exponential_ramp_a_rate_multiple_blocks
    //      test_exponential_ramp_k_rate_multiple_blocks
    pub fn value(&self) -> f32 {
        self.raw_parts.current_value.load(Ordering::Acquire)
    }

    /// Set the value of the `AudioParam`.
    ///
    /// Is equivalent to calling the `set_value_at_time` method with the current
    /// AudioContext's currentTime
    //
    // @note: Setting this attribute has the effect of assigning the requested value
    // to the [[current value]] slot, and calling the setValueAtTime() method
    // with the current AudioContext's currentTime and [[current value]].
    // Any exceptions that would be thrown by setValueAtTime() will also be
    // thrown by setting this attribute.
    // cf. https://www.w3.org/TR/webaudio/#dom-audioparam-value
    pub fn set_value(&self, value: f32) -> &Self {
        self.send_event(self.set_value_raw(value))
    }

    fn set_value_raw(&self, value: f32) -> AudioParamEvent {
        assert_is_finite(value);
        // current_value should always be clamped
        let clamped = value.clamp(self.raw_parts.min_value, self.raw_parts.max_value);
        self.raw_parts
            .current_value
            .store(clamped, Ordering::Release);

        // this event is meant to update param intrinsic value before any calculation
        // is done, will behave as SetValueAtTime with `time == block_timestamp`
        AudioParamEvent {
            event_type: AudioParamEventType::SetValue,
            value,
            time: 0.,
            time_constant: None,
            cancel_time: None,
            duration: None,
            values: None,
        }
    }

    /// Schedules a parameter value change at the given time.
    ///
    /// # Panics
    ///
    /// Will panic if `start_time` is negative
    pub fn set_value_at_time(&self, value: f32, start_time: f64) -> &Self {
        self.send_event(self.set_value_at_time_raw(value, start_time))
    }

    fn set_value_at_time_raw(&self, value: f32, start_time: f64) -> AudioParamEvent {
        assert_is_finite(value);
        assert_valid_time_value(start_time);

        AudioParamEvent {
            event_type: AudioParamEventType::SetValueAtTime,
            value,
            time: start_time,
            time_constant: None,
            cancel_time: None,
            duration: None,
            values: None,
        }
    }

    /// Schedules a linear continuous change in parameter value from the
    /// previous scheduled parameter value to the given value.
    ///
    /// # Panics
    ///
    /// Will panic if `end_time` is negative
    pub fn linear_ramp_to_value_at_time(&self, value: f32, end_time: f64) -> &Self {
        self.send_event(self.linear_ramp_to_value_at_time_raw(value, end_time))
    }

    fn linear_ramp_to_value_at_time_raw(&self, value: f32, end_time: f64) -> AudioParamEvent {
        assert_is_finite(value);
        assert_valid_time_value(end_time);

        AudioParamEvent {
            event_type: AudioParamEventType::LinearRampToValueAtTime,
            value,
            time: end_time,
            time_constant: None,
            cancel_time: None,
            duration: None,
            values: None,
        }
    }

    /// Schedules an exponential continuous change in parameter value from the
    /// previous scheduled parameter value to the given value.
    ///
    /// # Panics
    ///
    /// Will panic if:
    /// - `value` is zero
    /// - `end_time` is negative
    pub fn exponential_ramp_to_value_at_time(&self, value: f32, end_time: f64) -> &Self {
        self.send_event(self.exponential_ramp_to_value_at_time_raw(value, end_time))
    }

    fn exponential_ramp_to_value_at_time_raw(&self, value: f32, end_time: f64) -> AudioParamEvent {
        assert_not_zero(value);
        assert_valid_time_value(end_time);

        AudioParamEvent {
            event_type: AudioParamEventType::ExponentialRampToValueAtTime,
            value,
            time: end_time,
            time_constant: None,
            cancel_time: None,
            duration: None,
            values: None,
        }
    }

    /// Start exponentially approaching the target value at the given time with
    /// a rate having the given time constant.
    ///
    /// # Panics
    ///
    /// Will panic if:
    /// - `start_time` is negative
    /// - `time_constant` is negative
    pub fn set_target_at_time(&self, value: f32, start_time: f64, time_constant: f64) -> &Self {
        self.send_event(self.set_target_at_time_raw(value, start_time, time_constant))
    }

    fn set_target_at_time_raw(
        &self,
        value: f32,
        start_time: f64,
        time_constant: f64,
    ) -> AudioParamEvent {
        assert_is_finite(value);
        assert_valid_time_value(start_time);
        assert_valid_time_value(time_constant);

        // [spec] If timeConstant is zero, the output value jumps immediately to the final value.
        if time_constant == 0. {
            AudioParamEvent {
                event_type: AudioParamEventType::SetValueAtTime,
                value,
                time: start_time,
                time_constant: None,
                cancel_time: None,
                duration: None,
                values: None,
            }
        } else {
            AudioParamEvent {
                event_type: AudioParamEventType::SetTargetAtTime,
                value,
                time: start_time,
                time_constant: Some(time_constant),
                cancel_time: None,
                duration: None,
                values: None,
            }
        }
    }

    /// Cancels all scheduled parameter changes with times greater than or equal
    /// to `cancel_time`.
    ///
    /// # Panics
    ///
    /// Will panic if `cancel_time` is negative
    pub fn cancel_scheduled_values(&self, cancel_time: f64) -> &Self {
        self.send_event(self.cancel_scheduled_values_raw(cancel_time))
    }

    fn cancel_scheduled_values_raw(&self, cancel_time: f64) -> AudioParamEvent {
        assert_valid_time_value(cancel_time);

        AudioParamEvent {
            event_type: AudioParamEventType::CancelScheduledValues,
            value: 0., // no value
            time: cancel_time,
            time_constant: None,
            cancel_time: None,
            duration: None,
            values: None,
        }
    }

    /// Cancels all scheduled parameter changes with times greater than or equal
    /// to `cancel_time` and the automation value that would have happened at
    /// that time is then propagated for all future time.
    ///
    /// # Panics
    ///
    /// Will panic if `cancel_time` is negative
    pub fn cancel_and_hold_at_time(&self, cancel_time: f64) -> &Self {
        self.send_event(self.cancel_and_hold_at_time_raw(cancel_time))
    }

    fn cancel_and_hold_at_time_raw(&self, cancel_time: f64) -> AudioParamEvent {
        assert_valid_time_value(cancel_time);

        AudioParamEvent {
            event_type: AudioParamEventType::CancelAndHoldAtTime,
            value: 0., // value will be defined by cancel event
            time: cancel_time,
            time_constant: None,
            cancel_time: None,
            duration: None,
            values: None,
        }
    }

    /// Sets an array of arbitrary parameter values starting at the given time
    /// for the given duration.
    ///
    /// # Panics
    ///
    /// Will panic if:
    /// - `value` length is less than 2
    /// - `start_time` is negative
    /// - `duration` is negative or equal to zero
    pub fn set_value_curve_at_time(&self, values: &[f32], start_time: f64, duration: f64) -> &Self {
        self.send_event(self.set_value_curve_at_time_raw(values, start_time, duration))
    }

    fn set_value_curve_at_time_raw(
        &self,
        values: &[f32],
        start_time: f64,
        duration: f64,
    ) -> AudioParamEvent {
        assert_sequence_length(values);
        assert_valid_time_value(start_time);
        assert_strictly_positive(duration);

        // When this method is called, an internal copy of the curve is
        // created for automation purposes.
        let copy = values.to_vec();
        let boxed_copy = copy.into_boxed_slice();

        AudioParamEvent {
            event_type: AudioParamEventType::SetValueCurveAtTime,
            value: 0., // value will be defined at the end of the event
            time: start_time,
            time_constant: None,
            cancel_time: None,
            duration: Some(duration),
            values: Some(boxed_copy),
        }
    }

    // helper function to detach from context (for borrow reasons)
    pub(crate) fn into_raw_parts(self) -> AudioParamInner {
        let Self {
            registration: _,
            raw_parts,
        } = self;
        raw_parts
    }

    // helper function to attach to context (for borrow reasons)
    pub(crate) fn from_raw_parts(
        registration: AudioContextRegistration,
        raw_parts: AudioParamInner,
    ) -> Self {
        Self {
            registration: registration.into(),
            raw_parts,
        }
    }

    fn send_event(&self, event: AudioParamEvent) -> &Self {
        self.registration().post_message(event);
        self
    }
}

struct BlockInfos {
    block_time: f64,
    dt: f64,
    count: usize,
    is_a_rate: bool,
    next_block_time: f64,
}

#[derive(Debug)]
pub(crate) struct AudioParamProcessor {
    default_value: f32, // immutable
    min_value: f32,     // immutable
    max_value: f32,     // immutable
    intrinsic_value: f32,
    automation_rate: AutomationRate,
    current_value: Arc<AtomicF32>,
    event_timeline: AudioParamEventTimeline,
    last_event: Option<AudioParamEvent>,
    buffer: ArrayVec<f32, RENDER_QUANTUM_SIZE>,
}

impl AudioProcessor for AudioParamProcessor {
    fn process(
        &mut self,
        inputs: &[AudioRenderQuantum],
        outputs: &mut [AudioRenderQuantum],
        _params: AudioParamValues<'_>,
        scope: &AudioWorkletGlobalScope,
    ) -> bool {
        let period = 1. / scope.sample_rate as f64;

        let input = &inputs[0]; // single input mode
        let output = &mut outputs[0];

        self.compute_intrinsic_values(scope.current_time, period, RENDER_QUANTUM_SIZE);
        self.mix_to_output(input, output);

        true // has intrinsic value
    }

    fn onmessage(&mut self, msg: &mut dyn Any) {
        if let Some(automation_rate) = msg.downcast_ref::<AutomationRate>() {
            self.automation_rate = *automation_rate;
            return;
        }

        if let Some(event) = msg.downcast_mut::<AudioParamEvent>() {
            // Avoid deallocation of the event by replacing it with a tombstone.
            let tombstone_event = AudioParamEvent {
                event_type: AudioParamEventType::SetValue,
                value: Default::default(),
                time: Default::default(),
                time_constant: None,
                cancel_time: None,
                duration: None,
                values: None,
            };
            let event = std::mem::replace(event, tombstone_event);
            self.handle_incoming_event(event);
            return;
        };

        log::warn!("AudioParamProcessor: Dropping incoming message {msg:?}");
    }
}

impl AudioParamProcessor {
    // Warning: compute_intrinsic_values in called directly in the unit tests so
    // everything important for the tests should be done here
    // The returned value is only used in tests
    fn compute_intrinsic_values(&mut self, block_time: f64, dt: f64, count: usize) -> &[f32] {
        self.compute_buffer(block_time, dt, count);
        self.buffer.as_slice()
    }

    fn mix_to_output(&mut self, input: &AudioRenderQuantum, output: &mut AudioRenderQuantum) {
        #[cfg(test)]
        assert!(self.buffer.len() == 1 || self.buffer.len() == RENDER_QUANTUM_SIZE);

        // handle all k-rate and inactive a-rate processing
        if self.buffer.len() == 1 || !self.automation_rate.is_a_rate() {
            let mut value = self.buffer[0];

            // we can return a 1-sized buffer if either
            // - the input signal is zero, or
            // - if this is k-rate processing
            if input.is_silent() || !self.automation_rate.is_a_rate() {
                output.set_single_valued(true);

                value += input.channel_data(0)[0];

                if value.is_nan() {
                    value = self.default_value;
                } else {
                    // do not use `clamp` because it has extra branches for NaN or when max < min
                    value = value.max(self.min_value).min(self.max_value);
                }
                output.channel_data_mut(0)[0] = value;
            } else {
                // a-rate processing and input non-zero
                output.set_single_valued(false);
                *output = input.clone();
                output.channel_data_mut(0).iter_mut().for_each(|o| {
                    *o += value;

                    if o.is_nan() {
                        *o = self.default_value;
                    } else {
                        // do not use `clamp` because it has extra branches for NaN or when max < min
                        *o = o.max(self.min_value).min(self.max_value);
                    }
                });
            }
        } else {
            // a-rate processing
            *output = input.clone();
            output.set_single_valued(false);

            output
                .channel_data_mut(0)
                .iter_mut()
                .zip(self.buffer.iter())
                .for_each(|(o, p)| {
                    *o += p;

                    if o.is_nan() {
                        *o = self.default_value;
                    } else {
                        // do not use `clamp` because it has extra branches for NaN or when max < min
                        *o = o.max(self.min_value).min(self.max_value);
                    }
                });
        }
    }

    fn handle_incoming_event(&mut self, event: AudioParamEvent) {
        // cf. https://www.w3.org/TR/webaudio/#computation-of-value
        // 1. paramIntrinsicValue will be calculated at each time, which is either the
        // value set directly to the value attribute, or, if there are any automation
        // events with times before or at this time, the value as calculated from
        // these events. If automation events are removed from a given time range,
        // then the paramIntrinsicValue value will remain unchanged and stay at its
        // previous value until either the value attribute is directly set, or
        // automation events are added for the time range.

        // handle CancelScheduledValues events
        // cf. https://www.w3.org/TR/webaudio/#dom-audioparam-cancelscheduledvalues
        if event.event_type == AudioParamEventType::CancelScheduledValues {
            // peek current event before inserting new events, and possibly sort
            // the queue, we need that for checking that we are (or not) in the middle
            // of a ramp when handling `CancelScheduledValues`
            // @note - probably not robust enough in some edge cases where the
            // event is not the first received at this tick (`SetValueCurveAtTime`
            // and `CancelAndHold` need to sort the queue)
            let some_current_event = self.event_timeline.unsorted_peek();

            match some_current_event {
                None => (),
                Some(current_event) => {
                    // @note - The spec is not particularly clear about what to do
                    // with on-going SetTarget events, but both Chrome and Firefox
                    // ignore the Cancel event if at same time nor if startTime is
                    // equal to cancelTime, this can makes sens as the event time
                    // (which is a `startTime` for `SetTarget`) is before the `cancelTime`
                    // (maybe this should be clarified w/ spec editors)

                    match current_event.event_type {
                        AudioParamEventType::LinearRampToValueAtTime
                        | AudioParamEventType::ExponentialRampToValueAtTime
                            // we are in the middle of a ramp
                            //
                            // @note - Firefox and Chrome behave differently
                            // on this: Firefox actually restore intrinsic_value
                            // from the value at the beginning of the vent, while
                            // Chrome just keeps the current intrinsic_value
                            // The spec is not very clear there, but Firefox
                            // seems to be more compliant:
                            // "Any active automations whose automation event
                            // time is less than cancelTime are also cancelled,
                            // and such cancellations may cause discontinuities
                            // because the original value (**from before such
                            // automation**) is restored immediately."
                            //
                            // @note - last_event cannot be `None` here, because
                            // linear or exponential ramps are always preceded
                            // by another event (even a set_value_at_time
                            // inserted implicitly), so if the ramp is the next
                            // event that means that at least one event has
                            // already been processed.
                            if current_event.time >= event.time => {
                                let last_event = self.last_event.as_ref().unwrap();
                                self.intrinsic_value = last_event.value;
                            }
                        _ => (),
                    }
                }
            }

            // remove all events in queue where event.time >= cancel_time
            // i.e. keep all events where event.time < cancel_time
            self.event_timeline
                .retain(|queued| queued.time < event.time);
            return; // cancel_values events are not inserted in queue
        }

        if event.event_type == AudioParamEventType::CancelAndHoldAtTime {
            // 1. Let 𝐸1 be the event (if any) at time 𝑡1 where 𝑡1 is the
            // largest number satisfying 𝑡1 ≤ 𝑡𝑐.
            // 2. Let 𝐸2 be the event (if any) at time 𝑡2 where 𝑡2 is the
            // smallest number satisfying 𝑡𝑐 < 𝑡2.
            let mut e1: Option<&mut AudioParamEvent> = None;
            let mut e2: Option<&mut AudioParamEvent> = None;
            let mut t1 = f64::MIN;
            let mut t2 = f64::MAX;
            // we need a sorted timeline here to find siblings
            self.event_timeline.sort();

            for queued in self.event_timeline.iter_mut() {
                // closest before cancel time: if several events at same time,
                // we want the last one
                if queued.time >= t1 && queued.time <= event.time {
                    t1 = queued.time;
                    e1 = Some(queued);
                    // closest after cancel time: if several events at same time,
                    // we want the first one
                } else if queued.time < t2 && queued.time > event.time {
                    t2 = queued.time;
                    e2 = Some(queued);
                }
            }

            // If 𝐸2 exists:
            if let Some(matched) = e2 {
                // If 𝐸2 is a linear or exponential ramp,
                // Effectively rewrite 𝐸2 to be the same kind of ramp ending
                // at time 𝑡𝑐 with an end value that would be the value of the
                // original ramp at time 𝑡𝑐.
                // @note - this is done during the actual computation of the
                //  ramp using the cancel_time
                if matched.event_type == AudioParamEventType::LinearRampToValueAtTime
                    || matched.event_type == AudioParamEventType::ExponentialRampToValueAtTime
                {
                    matched.cancel_time = Some(event.time);
                }
            } else if let Some(matched) = e1 {
                if matched.event_type == AudioParamEventType::SetTargetAtTime {
                    // Implicitly insert a setValueAtTime event at time 𝑡𝑐 with
                    // the value that the setTarget would
                    // @note - same strategy as for ramps
                    matched.cancel_time = Some(event.time);
                } else if matched.event_type == AudioParamEventType::SetValueCurveAtTime {
                    // If 𝐸1 is a setValueCurve with a start time of 𝑡3 and a duration of 𝑑
                    // If 𝑡𝑐 <= 𝑡3 + 𝑑 :
                    // Effectively replace this event with a setValueCurve event
                    // with a start time of 𝑡3 and a new duration of 𝑡𝑐−𝑡3. However,
                    // this is not a true replacement; this automation MUST take
                    // care to produce the same output as the original, and not
                    // one computed using a different duration. (That would cause
                    // sampling of the value curve in a slightly different way,
                    // producing different results.)
                    let start_time = matched.time;
                    let duration = matched.duration.unwrap();

                    if event.time <= start_time + duration {
                        matched.cancel_time = Some(event.time);
                    }
                }
            }

            // [spec] Remove all events with time greater than 𝑡𝑐.
            self.event_timeline.retain(|queued| {
                let mut time = queued.time;
                // if the event has a `cancel_time` we use it instead of `time`
                if let Some(cancel_time) = queued.cancel_time {
                    time = cancel_time;
                }

                time <= event.time
            });
            return; // cancel_and_hold events are not inserted timeline
        }

        // handle SetValueCurveAtTime
        // @note - These rules argue in favor of having events inserted in
        // the control thread, let's panic for now
        //
        // [spec] If setValueCurveAtTime() is called for time 𝑇 and duration 𝐷
        // and there are any events having a time strictly greater than 𝑇, but
        // strictly less than 𝑇+𝐷, then a NotSupportedError exception MUST be thrown.
        // In other words, it’s not ok to schedule a value curve during a time period
        // containing other events, but it’s ok to schedule a value curve exactly
        // at the time of another event.
        if event.event_type == AudioParamEventType::SetValueCurveAtTime {
            // check if we don't try to insert at the time of another event
            let start_time = event.time;
            let end_time = start_time + event.duration.unwrap();

            for queued in self.event_timeline.iter() {
                assert!(
                    queued.time <= start_time || queued.time >= end_time,
                    "NotSupportedError - scheduling SetValueCurveAtTime ({:?}) at time of another automation event ({:?})",
                    event, queued,
                );
            }
        }

        // [spec] Similarly a NotSupportedError exception MUST be thrown if any
        // automation method is called at a time which is contained in [𝑇,𝑇+𝐷), 𝑇
        // being the time of the curve and 𝐷 its duration.
        // @note - Cancel methods are not automation methods
        if event.event_type == AudioParamEventType::SetValueAtTime
            || event.event_type == AudioParamEventType::SetValue
            || event.event_type == AudioParamEventType::LinearRampToValueAtTime
            || event.event_type == AudioParamEventType::ExponentialRampToValueAtTime
            || event.event_type == AudioParamEventType::SetTargetAtTime
        {
            for queued in self.event_timeline.iter() {
                if queued.event_type == AudioParamEventType::SetValueCurveAtTime {
                    let start_time = queued.time;
                    let end_time = start_time + queued.duration.unwrap();

                    assert!(
                        event.time <= start_time || event.time >= end_time,
                        "NotSupportedError - scheduling automation event ({:?}) during SetValueCurveAtTime ({:?})",
                        event, queued,
                    );
                }
            }
        }

        // handle SetValue - param intrinsic value must be updated from event value
        if event.event_type == AudioParamEventType::SetValue {
            self.intrinsic_value = event.value;
        }

        // If no event in the timeline and event_type is `LinearRampToValueAtTime`
        // or `ExponentialRampToValue` at time, we must insert a `SetValueAtTime`
        // with intrinsic value and calling time.
        // cf. https://www.w3.org/TR/webaudio/#dom-audioparam-linearramptovalueattime
        // cf. https://www.w3.org/TR/webaudio/#dom-audioparam-exponentialramptovalueattime
        if self.event_timeline.is_empty()
            && self.last_event.is_none()
            && (event.event_type == AudioParamEventType::LinearRampToValueAtTime
                || event.event_type == AudioParamEventType::ExponentialRampToValueAtTime)
        {
            let set_value_event = AudioParamEvent {
                event_type: AudioParamEventType::SetValue,
                value: self.intrinsic_value,
                // make sure the event is applied before any other event, time
                // will be replaced by the block timestamp during event processing
                time: 0.,
                time_constant: None,
                cancel_time: None,
                duration: None,
                values: None,
            };

            self.event_timeline.push(set_value_event);
        }

        // for SetTarget, this behavior is not per se specified, but it allows
        // to make sure we have a stable start_value available without having
        // to store it elsewhere.
        if self.event_timeline.is_empty()
            && event.event_type == AudioParamEventType::SetTargetAtTime
        {
            let set_value_event = AudioParamEvent {
                event_type: AudioParamEventType::SetValue,
                value: self.intrinsic_value,
                // make sure the event is applied before any other event, time
                // will be replaced by the block timestamp during event processing
                time: 0.,
                time_constant: None,
                cancel_time: None,
                duration: None,
                values: None,
            };

            self.event_timeline.push(set_value_event);
        }

        self.event_timeline.push(event);
        self.event_timeline.sort();
    }

    // https://www.w3.org/TR/webaudio/%23dom-audioparam-linearramptovalueattime#dom-audioparam-setvalueattime
    fn compute_set_value_automation(&mut self, infos: &BlockInfos) -> bool {
        let event = self.event_timeline.peek().unwrap();
        let mut time = event.time;

        // `set_value` and implicitly inserted events are inserted with `time = 0.`,
        // so that we ensure they are processed first. Replacing their `time` with
        // `block_time` allows to conform to the spec:
        // cf. https://www.w3.org/TR/webaudio/#dom-audioparam-value
        // cf. https://www.w3.org/TR/webaudio/#dom-audioparam-linearramptovalueattime
        // cf. https://www.w3.org/TR/webaudio/#dom-audioparam-exponentialramptovalueattime
        if time == 0. {
            time = infos.block_time;
        }

        // fill buffer with current intrinsic value until `event.time`
        if infos.is_a_rate {
            let end_index = ((time - infos.block_time).max(0.) / infos.dt).round() as usize;
            let end_index_clipped = end_index.min(infos.count);

            for _ in self.buffer.len()..end_index_clipped {
                self.buffer.push(self.intrinsic_value);
            }
        }

        // event time is not reached within this block, we are done
        if time > infos.next_block_time {
            return true;
        }

        // else update `intrisic_value` and `last_event` and continue the loop
        self.intrinsic_value = event.value;
        if !infos.is_a_rate && time <= infos.block_time {
            // A step at the quantum boundary is already effective for k-rate.
            // Steps later in this quantum only affect its successor.
            self.buffer[0] = event.value;
        }

        // no computation has been done on `time`
        #[allow(clippy::float_cmp)]
        if time != event.time {
            let mut event = self.event_timeline.pop().unwrap();
            event.time = time;
            self.last_event = Some(event);
        } else {
            self.last_event = self.event_timeline.pop();
        }

        false
    }

    // cf. https://www.w3.org/TR/webaudio/#dom-audioparam-linearramptovalueattime
    // 𝑣(𝑡) = 𝑉0 + (𝑉1−𝑉0) * ((𝑡−𝑇0) / (𝑇1−𝑇0))
    fn compute_linear_ramp_automation(&mut self, infos: &BlockInfos) -> bool {
        let event = self.event_timeline.peek().unwrap();
        let last_event = self.last_event.as_ref().unwrap();

        let start_time = last_event.time;
        let mut end_time = event.time;
        // compute duration before clapping `end_time` to`cancel_time`, to keep
        // declared slope of the ramp consistent
        let duration = end_time - start_time;
        if let Some(cancel_time) = event.cancel_time {
            end_time = cancel_time;
        }

        let start_value = last_event.value;
        let end_value = event.value;
        let diff = end_value - start_value;

        if infos.is_a_rate {
            let start_index = self.buffer.len();
            // TODO use ceil() or round() when `end_time` is between two samples?
            // <https://github.com/orottier/web-audio-api-rs/pull/460>
            let end_index = ((end_time - infos.block_time).max(0.) / infos.dt).round() as usize;
            let end_index_clipped = end_index.min(infos.count);

            // compute "real" value according to `t` then clamp it
            // cf. Example 7 https://www.w3.org/TR/webaudio/#computation-of-value
            if end_index_clipped > start_index {
                let mut time = (start_index as f64).mul_add(infos.dt, infos.block_time);

                let mut value = 0.;
                for _ in start_index..end_index_clipped {
                    value =
                        compute_linear_ramp_sample(start_time, duration, start_value, diff, time);
                    self.buffer.push(value);
                    time += infos.dt;
                }
                self.intrinsic_value = value;
            }
        }

        // Event will continue in next tick:
        // compute value for `next_block_time` so that `param.value()`
        // stays coherent, also allows to properly fill k-rate
        // within next block too
        if end_time >= infos.next_block_time {
            let value = compute_linear_ramp_sample(
                start_time,
                duration,
                start_value,
                diff,
                infos.next_block_time,
            );
            self.intrinsic_value = value;

            return true;
        }

        // Event cancelled during this block
        if event.cancel_time.is_some() {
            let value =
                compute_linear_ramp_sample(start_time, duration, start_value, diff, end_time);

            self.intrinsic_value = value;

            let mut last_event = self.event_timeline.pop().unwrap();
            last_event.time = end_time;
            last_event.value = value;
            self.last_event = Some(last_event);
        // Event ended during this block
        } else {
            self.intrinsic_value = end_value;
            self.last_event = self.event_timeline.pop();
        }

        false
    }

    // cf. https://www.w3.org/TR/webaudio/#dom-audioparam-exponentialramptovalueattime
    // v(t) = v1 * (v2/v1)^((t-t1) / (t2-t1))
    fn compute_exponential_ramp_automation(&mut self, infos: &BlockInfos) -> bool {
        let event = self.event_timeline.peek().unwrap();
        let last_event = self.last_event.as_ref().unwrap();

        let start_time = last_event.time;
        let mut end_time = event.time;
        // compute duration before clapping `end_time` to `cancel_time`, to keep
        // declared slope of the ramp consistent
        let duration = end_time - start_time;
        if let Some(cancel_time) = event.cancel_time {
            end_time = cancel_time;
        }

        let start_value = last_event.value;
        let end_value = event.value;
        let ratio = end_value / start_value;

        // Handle edge cases:
        // > If 𝑉0 and 𝑉1 have opposite signs or if 𝑉0 is zero,
        // > then 𝑣(𝑡)=𝑉0 for 𝑇0≤𝑡<𝑇1.
        // as:
        // > If there are no more events after this ExponentialRampToValue
        // > event then for 𝑡≥𝑇1, 𝑣(𝑡)=𝑉1.
        // this should thus behave as a SetValue
        if start_value == 0. || start_value * end_value < 0. {
            let event = AudioParamEvent {
                event_type: AudioParamEventType::SetValueAtTime,
                time: end_time,
                value: end_value,
                time_constant: None,
                cancel_time: None,
                duration: None,
                values: None,
            };

            self.event_timeline.replace_peek(event);
            return false;
        }

        if infos.is_a_rate {
            let start_index = self.buffer.len();
            // TODO use ceil() or round() when `end_time` is between two samples?
            // <https://github.com/orottier/web-audio-api-rs/pull/460>
            let end_index = ((end_time - infos.block_time).max(0.) / infos.dt).round() as usize;
            let end_index_clipped = end_index.min(infos.count);

            if end_index_clipped > start_index {
                let mut time = (start_index as f64).mul_add(infos.dt, infos.block_time);

                let mut value = 0.;
                for _ in start_index..end_index_clipped {
                    value = compute_exponential_ramp_sample(
                        start_time,
                        duration,
                        start_value,
                        ratio,
                        time,
                    );

                    self.buffer.push(value);

                    time += infos.dt;
                }
                self.intrinsic_value = value;
            }
        }

        // Event will continue in next tick:
        // compute value for `next_block_time` so that `param.value()`
        // stays coherent, also allows to properly fill k-rate
        // within next block too
        if end_time >= infos.next_block_time {
            let value = compute_exponential_ramp_sample(
                start_time,
                duration,
                start_value,
                ratio,
                infos.next_block_time,
            );
            self.intrinsic_value = value;

            return true;
        }

        // Event cancelled during this block
        if event.cancel_time.is_some() {
            let value =
                compute_exponential_ramp_sample(start_time, duration, start_value, ratio, end_time);

            self.intrinsic_value = value;

            let mut last_event = self.event_timeline.pop().unwrap();
            last_event.time = end_time;
            last_event.value = value;
            self.last_event = Some(last_event);

        // Event ended during this block
        } else {
            self.intrinsic_value = end_value;
            self.last_event = self.event_timeline.pop();
        }

        false
    }

    // https://webaudio.github.io/web-audio-api/#dom-audioparam-settargetattime
    // 𝑣(𝑡) = 𝑉1 + (𝑉0 − 𝑉1) * 𝑒^−((𝑡−𝑇0) / 𝜏)
    // Note that as SetTarget never resolves on an end value, a `SetTarget` event
    // is inserted in the timeline when the value is close enough to the target.
    // cf. `SNAP_TO_TARGET`
    fn compute_set_target_automation(&mut self, infos: &BlockInfos) -> bool {
        let event = self.event_timeline.peek().unwrap();
        let mut end_time = infos.next_block_time;
        let mut ended = false;

        // handle next event stop SetTarget if any
        let some_next_event = self.event_timeline.next();

        if let Some(next_event) = some_next_event {
            match next_event.event_type {
                AudioParamEventType::LinearRampToValueAtTime
                | AudioParamEventType::ExponentialRampToValueAtTime => {
                    // [spec] If the preceding event is a SetTarget
                    // event, 𝑇0 and 𝑉0 are chosen from the current
                    // time and value of SetTarget automation. That
                    // is, if the SetTarget event has not started,
                    // 𝑇0 is the start time of the event, and 𝑉0
                    // is the value just before the SetTarget event
                    // starts. In this case, the LinearRampToValue
                    // event effectively replaces the SetTarget event.
                    // If the SetTarget event has already started,
                    // 𝑇0 is the current context time, and 𝑉0 is
                    // the current SetTarget automation value at time 𝑇0.
                    // In both cases, the automation curve is continuous.
                    end_time = infos.block_time;
                    ended = true;
                }
                _ => {
                    // For all other events, the SetTarget
                    // event ends at the time of the next event.
                    if next_event.time < infos.next_block_time {
                        end_time = next_event.time;
                        ended = true;
                    }
                }
            }
        }

        // handle CancelAndHoldAtTime
        if let Some(cancel_time) = event.cancel_time {
            if cancel_time < infos.next_block_time {
                end_time = cancel_time;
                ended = true;
            }
        }

        let start_time = event.time;
        // if SetTarget is the first event registered, we implicitly
        // insert a SetValue event just before just as for Ramps.
        // Therefore we are sure last_event exists
        let start_value = self.last_event.as_ref().unwrap().value;
        let end_value = event.value;
        let diff = start_value - end_value;
        let time_constant = event.time_constant.unwrap();

        if infos.is_a_rate {
            let start_index = self.buffer.len();
            // TODO use ceil() or round() when `end_time` is between two samples?
            // <https://github.com/orottier/web-audio-api-rs/pull/460>
            let end_index = ((end_time - infos.block_time).max(0.) / infos.dt).round() as usize;
            let end_index_clipped = end_index.min(infos.count);

            if end_index_clipped > start_index {
                let mut time = (start_index as f64).mul_add(infos.dt, infos.block_time);

                let mut value = 0.;
                for _ in start_index..end_index_clipped {
                    // check if we have reached start_time
                    value = if time - start_time < 0. {
                        self.intrinsic_value
                    } else {
                        compute_set_target_sample(start_time, time_constant, end_value, diff, time)
                    };

                    self.buffer.push(value);
                    time += infos.dt;
                }
                self.intrinsic_value = value;
            }
        }

        if !ended {
            // compute value for `next_block_time` so that `param.value()`
            // stays coherent (see. comment in `AudioParam`)
            // allows to properly fill k-rate within next block too
            let value = compute_set_target_sample(
                start_time,
                time_constant,
                end_value,
                diff,
                infos.next_block_time,
            );

            let diff = (end_value - value).abs();

            // abort event if diff is below SNAP_TO_TARGET
            if diff < SNAP_TO_TARGET {
                self.intrinsic_value = end_value;

                // if end_value is zero, the buffer might contain
                // subnormals, we need to check that and flush to zero
                if end_value == 0. {
                    for v in self.buffer.iter_mut() {
                        if v.is_subnormal() {
                            *v = 0.;
                        }
                    }
                }

                let event = AudioParamEvent {
                    event_type: AudioParamEventType::SetValueAtTime,
                    time: infos.next_block_time,
                    value: end_value,
                    time_constant: None,
                    cancel_time: None,
                    duration: None,
                    values: None,
                };

                self.event_timeline.replace_peek(event);
            } else {
                self.intrinsic_value = value;
            }

            return true;
        }

        // setTarget has no "real" end value, compute according
        // to next event start time
        let value = compute_set_target_sample(start_time, time_constant, end_value, diff, end_time);

        self.intrinsic_value = value;
        // end_value and end_time must be stored for use
        // as start time by next event
        let mut event = self.event_timeline.pop().unwrap();
        event.time = end_time;
        event.value = value;
        self.last_event = Some(event);

        false
    }

    fn compute_set_value_curve_automation(&mut self, infos: &BlockInfos) -> bool {
        let event = self.event_timeline.peek().unwrap();
        let start_time = event.time;
        let duration = event.duration.unwrap();
        let values = event.values.as_ref().unwrap();
        let mut end_time = start_time + duration;

        // we must check for the cancel event after we have
        // the "real" duration computed to not change the
        // slope of the ramp
        if let Some(cancel_time) = event.cancel_time {
            end_time = cancel_time;
        }

        if infos.is_a_rate {
            let start_index = self.buffer.len();
            // TODO use ceil() or round() when `end_time` is between two samples?
            // <https://github.com/orottier/web-audio-api-rs/pull/460>
            let end_index = ((end_time - infos.block_time).max(0.) / infos.dt).round() as usize;
            let end_index_clipped = end_index.min(infos.count);

            if end_index_clipped > start_index {
                let mut time = (start_index as f64).mul_add(infos.dt, infos.block_time);

                let mut value = 0.;
                for _ in start_index..end_index_clipped {
                    // check if we have reached start_time
                    value = if time < start_time {
                        self.intrinsic_value
                    } else {
                        compute_set_value_curve_sample(start_time, duration, values, time)
                    };

                    self.buffer.push(value);

                    time += infos.dt;
                }
                self.intrinsic_value = value;
            }
        }

        // event will continue in next tick
        if end_time >= infos.next_block_time {
            // compute value for `next_block_time` so that `param.value()`
            // stays coherent (see. comment in `AudioParam`)
            // allows to properly fill k-rate within next block too
            let value =
                compute_set_value_curve_sample(start_time, duration, values, infos.next_block_time);
            self.intrinsic_value = value;

            return true;
        }

        // event has been cancelled
        if event.cancel_time.is_some() {
            let value = compute_set_value_curve_sample(start_time, duration, values, end_time);

            self.intrinsic_value = value;

            let mut last_event = self.event_timeline.pop().unwrap();
            last_event.time = end_time;
            last_event.value = value;
            self.last_event = Some(last_event);
        // event has ended
        } else {
            let value = values[values.len() - 1];

            let mut last_event = self.event_timeline.pop().unwrap();
            last_event.time = end_time;
            last_event.value = value;

            self.intrinsic_value = value;
            self.last_event = Some(last_event);
        }

        false
    }

    fn compute_buffer(&mut self, block_time: f64, dt: f64, count: usize) {
        // Set [[current value]] to the value of paramIntrinsicValue at the
        // beginning of this render quantum.
        let clamped = self.intrinsic_value.clamp(self.min_value, self.max_value);
        self.current_value.store(clamped, Ordering::Release);

        // clear the buffer for this block
        self.buffer.clear();

        let is_a_rate = self.automation_rate.is_a_rate();
        let next_block_time = dt.mul_add(count as f64, block_time);

        // Check if we can safely return a buffer of length 1 even for a-rate params.
        // Several cases allow us to do so:
        // - The timeline is empty
        // - The timeline is not empty: in such case if `event.time >= next_block_time`
        //   AND `event_type` is not `LinearRampToValueAtTime` or `ExponentialRampToValueAtTime`
        //   this is safe, i.e.:
        //   + For linear and exponential ramps `event.time` is the end time while their
        //   start time is `last_event.time`, therefore if `peek()` is of these
        //   types, we are in the middle of the ramp.
        //   + For all other event, `event.time` is their start time.
        //   (@note - `SetTargetAtTime` events also uses `last_event` but only for
        //   its value, not for its timing information, so no problem there)
        let is_constant_block = match self.event_timeline.peek() {
            None => true,
            Some(event) => {
                if event.event_type != AudioParamEventType::LinearRampToValueAtTime
                    && event.event_type != AudioParamEventType::ExponentialRampToValueAtTime
                {
                    event.time >= next_block_time
                } else {
                    false
                }
            }
        };

        if !is_a_rate || is_constant_block {
            self.buffer.push(self.intrinsic_value);
            // nothing to compute in timeline, for both k-rate and a-rate
            if is_constant_block {
                return;
            }
        }

        // pack block infos for automation methods
        let block_infos = BlockInfos {
            block_time,
            dt,
            count,
            is_a_rate,
            next_block_time,
        };

        loop {
            let next_event_type = self.event_timeline.peek().map(|e| e.event_type);

            let exit_loop = match next_event_type {
                None => {
                    if is_a_rate {
                        // we use `count` rather then `buffer.remaining_capacity`
                        // to correctly handle unit tests where `count` is lower than
                        // RENDER_QUANTUM_SIZE
                        for _ in self.buffer.len()..count {
                            self.buffer.push(self.intrinsic_value);
                        }
                    }
                    true
                }
                Some(AudioParamEventType::SetValue) | Some(AudioParamEventType::SetValueAtTime) => {
                    self.compute_set_value_automation(&block_infos)
                }
                Some(AudioParamEventType::LinearRampToValueAtTime) => {
                    self.compute_linear_ramp_automation(&block_infos)
                }
                Some(AudioParamEventType::ExponentialRampToValueAtTime) => {
                    self.compute_exponential_ramp_automation(&block_infos)
                }
                Some(AudioParamEventType::SetTargetAtTime) => {
                    self.compute_set_target_automation(&block_infos)
                }
                Some(AudioParamEventType::SetValueCurveAtTime) => {
                    self.compute_set_value_curve_automation(&block_infos)
                }
                _ => panic!(
                    "AudioParamEvent {:?} should not appear in AudioParamEventTimeline",
                    next_event_type.unwrap()
                ),
            };

            if exit_loop {
                break;
            }
        }
        // Publish the value after applying events effective at this boundary.
        let current = self.buffer[0].clamp(self.min_value, self.max_value);
        self.current_value.store(current, Ordering::Release);
    }
}

pub(crate) fn audio_param_pair(
    descriptor: AudioParamDescriptor,
    registration: AudioContextRegistration,
) -> (AudioParam, AudioParamProcessor) {
    let AudioParamDescriptor {
        automation_rate,
        default_value,
        max_value,
        min_value,
        ..
    } = descriptor;

    assert_is_finite(default_value);
    assert_is_finite(min_value);
    assert_is_finite(max_value);
    assert!(
        min_value <= default_value,
        "InvalidStateError - AudioParam minValue should be <= defaultValue"
    );
    assert!(
        default_value <= max_value,
        "InvalidStateError - AudioParam defaultValue should be <= maxValue"
    );

    let current_value = Arc::new(AtomicF32::new(default_value));

    let param = AudioParam {
        registration: registration.into(),
        raw_parts: AudioParamInner {
            default_value,
            max_value,
            min_value,
            automation_rate_constrained: false,
            automation_rate: Arc::new(Mutex::new(automation_rate)),
            current_value: Arc::clone(&current_value),
        },
    };

    let processor = AudioParamProcessor {
        intrinsic_value: default_value,
        current_value,
        default_value,
        min_value,
        max_value,
        automation_rate,
        event_timeline: AudioParamEventTimeline::new(),
        last_event: None,
        buffer: ArrayVec::new(),
    };

    (param, processor)
}

#[cfg(test)]
mod tests {
    use float_eq::assert_float_eq;

    use crate::context::{BaseAudioContext, OfflineAudioContext};
    use crate::render::Alloc;

    use super::*;

    #[test]
    #[should_panic]
    fn test_assert_strictly_positive_fail() {
        assert_strictly_positive(0.);
    }

    #[test]
    fn test_assert_strictly_positive() {
        assert_strictly_positive(0.1);
    }

    #[test]
    #[should_panic]
    fn test_assert_not_zero_fail() {
        assert_not_zero(0.);
    }

    #[test]
    fn test_assert_not_zero() {
        assert_not_zero(-0.1);
        assert_not_zero(0.1);
    }

    #[test]
    #[should_panic]
    fn test_assert_sequence_length_fail() {
        assert_sequence_length(&[0.; 1]);
    }

    #[test]
    fn test_assert_sequence_length() {
        assert_sequence_length(&[0.; 2]);
    }

    #[test]
    fn test_default_and_accessors() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 0.,
            min_value: -10.,
            max_value: 10.,
        };
        let (param, _render) = audio_param_pair(opts, context.mock_registration());

        assert_eq!(param.automation_rate(), AutomationRate::A);
        assert_float_eq!(param.default_value(), 0., abs_all <= 0.);
        assert_float_eq!(param.min_value(), -10., abs_all <= 0.);
        assert_float_eq!(param.max_value(), 10., abs_all <= 0.);
        assert_float_eq!(param.value(), 0., abs_all <= 0.);
    }

    #[test]
    fn test_automation_rate_synchronicity_on_control_thread() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 0.,
            min_value: 0.,
            max_value: 1.,
        };
        let (param, _render) = audio_param_pair(opts, context.mock_registration());

        param.set_automation_rate(AutomationRate::K);
        assert_eq!(param.automation_rate(), AutomationRate::K);
    }

    #[test]
    fn test_audioparam_clones_in_sync() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 0.,
            min_value: -10.,
            max_value: 10.,
        };
        let (param1, mut render) = audio_param_pair(opts, context.mock_registration());
        let param2 = param1.clone();

        // changing automation rate on param1 should reflect in param2
        param1.set_automation_rate(AutomationRate::K);
        assert_eq!(param2.automation_rate(), AutomationRate::K);

        // setting value on param1 should reflect in param2
        render.handle_incoming_event(param1.set_value_raw(2.));
        assert_float_eq!(param1.value(), 2., abs_all <= 0.);
        assert_float_eq!(param2.value(), 2., abs_all <= 0.);

        // setting value on param2 should reflect in param1
        render.handle_incoming_event(param2.set_value_raw(3.));
        assert_float_eq!(param1.value(), 3., abs_all <= 0.);
        assert_float_eq!(param2.value(), 3., abs_all <= 0.);
    }

    #[test]
    fn test_quantum_boundary_step_updates_value_before_krate_sampling() {
        for automation_rate in [AutomationRate::A, AutomationRate::K] {
            let context = OfflineAudioContext::new(1, 1, 48000.);
            let descriptor = AudioParamDescriptor {
                name: String::new(),
                automation_rate,
                default_value: 0.,
                min_value: -10.,
                max_value: 10.,
            };
            let (param, mut render) = audio_param_pair(descriptor, context.mock_registration());
            render.handle_incoming_event(param.set_value_at_time_raw(1., 0.));
            render.handle_incoming_event(param.set_value_at_time_raw(2., 5.));
            assert_eq!(param.value(), 0.);
            let values = render.compute_intrinsic_values(0., 1., 10);
            if automation_rate == AutomationRate::A {
                assert_eq!(values, &[1., 1., 1., 1., 1., 2., 2., 2., 2., 2.]);
            } else {
                assert_eq!(values, &[1.]);
            }
            assert_eq!(param.value(), 1.);
            assert_eq!(render.compute_intrinsic_values(10., 1., 10), &[2.]);
            assert_eq!(param.value(), 2.);
        }
    }

    #[test]
    fn test_set_value() {
        {
            let context = OfflineAudioContext::new(1, 1, 48000.);

            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: -10.,
                max_value: 10.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            render.handle_incoming_event(param.set_value_raw(2.));

            assert_float_eq!(param.value(), 2., abs_all <= 0.);

            let vs = render.compute_intrinsic_values(0., 1., 10);

            assert_float_eq!(param.value(), 2., abs_all <= 0.);
            assert_float_eq!(vs, &[2.; 10][..], abs_all <= 0.);
        }

        // make sure param.value() is properly clamped
        {
            let context = OfflineAudioContext::new(1, 1, 48000.);

            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 1.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            render.handle_incoming_event(param.set_value_raw(2.));

            assert_float_eq!(param.value(), 1., abs_all <= 0.);

            let vs = render.compute_intrinsic_values(0., 1., 10);

            // value should clamped while intrinsic value should not
            assert_float_eq!(param.value(), 1., abs_all <= 0.);
            assert_float_eq!(vs, &[2.; 10][..], abs_all <= 0.);
        }
    }

    #[test]
    fn test_steps_a_rate() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        {
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: -10.,
                max_value: 10.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            render.handle_incoming_event(param.set_value_at_time_raw(5., 2.0));
            render.handle_incoming_event(param.set_value_at_time_raw(12., 8.0)); // should clamp
            render.handle_incoming_event(param.set_value_at_time_raw(8., 10.0)); // should not occur 1st run

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(
                vs,
                &[0., 0., 5., 5., 5., 5., 5., 5., 12., 12.][..],
                abs_all <= 0.
            );

            // no event left in timeline, i.e. length is 1
            let vs = render.compute_intrinsic_values(10., 1., 10);
            assert_float_eq!(vs, &[8.; 1][..], abs_all <= 0.);
        }

        {
            // events spread on several blocks
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: -10.,
                max_value: 10.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            render.handle_incoming_event(param.set_value_at_time_raw(5., 2.0));
            render.handle_incoming_event(param.set_value_at_time_raw(8., 12.0));

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(
                vs,
                &[0., 0., 5., 5., 5., 5., 5., 5., 5., 5.][..],
                abs_all <= 0.
            );

            let vs = render.compute_intrinsic_values(10., 1., 10);
            assert_float_eq!(
                vs,
                &[5., 5., 8., 8., 8., 8., 8., 8., 8., 8.][..],
                abs_all <= 0.
            );
        }
    }

    #[test]
    fn test_steps_k_rate() {
        let context = OfflineAudioContext::new(1, 1, 48000.);
        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::K,
            default_value: 0.,
            min_value: -10.,
            max_value: 10.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());

        render.handle_incoming_event(param.set_value_at_time_raw(5., 2.0));
        render.handle_incoming_event(param.set_value_at_time_raw(12., 8.0)); // should not appear in results
        render.handle_incoming_event(param.set_value_at_time_raw(8., 10.0)); // should not occur 1st run
        render.handle_incoming_event(param.set_value_at_time_raw(3., 14.0)); // should appear in 3rd run

        let vs = render.compute_intrinsic_values(0., 1., 10);
        assert_float_eq!(vs, &[0.; 1][..], abs_all <= 0.);

        let vs = render.compute_intrinsic_values(10., 1., 10);
        assert_float_eq!(vs, &[8.; 1][..], abs_all <= 0.);

        let vs = render.compute_intrinsic_values(20., 1., 10);
        assert_float_eq!(vs, &[3.; 1][..], abs_all <= 0.);
    }

    #[test]
    fn test_linear_ramp_arate() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 0.,
            min_value: -10.,
            max_value: 10.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());

        // set to 5 at t = 2
        render.handle_incoming_event(param.set_value_at_time_raw(5., 2.0));
        // ramp to 8 from t = 2 to t = 5
        render.handle_incoming_event(param.linear_ramp_to_value_at_time_raw(8.0, 5.0));
        // ramp to 0 from t = 5 to t = 13
        render.handle_incoming_event(param.linear_ramp_to_value_at_time_raw(0., 13.0));

        let vs = render.compute_intrinsic_values(0., 1., 10);
        assert_float_eq!(
            vs,
            &[0., 0., 5., 6., 7., 8., 7., 6., 5., 4.][..],
            abs_all <= 0.
        );
    }

    #[test]
    fn test_linear_ramp_arate_end_of_block() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 0.,
            min_value: -10.,
            max_value: 10.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());

        // set to 0 at t = 0
        render.handle_incoming_event(param.set_value_at_time_raw(0., 0.));
        // ramp to 9 from t = 0 to t = 9
        render.handle_incoming_event(param.linear_ramp_to_value_at_time_raw(9.0, 9.0));

        let vs = render.compute_intrinsic_values(0., 1., 10);
        assert_float_eq!(
            vs,
            &[0., 1., 2., 3., 4., 5., 6., 7., 8., 9.][..],
            abs_all <= 0.
        );
    }

    #[test]
    // @note - with real params, a set_value event is always used to provide
    // init value to the param. Therefore this test is purely theoretical and
    // in real world the param would not behave like that, which is wrong
    // @todo - open an issue to review how init value is passed (or how last_event is set)
    fn test_linear_ramp_arate_implicit_set_value() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 0.,
            min_value: -10.,
            max_value: 10.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());

        // mimic a ramp inserted after start
        // i.e. setTimeout(() => param.linearRampToValueAtTime(10, now + 10)), 10 * 1000);

        // no event in timeline here, i.e. length is 1
        let vs = render.compute_intrinsic_values(0., 1., 10);
        assert_float_eq!(vs, &[0.; 1][..], abs_all <= 0.);

        // implicitly insert a SetValue event at time 10
        render.handle_incoming_event(param.linear_ramp_to_value_at_time_raw(10.0, 20.0));

        let vs = render.compute_intrinsic_values(10., 1., 10);
        assert_float_eq!(
            vs,
            &[0., 1., 2., 3., 4., 5., 6., 7., 8., 9.][..],
            abs_all <= 0.
        );

        // ramp finishes on first value of this block, i.e. length is 10
        let vs = render.compute_intrinsic_values(20., 1., 10);
        assert_float_eq!(vs, &[10.; 10][..], abs_all <= 0.);
    }

    #[test]
    fn test_linear_ramp_arate_multiple_blocks() {
        // regression test for issue #9
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 0.,
            min_value: -20.,
            max_value: 20.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());

        // ramp to 20 from t = 0 to t = 20
        render.handle_incoming_event(param.linear_ramp_to_value_at_time_raw(20.0, 20.0));

        // first quantum t = 0..10
        let vs = render.compute_intrinsic_values(0., 1., 10);
        assert_float_eq!(
            vs,
            &[0., 1., 2., 3., 4., 5., 6., 7., 8., 9.][..],
            abs_all <= 0.
        );
        assert_float_eq!(param.value(), 0., abs <= 0.);

        // next quantum t = 10..20
        let vs = render.compute_intrinsic_values(10., 1., 10);
        assert_float_eq!(
            vs,
            &[10., 11., 12., 13., 14., 15., 16., 17., 18., 19.][..],
            abs_all <= 0.
        );
        assert_float_eq!(param.value(), 10., abs <= 0.);

        // ramp finished t = 20..30
        let vs = render.compute_intrinsic_values(20., 1., 10);
        assert_float_eq!(vs, &[20.0; 10][..], abs_all <= 0.);
        assert_float_eq!(param.value(), 20., abs <= 0.);
    }

    #[test]
    fn test_linear_ramp_krate_multiple_blocks() {
        // regression test for issue #9
        let context = OfflineAudioContext::new(1, 1, 48000.);

        {
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::K,
                default_value: 0.,
                min_value: -20.,
                max_value: 20.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            // ramp to 20 from t = 0 to t = 20
            render.handle_incoming_event(param.linear_ramp_to_value_at_time_raw(20.0, 20.0));
            // first quantum t = 0..10
            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(vs, &[0.; 1][..], abs_all <= 0.);
            assert_float_eq!(param.value(), 0., abs <= 0.);
            // next quantum t = 10..20
            let vs = render.compute_intrinsic_values(10., 1., 10);
            assert_float_eq!(vs, &[10.; 1][..], abs_all <= 0.);
            assert_float_eq!(param.value(), 10., abs <= 0.);
            // ramp finished t = 20..30
            let vs = render.compute_intrinsic_values(20., 1., 10);
            assert_float_eq!(vs, &[20.0; 1][..], abs_all <= 0.);
            assert_float_eq!(param.value(), 20., abs <= 0.);
        }

        {
            // finish in the middle of a block
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::K,
                default_value: 0.,
                min_value: -20.,
                max_value: 20.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            // ramp to 20 from t = 0 to t = 20
            render.handle_incoming_event(param.linear_ramp_to_value_at_time_raw(15.0, 15.0));
            // first quantum t = 0..10
            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(vs, &[0.; 1][..], abs_all <= 0.);
            assert_float_eq!(param.value(), 0., abs <= 0.);
            // next quantum t = 10..20
            let vs = render.compute_intrinsic_values(10., 1., 10);
            assert_float_eq!(vs, &[10.; 1][..], abs_all <= 0.);
            assert_float_eq!(param.value(), 10., abs <= 0.);
            // ramp finished t = 20..30
            let vs = render.compute_intrinsic_values(20., 1., 10);
            assert_float_eq!(vs, &[15.0; 1][..], abs_all <= 0.);
            assert_float_eq!(param.value(), 15., abs <= 0.);
        }
    }

    #[test]
    fn test_linear_ramp_start_time() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 0.,
            min_value: -10.,
            max_value: 10.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());

        render.handle_incoming_event(param.set_value_at_time_raw(1., 0.));
        render.handle_incoming_event(param.linear_ramp_to_value_at_time_raw(-1., 10.));
        let vs = render.compute_intrinsic_values(0., 1., 10);
        assert_float_eq!(
            vs,
            &[1., 0.8, 0.6, 0.4, 0.2, 0., -0.2, -0.4, -0.6, -0.8][..],
            abs_all <= 1e-7
        );

        let vs = render.compute_intrinsic_values(10., 1., 10);
        assert_float_eq!(vs, &[-1.; 10][..], abs_all <= 0.);

        // start time should be end time of last event, i.e. 10.
        render.handle_incoming_event(param.linear_ramp_to_value_at_time_raw(1., 30.));

        let vs = render.compute_intrinsic_values(20., 1., 10);
        assert_float_eq!(
            vs,
            &[0., 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9][..],
            abs_all <= 1e-7
        );
    }

    #[test]
    fn test_exponential_ramp_a_rate() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 0.,
            min_value: 0.,
            max_value: 1.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());

        // set to 0.0001 at t=0 (0. is a special case)
        render.handle_incoming_event(param.set_value_at_time_raw(0.0001, 0.));
        // ramp to 1 from t = 0 to t = 10
        render.handle_incoming_event(param.exponential_ramp_to_value_at_time_raw(1.0, 10.));

        // compute resulting buffer:
        // v(t) = v1*(v2/v1)^((t-t1)/(t2-t1))
        let mut res = Vec::<f32>::with_capacity(10);
        let start: f32 = 0.0001;
        let end: f32 = 1.;

        for t in 0..10 {
            let value = start * (end / start).powf(t as f32 / 10.);
            res.push(value);
        }

        let vs = render.compute_intrinsic_values(0., 1., 10);
        assert_float_eq!(vs, &res[..], abs_all <= 0.);

        let vs = render.compute_intrinsic_values(10., 1., 10);
        assert_float_eq!(vs, &[1.0; 10][..], abs_all <= 0.);
    }

    #[test]
    fn test_exponential_ramp_a_rate_multiple_blocks() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 0.,
            min_value: 0.,
            max_value: 1.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());

        let start: f32 = 0.0001; // use 0.0001 as 0. is a special case
        let end: f32 = 1.;
        render.handle_incoming_event(param.set_value_at_time_raw(start, 3.));
        // ramp to 1 from t = 3. to t = 13.
        render.handle_incoming_event(param.exponential_ramp_to_value_at_time_raw(end, 13.));

        // compute resulting buffer:
        let mut res = vec![0.; 3];
        // set_value is implicit here as this is the first value of the computed ramp
        // exponential ramp (v(t) = v1*(v2/v1)^((t-t1)/(t2-t1)))
        for t in 0..10 {
            let value = start * (end / start).powf(t as f32 / 10.);
            res.push(value);
        }
        // fill remaining with target value
        res.append(&mut vec![1.; 7]);

        let vs = render.compute_intrinsic_values(0., 1., 10);
        assert_float_eq!(vs, &res[0..10], abs_all <= 0.);
        assert_float_eq!(param.value(), res[0], abs <= 0.);

        let vs = render.compute_intrinsic_values(10., 1., 10);
        assert_float_eq!(vs, &res[10..20], abs_all <= 0.);
        assert_float_eq!(param.value(), res[10], abs <= 0.);
    }

    #[test]
    fn test_exponential_ramp_a_rate_zero_and_opposite_target() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        {
            // zero target
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 1.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            // set v=0. at t=0 (0. is a special case)
            render.handle_incoming_event(param.set_value_at_time_raw(0., 0.));
            // ramp to 1 from t=0 to t=5 -> should behave as a set target at t=5
            render.handle_incoming_event(param.exponential_ramp_to_value_at_time_raw(1.0, 5.));

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(
                vs,
                &[0., 0., 0., 0., 0., 1., 1., 1., 1., 1.][..],
                abs_all <= 0.
            );
        }

        {
            // opposite signs
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: -1.,
                max_value: 1.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            // set v=-1. at t=0
            render.handle_incoming_event(param.set_value_at_time_raw(-1., 0.));
            // ramp to 1 from t=0 to t=5 -> should behave as a set target at t=5
            render.handle_incoming_event(param.exponential_ramp_to_value_at_time_raw(1.0, 5.));

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(
                vs,
                &[-1., -1., -1., -1., -1., 1., 1., 1., 1., 1.][..],
                abs_all <= 0.
            );
        }
    }

    #[test]
    #[should_panic]
    fn test_exponential_ramp_to_zero() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 1.,
            min_value: 0.,
            max_value: 1.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());
        render.handle_incoming_event(param.exponential_ramp_to_value_at_time_raw(0.0, 10.));
    }

    #[test]
    fn test_exponential_ramp_k_rate_multiple_blocks() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::K,
            default_value: 0.,
            min_value: 0.,
            max_value: 1.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());

        let start: f32 = 0.0001; // use 0.0001 as 0. is a special case
        let end: f32 = 1.;
        render.handle_incoming_event(param.set_value_at_time_raw(start, 3.));
        // ramp to 1 from t = 3. to t = 13.
        render.handle_incoming_event(param.exponential_ramp_to_value_at_time_raw(end, 13.));

        // compute resulting buffer:
        let mut res = vec![0.; 3];
        // set_value is implicit here as this is the first value of the computed ramp
        // exponential ramp (v(t) = v1*(v2/v1)^((t-t1)/(t2-t1)))
        for t in 0..10 {
            let value = start * (end / start).powf(t as f32 / 10.);
            res.push(value);
        }
        // fill remaining with target value
        res.append(&mut vec![1.; 7]);

        // recreate k-rate blocks from computed values
        let vs = render.compute_intrinsic_values(0., 1., 10);
        assert_float_eq!(vs, &[res[0]; 1][..], abs_all <= 0.);

        let vs = render.compute_intrinsic_values(10., 1., 10);
        assert_float_eq!(vs, &[res[10]; 1][..], abs_all <= 0.);

        let vs = render.compute_intrinsic_values(20., 1., 10);
        assert_float_eq!(vs, &[1.; 1][..], abs_all <= 0.);
    }

    #[test]
    fn test_exponential_ramp_k_rate_zero_and_opposite_target() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        {
            // zero target
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::K,
                default_value: 0.,
                min_value: 0.,
                max_value: 1.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            // ramp to 1 from t=0 to t=5 -> should behave as a set target at t=5
            render.handle_incoming_event(param.exponential_ramp_to_value_at_time_raw(1.0, 5.));

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(vs, &[0.; 1][..], abs_all <= 0.);

            let vs = render.compute_intrinsic_values(10., 1., 10);
            assert_float_eq!(vs, &[1.; 1][..], abs_all <= 0.);
        }

        {
            // opposite signs
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::K,
                default_value: -1.,
                min_value: -1.,
                max_value: 1.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            // ramp to 1 from t=0 to t=5 -> should behave as a set target at t=5
            render.handle_incoming_event(param.exponential_ramp_to_value_at_time_raw(1.0, 5.));

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(vs, &[-1.; 1][..], abs_all <= 0.);

            let vs = render.compute_intrinsic_values(10., 1., 10);
            assert_float_eq!(vs, &[1.; 1][..], abs_all <= 0.);
        }
    }

    #[test]
    fn test_exponential_ramp_start_time() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 0.,
            min_value: -10.,
            max_value: 10.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());

        render.handle_incoming_event(param.set_value_at_time_raw(0., 0.));
        render.handle_incoming_event(param.linear_ramp_to_value_at_time_raw(1., 10.));

        let vs = render.compute_intrinsic_values(0., 1., 10);
        assert_float_eq!(
            vs,
            &[0., 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9][..],
            abs_all <= 1e-7
        );

        let vs = render.compute_intrinsic_values(10., 1., 10);
        assert_float_eq!(vs, &[1.; 10][..], abs_all <= 0.);

        // start time should be end time of last event, i.e. 10.
        render.handle_incoming_event(param.exponential_ramp_to_value_at_time_raw(0.0001, 30.));
        let vs = render.compute_intrinsic_values(20., 1., 10);
        // compute expected on 20 samples, the 10 last ones should be in vs
        let start: f32 = 1.;
        let end: f32 = 0.0001;
        let mut res = [0.; 20];
        for (t, v) in res.iter_mut().enumerate() {
            *v = start * (end / start).powf(t as f32 / 20.);
        }

        assert_float_eq!(vs, &res[10..], abs_all <= 1e-7);
    }

    #[test]
    fn test_set_target_at_time_a_rate() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        {
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 1.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());
            // 𝑣(𝑡) = 𝑉1 + (𝑉0 − 𝑉1) * 𝑒^−((𝑡−𝑇0) / 𝜏)
            let v0: f32 = 0.;
            let v1: f32 = 1.;
            let t0: f64 = 0.;
            let time_constant: f64 = 1.;

            render.handle_incoming_event(param.set_value_at_time_raw(v0, t0));
            render.handle_incoming_event(param.set_target_at_time_raw(v1, t0, time_constant));
            let vs = render.compute_intrinsic_values(0., 1., 10);

            let mut res = Vec::<f32>::with_capacity(10);
            for t in 0..10 {
                let val = v1 + (v0 - v1) * (-((t as f64 - t0) / time_constant)).exp() as f32;
                res.push(val);
            }

            assert_float_eq!(vs, &res[..], abs_all <= 0.);
        }

        {
            // implicit SetValue if SetTarget is first event
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 1.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());
            // 𝑣(𝑡) = 𝑉1 + (𝑉0 − 𝑉1) * 𝑒^−((𝑡−𝑇0) / 𝜏)
            let v0: f32 = 0.; // will be implicitly set in param (see default_value)
            let v1: f32 = 1.;
            let t0: f64 = 0.;
            let time_constant: f64 = 1.;

            render.handle_incoming_event(param.set_target_at_time_raw(v1, t0, time_constant));
            let vs = render.compute_intrinsic_values(0., 1., 10);

            let mut res = Vec::<f32>::with_capacity(10);
            for t in 0..10 {
                let val = v1 + (v0 - v1) * (-((t as f64 - t0) / time_constant)).exp() as f32;
                res.push(val);
            }

            assert_float_eq!(vs, &res[..], abs_all <= 0.);
        }

        {
            // start later in block with arbitrary values
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 100.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());
            // 𝑣(𝑡) = 𝑉1 + (𝑉0 − 𝑉1) * 𝑒^−((𝑡−𝑇0) / 𝜏)
            let v0: f32 = 1.;
            let v1: f32 = 42.;
            let t0: f64 = 1.;
            let time_constant: f64 = 2.1;

            render.handle_incoming_event(param.set_value_at_time_raw(v0, t0));
            render.handle_incoming_event(param.set_target_at_time_raw(v1, t0, time_constant));

            let mut res = Vec::<f32>::with_capacity(10);
            for t in 0..10 {
                let val = v1 + (v0 - v1) * (-((t as f64 - t0) / time_constant)).exp() as f32;
                res.push(val);
            }
            // start_time is 1.
            res[0] = 0.;

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(vs, &res[..], abs_all <= 0.);
        }

        {
            // handle time_constant == 0.
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 100.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());
            render.handle_incoming_event(param.set_target_at_time_raw(1., 1., 0.));

            let mut res = [1.; 10];
            res[0] = 0.; // start_time is 1.

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(vs, &res[..], abs_all <= 0.);
        }
    }

    #[test]
    fn test_set_target_at_time_a_rate_multiple_blocks() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        {
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 2.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());
            // 𝑣(𝑡) = 𝑉1 + (𝑉0 − 𝑉1) * 𝑒^−((𝑡−𝑇0) / 𝜏)
            let v0: f32 = 0.;
            let v1: f32 = 2.;
            let t0: f64 = 0.;
            let time_constant: f64 = 1.;
            // ramp to 1 from t=0 to t=5 -> should behave as a set target at t=5
            render.handle_incoming_event(param.set_value_at_time_raw(v0, t0));
            render.handle_incoming_event(param.set_target_at_time_raw(v1, t0, time_constant));

            let mut res = Vec::<f32>::with_capacity(20);
            for t in 0..20 {
                let val = v1 + (v0 - v1) * (-((t as f64 - t0) / time_constant)).exp() as f32;
                res.push(val);
            }

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(vs, &res[0..10], abs_all <= 0.);

            let vs = render.compute_intrinsic_values(10., 1., 10);
            assert_float_eq!(vs, &res[10..20], abs_all <= 0.);
        }
    }

    #[test]
    fn test_set_target_at_time_a_rate_followed_by_set_value() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        {
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 2.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());
            // 𝑣(𝑡) = 𝑉1 + (𝑉0 − 𝑉1) * 𝑒^−((𝑡−𝑇0) / 𝜏)
            let v0: f32 = 0.;
            let v1: f32 = 2.;
            let t0: f64 = 0.;
            let time_constant: f64 = 1.;

            render.handle_incoming_event(param.set_value_at_time_raw(v0, t0));
            render.handle_incoming_event(param.set_target_at_time_raw(v1, t0, time_constant));
            render.handle_incoming_event(param.set_value_at_time_raw(0.5, 15.));

            let mut res = Vec::<f32>::with_capacity(20);

            for t in 0..15 {
                let val = v1 + (v0 - v1) * (-((t as f64 - t0) / time_constant)).exp() as f32;
                res.push(val);
            }

            res.resize(20, 0.5);

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(vs, &res[0..10], abs_all <= 0.);

            let vs = render.compute_intrinsic_values(10., 1., 10);
            assert_float_eq!(vs, &res[10..20], abs_all <= 0.);
        }
    }

    #[test]
    fn test_set_target_at_time_ends_at_threshold() {
        let context = OfflineAudioContext::new(1, 1, 48000.);
        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 0.,
            min_value: 0.,
            max_value: 2.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());

        render.handle_incoming_event(param.set_value_at_time_raw(1., 0.));
        render.handle_incoming_event(param.set_target_at_time_raw(0., 1., 0.2));

        let vs = render.compute_intrinsic_values(0., 1., 128);
        for v in vs.iter() {
            assert!(!v.is_subnormal());
        }

        // check peek() has been replaced with set_value event
        let peek = render.event_timeline.peek();
        assert_eq!(
            peek.unwrap().event_type,
            AudioParamEventType::SetValueAtTime
        );

        // this buffer should be filled with target values
        let vs = render.compute_intrinsic_values(10., 1., 128);
        assert_float_eq!(vs[..], [0.; 128], abs_all <= 0.);
    }

    #[test]
    fn test_set_target_at_time_waits_for_start_time() {
        let context = OfflineAudioContext::new(1, 1, 48000.);
        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 0.,
            min_value: 0.,
            max_value: 2.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());

        render.handle_incoming_event(param.set_value_at_time_raw(1., 0.));
        render.handle_incoming_event(param.set_target_at_time_raw(0., 5., 1.));

        let vs = render.compute_intrinsic_values(0., 1., 10);
        assert_float_eq!(vs[0], 1., abs <= 0.);
        assert_float_eq!(vs[1], 1., abs <= 0.);
        assert_float_eq!(vs[2], 1., abs <= 0.);
        assert_float_eq!(vs[3], 1., abs <= 0.);
        assert_float_eq!(vs[4], 1., abs <= 0.);
        assert_float_eq!(vs[5], 1., abs <= 0.);
    }

    #[test]
    fn test_set_target_at_time_a_rate_followed_by_ramp() {
        let context = OfflineAudioContext::new(1, 1, 48000.);
        {
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 10.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());
            // 𝑣(𝑡) = 𝑉1 + (𝑉0 − 𝑉1) * 𝑒^−((𝑡−𝑇0) / 𝜏)
            let v0: f32 = 0.;
            let v1: f32 = 2.;
            let t0: f64 = 0.;
            let time_constant: f64 = 10.;

            render.handle_incoming_event(param.set_value_at_time_raw(v0, t0));
            render.handle_incoming_event(param.set_target_at_time_raw(v1, t0, time_constant));

            let mut res = Vec::<f32>::with_capacity(20);

            for t in 0..11 {
                // we compute the 10th elements as it will be the start value of the ramp
                let val = v1 + (v0 - v1) * (-((t as f64 - t0) / time_constant)).exp() as f32;
                res.push(val);
            }

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(vs, &res[0..10], abs_all <= 0.);

            // ramp
            let v0 = res.pop().unwrap(); // v0 is defined by the SetTarget
            let v1 = 10.;
            let t0 = 10.;
            let t1 = 20.;

            render.handle_incoming_event(param.linear_ramp_to_value_at_time_raw(v1, t1));

            for t in 10..20 {
                let time = t as f64;
                let value = v0 + (v1 - v0) * (time - t0) as f32 / (t1 - t0) as f32;
                res.push(value);
            }

            let vs = render.compute_intrinsic_values(10., 1., 10);
            assert_float_eq!(vs, &res[10..20], abs_all <= 1.0e-6);
            // ramp ended
            let vs = render.compute_intrinsic_values(20., 1., 10);
            assert_float_eq!(vs, &[v1; 10][..], abs_all <= 0.);
        }
    }

    #[test]
    fn test_set_target_at_time_k_rate_multiple_blocks() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        {
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::K,
                default_value: 0.,
                min_value: 0.,
                max_value: 2.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());
            // 𝑣(𝑡) = 𝑉1 + (𝑉0 − 𝑉1) * 𝑒^−((𝑡−𝑇0) / 𝜏)
            let v0: f32 = 0.;
            let v1: f32 = 2.;
            let t0: f64 = 0.;
            let time_constant: f64 = 1.;
            // ramp to 1 from t=0 to t=5 -> should behave as a set target at t=5
            render.handle_incoming_event(param.set_value_at_time_raw(v0, t0));
            render.handle_incoming_event(param.set_target_at_time_raw(v1, t0, time_constant));

            let mut res = Vec::<f32>::with_capacity(20);
            for t in 0..20 {
                let val = v1 + (v0 - v1) * (-((t as f64 - t0) / time_constant)).exp() as f32;
                res.push(val);
            }

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(vs, &[res[0]; 1][..], abs_all <= 0.);

            let vs = render.compute_intrinsic_values(10., 1., 10);
            assert_float_eq!(vs, &[res[10]; 1][..], abs_all <= 0.);
        }
    }

    #[test]
    // regression test for bcebfe6
    fn test_set_target_at_time_snap_to_value() {
        let context = OfflineAudioContext::new(1, 1, 48000.);
        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 0.,
            min_value: 0.,
            max_value: 1.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());
        let v0: f32 = 1.;
        let v1: f32 = 0.;
        let t0: f64 = 0.;
        let time_constant: f64 = 1.;

        render.handle_incoming_event(param.set_value_at_time_raw(v0, t0));
        render.handle_incoming_event(param.set_target_at_time_raw(v1, t0, time_constant));

        let mut res = [0.; 30];
        // 𝑣(𝑡) = 𝑉1 + (𝑉0 − 𝑉1) * 𝑒^−((𝑡−𝑇0) / 𝜏)
        res.iter_mut().enumerate().for_each(|(t, r)| {
            *r = v1 + (v0 - v1) * (-((t as f64 - t0) / time_constant)).exp() as f32;
        });

        let vs = render.compute_intrinsic_values(0., 1., 10);
        assert_float_eq!(vs, &res[..10], abs_all <= 0.);

        let vs = render.compute_intrinsic_values(10., 1., 10);
        assert_float_eq!(vs, &res[10..20], abs_all <= 0.);

        // the distance between the target value and the value just after this block
        // is smaller than SNAP_TO_TARGET (i.e. 1e-10)
        let vs = render.compute_intrinsic_values(20., 1., 10);
        assert_float_eq!(vs, &res[20..30], abs_all <= 0.);

        // then this block should be [0.; 10]
        let vs = render.compute_intrinsic_values(30., 1., 10);
        assert_float_eq!(vs, &[0.; 10][..], abs_all <= 0.);
    }

    #[test]
    fn test_cancel_scheduled_values() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 0.,
            min_value: 0.,
            max_value: 10.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());
        for t in 0..10 {
            render.handle_incoming_event(param.set_value_at_time_raw(t as f32, t as f64));
        }

        render.handle_incoming_event(param.cancel_scheduled_values_raw(5.));

        let vs = render.compute_intrinsic_values(0., 1., 10);
        assert_float_eq!(
            vs,
            &[0., 1., 2., 3., 4., 4., 4., 4., 4., 4.][..],
            abs_all <= 0.
        );
    }

    #[test]
    fn test_cancel_scheduled_values_ramp() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        {
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 10.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            render.handle_incoming_event(param.set_value_at_time_raw(0., 0.));
            render.handle_incoming_event(param.linear_ramp_to_value_at_time_raw(10., 10.));
            // cancels the ramp, the set value event is kept in timeline
            render.handle_incoming_event(param.cancel_scheduled_values_raw(10.));

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(vs, &[0.; 10][..], abs_all <= 0.);
        }

        // ramp already started, go back to previous value
        {
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 20.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            render.handle_incoming_event(param.set_value_at_time_raw(0., 0.));
            render.handle_incoming_event(param.linear_ramp_to_value_at_time_raw(20., 20.));

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(
                vs,
                &[0., 1., 2., 3., 4., 5., 6., 7., 8., 9.][..],
                abs_all <= 0.
            );

            // the SetValue event has been consumed in first tick and the ramp
            // is removed from timeline, no event left in timeline (length is 1)
            render.handle_incoming_event(param.cancel_scheduled_values_raw(10.));

            let vs = render.compute_intrinsic_values(10., 1., 10);
            assert_float_eq!(vs, &[0.; 1][..], abs_all <= 0.);
        }

        // make sure we can't go into a situation where next_event is a ramp
        // and last_event is not defined
        // @see - note in CancelScheduledValues insertion in timeline
        {
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 10.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            // the SetValue from param inserted by the Ramp is left in timeline
            // i.e. length is 10
            render.handle_incoming_event(param.linear_ramp_to_value_at_time_raw(10., 10.));
            render.handle_incoming_event(param.cancel_scheduled_values_raw(10.)); // cancels the ramp

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(vs, &[0.; 10][..], abs_all <= 0.);
        }

        {
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 20.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            render.handle_incoming_event(param.linear_ramp_to_value_at_time_raw(20., 20.));

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(
                vs,
                &[0., 1., 2., 3., 4., 5., 6., 7., 8., 9.][..],
                abs_all <= 0.
            );

            // ramp is removed from timeline, no event left
            render.handle_incoming_event(param.cancel_scheduled_values_raw(10.));

            let vs = render.compute_intrinsic_values(10., 1., 10);
            assert_float_eq!(vs, &[0.; 1][..], abs_all <= 0.);
        }
    }

    #[test]
    fn test_cancel_and_hold() {
        let context = OfflineAudioContext::new(1, 1, 48000.);
        {
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 10.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            render.handle_incoming_event(param.set_value_at_time_raw(1., 1.));
            render.handle_incoming_event(param.set_value_at_time_raw(2., 2.));
            render.handle_incoming_event(param.set_value_at_time_raw(3., 3.));
            render.handle_incoming_event(param.set_value_at_time_raw(4., 4.));
            render.handle_incoming_event(param.cancel_and_hold_at_time_raw(2.5));

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(
                vs,
                &[0., 1., 2., 2., 2., 2., 2., 2., 2., 2.][0..10],
                abs_all <= 0.
            );
        }
    }

    #[test]
    fn test_cancel_and_hold_during_set_target() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        {
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 2.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());
            // 𝑣(𝑡) = 𝑉1 + (𝑉0 − 𝑉1) * 𝑒^−((𝑡−𝑇0) / 𝜏)
            let v0: f32 = 0.;
            let v1: f32 = 2.;
            let t0: f64 = 0.;
            let time_constant: f64 = 1.;

            render.handle_incoming_event(param.set_value_at_time_raw(v0, t0));
            render.handle_incoming_event(param.set_target_at_time_raw(v1, t0, time_constant));
            render.handle_incoming_event(param.cancel_and_hold_at_time_raw(15.));

            let mut res = Vec::<f32>::with_capacity(20);

            // compute index 15 to have hold_value
            for t in 0..16 {
                let val = v1 + (v0 - v1) * (-((t as f64 - t0) / time_constant)).exp() as f32;
                res.push(val);
            }

            let hold_value = res.pop().unwrap();
            res.resize(20, hold_value);

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(vs, &res[0..10], abs_all <= 0.);

            let vs = render.compute_intrinsic_values(10., 1., 10);
            assert_float_eq!(vs, &res[10..20], abs_all <= 0.);
        }
    }

    #[test]
    fn test_cancel_and_hold_during_linear_ramp() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        {
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 10.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            render.handle_incoming_event(param.linear_ramp_to_value_at_time_raw(10., 10.));
            render.handle_incoming_event(param.cancel_and_hold_at_time_raw(5.));

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(
                vs,
                &[0., 1., 2., 3., 4., 5., 5., 5., 5., 5.][0..10],
                abs_all <= 0.
            );
        }

        {
            // cancel between two samples
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 10.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            render.handle_incoming_event(param.linear_ramp_to_value_at_time_raw(10., 10.));
            render.handle_incoming_event(param.cancel_and_hold_at_time_raw(4.5));

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(
                vs,
                &[0., 1., 2., 3., 4., 4.5, 4.5, 4.5, 4.5, 4.5][0..10],
                abs_all <= 0.
            );
        }
    }

    #[test]
    fn test_cancel_and_hold_during_exponential_ramp() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        {
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 10.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            // set to 0.0001 at t=0 (0. is a special case)
            render.handle_incoming_event(param.set_value_at_time_raw(0.0001, 0.));
            render.handle_incoming_event(param.exponential_ramp_to_value_at_time_raw(1.0, 10.));
            render.handle_incoming_event(param.cancel_and_hold_at_time_raw(5.));

            // compute resulting buffer:
            // v(t) = v1*(v2/v1)^((t-t1)/(t2-t1))
            let mut res = Vec::<f32>::with_capacity(10);
            let start: f32 = 0.0001;
            let end: f32 = 1.;

            for t in 0..6 {
                let value = start * (end / start).powf(t as f32 / 10.);
                res.push(value);
            }

            let hold_value = res.pop().unwrap();
            res.resize(10, hold_value);

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(vs, &res[..], abs_all <= 0.);
        }

        {
            // cancel between 2 samples
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 10.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            // set to 0.0001 at t=0 (0. is a special case)
            render.handle_incoming_event(param.set_value_at_time_raw(0.0001, 0.));
            render.handle_incoming_event(param.exponential_ramp_to_value_at_time_raw(1.0, 10.));
            render.handle_incoming_event(param.cancel_and_hold_at_time_raw(4.5));

            // compute resulting buffer:
            // v(t) = v1*(v2/v1)^((t-t1)/(t2-t1))
            let mut res = Vec::<f32>::with_capacity(10);
            let start: f32 = 0.0001;
            let end: f32 = 1.;

            for t in 0..5 {
                let value = start * (end / start).powf(t as f32 / 10.);
                res.push(value);
            }

            let hold_value = start * (end / start).powf(4.5 / 10.);
            res.resize(10, hold_value);

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(vs, &res[..], abs_all <= 0.);
        }
    }

    #[test]
    fn test_cancel_and_hold_during_set_value_curve() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        {
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 2.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            let curve = [0., 0.5, 1., 0.5, 0.];
            render.handle_incoming_event(param.set_value_curve_at_time_raw(&curve[..], 0., 10.));
            render.handle_incoming_event(param.cancel_and_hold_at_time_raw(5.));

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(
                vs,
                &[0., 0.2, 0.4, 0.6, 0.8, 1., 1., 1., 1., 1.][..],
                abs_all <= 1e-7
            );
        }

        {
            // sub-sample
            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 2.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            let curve = [0., 0.5, 1., 0.5, 0.];
            render.handle_incoming_event(param.set_value_curve_at_time_raw(&curve[..], 0., 10.));
            render.handle_incoming_event(param.cancel_and_hold_at_time_raw(4.5));

            let vs = render.compute_intrinsic_values(0., 1., 10);
            assert_float_eq!(
                vs,
                &[0., 0.2, 0.4, 0.6, 0.8, 0.9, 0.9, 0.9, 0.9, 0.9][..],
                abs_all <= 1e-7
            );
        }
    }

    #[test]
    fn test_set_value_curve_at_time_a_rate() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 0.,
            min_value: 0.,
            max_value: 10.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());

        // set to 0.0001 at t=0 (0. is a special case)
        let curve = [0., 0.5, 1., 0.5, 0.];
        render.handle_incoming_event(param.set_value_curve_at_time_raw(&curve[..], 0., 10.));

        let vs = render.compute_intrinsic_values(0., 1., 10);
        assert_float_eq!(
            vs,
            &[0., 0.2, 0.4, 0.6, 0.8, 1., 0.8, 0.6, 0.4, 0.2][..],
            abs_all <= 1e-7
        );

        let vs = render.compute_intrinsic_values(10., 1., 10);
        assert_float_eq!(vs, &[0.; 10][..], abs_all <= 0.);
    }

    #[test]
    fn test_set_value_curve_at_time_a_rate_multiple_frames() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 0.,
            min_value: 0.,
            max_value: 10.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());

        // set to 0.0001 at t=0 (0. is a special case)
        let curve = [0., 0.5, 1., 0.5, 0.];
        render.handle_incoming_event(param.set_value_curve_at_time_raw(&curve[..], 0., 20.));

        let vs = render.compute_intrinsic_values(0., 1., 10);
        assert_float_eq!(
            vs,
            &[0., 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9][..],
            abs_all <= 1e-7
        );

        let vs = render.compute_intrinsic_values(10., 1., 10);
        assert_float_eq!(
            vs,
            &[1., 0.9, 0.8, 0.7, 0.6, 0.5, 0.4, 0.3, 0.2, 0.1][..],
            abs_all <= 1e-7
        );

        let vs = render.compute_intrinsic_values(20., 1., 10);
        assert_float_eq!(vs, &[0.; 10][..], abs_all <= 0.);
    }

    #[test]
    #[should_panic]
    fn test_set_value_curve_at_time_insert_while_another_event() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 1.,
            min_value: 0.,
            max_value: 1.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());

        render.handle_incoming_event(param.set_value_at_time_raw(0.0, 5.));

        let curve = [0., 0.5, 1., 0.5, 0.];
        render.handle_incoming_event(param.set_value_curve_at_time_raw(&curve[..], 0., 10.));
        // this is necessary as the panic is triggered in the audio thread
        // @note - argues in favor of maintaining the queue in control thread
        let _vs = render.compute_intrinsic_values(0., 1., 10);
    }

    #[test]
    #[should_panic]
    fn test_set_value_curve_at_time_insert_another_event_inside() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 1.,
            min_value: 0.,
            max_value: 1.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());

        let curve = [0., 0.5, 1., 0.5, 0.];
        render.handle_incoming_event(param.set_value_curve_at_time_raw(&curve[..], 0., 10.));
        render.handle_incoming_event(param.set_value_at_time_raw(0.0, 5.));
        // this is necessary as the panic is triggered in the audio thread
        // @note - argues in favor of maintaining the queue in control thread
        let _vs = render.compute_intrinsic_values(0., 1., 10);
    }

    #[test]
    fn test_set_value_curve_waits_for_start_time() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 0.,
            min_value: 0.,
            max_value: 10.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());

        // set to 0.0001 at t=0 (0. is a special case)
        let curve = [0., 0.5, 1., 0.5, 0.];
        render.handle_incoming_event(param.set_value_curve_at_time_raw(&curve[..], 5., 10.));

        let vs = render.compute_intrinsic_values(0., 1., 10);
        assert_float_eq!(
            vs,
            &[0., 0., 0., 0., 0., 0., 0.2, 0.4, 0.6, 0.8][..],
            abs_all <= 0.
        );
    }

    #[test]
    fn test_update_automation_rate_to_k() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: 0.,
            min_value: -10.,
            max_value: 10.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());

        render.onmessage(&mut AutomationRate::K);
        render.handle_incoming_event(param.set_value_at_time_raw(2., 0.000001));

        let vs = render.compute_intrinsic_values(0., 1., 10);
        assert_float_eq!(vs, &[0.; 1][..], abs_all <= 0.);
    }

    #[test]
    fn test_update_automation_rate_to_a() {
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::K,
            default_value: 0.,
            min_value: -10.,
            max_value: 10.,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());

        render.onmessage(&mut AutomationRate::A);
        render.handle_incoming_event(param.set_value_at_time_raw(2., 0.000001));

        let vs = render.compute_intrinsic_values(0., 1., 10);
        assert_float_eq!(vs, &[2.; 10][..], abs_all <= 0.);
    }

    #[test]
    fn test_varying_param_size() {
        // event registered online during rendering
        {
            let context = OfflineAudioContext::new(1, 1, 48000.);

            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 10.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            render.handle_incoming_event(param.set_value_at_time_raw(0., 0.));
            render.handle_incoming_event(param.linear_ramp_to_value_at_time_raw(9., 9.));

            // first block should be length 10 (128 in real world)
            let vs = render.compute_intrinsic_values(0., 1., 10);
            let expected = [0., 1., 2., 3., 4., 5., 6., 7., 8., 9.];
            assert_float_eq!(vs, &expected[..], abs_all <= 0.);

            // second block should have length 1
            let vs = render.compute_intrinsic_values(10., 1., 10);
            let expected = [9.; 1];
            assert_float_eq!(vs, &expected[..], abs_all <= 0.);

            // insert event in third block, should have length 10
            render.handle_incoming_event(param.set_value_at_time_raw(1., 25.));

            let vs = render.compute_intrinsic_values(20., 1., 10);
            let expected = [9., 9., 9., 9., 9., 1., 1., 1., 1., 1.];
            assert_float_eq!(vs, &expected[..], abs_all <= 0.);

            // fourth block should have length 1
            let vs = render.compute_intrinsic_values(30., 1., 10);
            let expected = [1.; 1];
            assert_float_eq!(vs, &expected[..], abs_all <= 0.);
        }

        // event registered before rendering
        {
            let context = OfflineAudioContext::new(1, 1, 48000.);

            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 10.,
            };
            let (param, mut render) = audio_param_pair(opts, context.mock_registration());

            render.handle_incoming_event(param.set_value_at_time_raw(0., 0.));
            render.handle_incoming_event(param.linear_ramp_to_value_at_time_raw(9., 9.));
            render.handle_incoming_event(param.set_value_at_time_raw(1., 25.));

            // first block should be length 10 (128 in real world)
            let vs = render.compute_intrinsic_values(0., 1., 10);
            let expected = [0., 1., 2., 3., 4., 5., 6., 7., 8., 9.];
            assert_float_eq!(vs, &expected[..], abs_all <= 0.);

            // second block should have length 1
            let vs = render.compute_intrinsic_values(10., 1., 10);
            let expected = [9.; 1];
            assert_float_eq!(vs, &expected[..], abs_all <= 0.);

            // set value event in third block, length should be 10
            let vs = render.compute_intrinsic_values(20., 1., 10);
            let expected = [9., 9., 9., 9., 9., 1., 1., 1., 1., 1.];
            assert_float_eq!(vs, &expected[..], abs_all <= 0.);

            // fourth block should have length 1
            let vs = render.compute_intrinsic_values(30., 1., 10);
            let expected = [1.; 1];
            assert_float_eq!(vs, &expected[..], abs_all <= 0.);
        }
    }

    #[test]
    fn test_varying_param_size_modulated() {
        let alloc = Alloc::with_capacity(1);

        // buffer length is 1 and input is silence (no modulation)
        {
            let context = OfflineAudioContext::new(1, 1, 48000.);

            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 10.,
            };
            let (_param, mut render) = audio_param_pair(opts, context.mock_registration());

            // no event in timeline, buffer length is 1
            let vs = render.compute_intrinsic_values(0., 1., 128);
            assert_float_eq!(vs, &[0.; 1][..], abs_all <= 0.);

            // mix to output step, input is silence
            let signal = alloc.silence();
            let input = AudioRenderQuantum::from(signal);

            let signal = alloc.silence();
            let mut output = AudioRenderQuantum::from(signal);

            render.mix_to_output(&input, &mut output);

            assert!(output.single_valued());
            assert_float_eq!(output.channel_data(0)[0], 0., abs <= 0.);
        }

        // buffer length is 1 and input is non silent
        {
            let context = OfflineAudioContext::new(1, 1, 48000.);

            let opts = AudioParamDescriptor {
                name: String::new(),
                automation_rate: AutomationRate::A,
                default_value: 0.,
                min_value: 0.,
                max_value: 10.,
            };
            let (_param, mut render) = audio_param_pair(opts, context.mock_registration());

            // no event in timeline, buffer length is 1
            let vs = render.compute_intrinsic_values(0., 1., 128);
            assert_float_eq!(vs, &[0.; 1][..], abs_all <= 0.);

            // mix to output step, input is not silence
            let signal = alloc.silence();
            let mut input = AudioRenderQuantum::from(signal);
            input.channel_data_mut(0)[0] = 1.;

            let signal = alloc.silence();
            let mut output = AudioRenderQuantum::from(signal);

            render.mix_to_output(&input, &mut output);

            let mut expected = [0.; 128];
            expected[0] = 1.;

            assert!(!output.single_valued());
            assert_float_eq!(output.channel_data(0)[..], &expected[..], abs_all <= 0.);
        }
    }

    #[test]
    fn test_k_rate_makes_input_single_valued() {
        let alloc = Alloc::with_capacity(1);
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::K,
            default_value: 0.,
            min_value: 0.,
            max_value: 10.,
        };
        let (_param, mut render) = audio_param_pair(opts, context.mock_registration());

        // no event in timeline, buffer length is 1
        let vs = render.compute_intrinsic_values(0., 1., 128);
        assert_float_eq!(vs, &[0.; 1][..], abs_all <= 0.);

        // mix to output step, input is not silence
        let signal = alloc.silence();
        let mut input = AudioRenderQuantum::from(signal);
        input.channel_data_mut(0)[0] = 1.;
        input.channel_data_mut(0)[1] = 2.;
        input.channel_data_mut(0)[2] = 3.;

        let signal = alloc.silence();
        let mut output = AudioRenderQuantum::from(signal);

        render.mix_to_output(&input, &mut output);

        // expect only 1, not the other values
        assert!(output.single_valued());
        assert_float_eq!(output.channel_data(0)[0], 1., abs <= 0.);
    }

    #[test]
    fn test_full_render_chain() {
        let alloc = Alloc::with_capacity(1);
        // prevent regression between the different processing stage
        let context = OfflineAudioContext::new(1, 1, 48000.);

        let min = 2.;
        let max = 42.;
        let default = 2.;

        let opts = AudioParamDescriptor {
            name: String::new(),
            automation_rate: AutomationRate::A,
            default_value: default,
            min_value: min,
            max_value: max,
        };
        let (param, mut render) = audio_param_pair(opts, context.mock_registration());

        render.handle_incoming_event(param.set_value_raw(128.));
        render.handle_incoming_event(param.linear_ramp_to_value_at_time_raw(0., 128.));

        let intrinsic_values = render.compute_intrinsic_values(0., 1., 128);
        let mut expected = [0.; 128];
        for (i, v) in expected.iter_mut().enumerate() {
            *v = 128. - i as f32;
        }
        assert_float_eq!(intrinsic_values, &expected[..], abs_all <= 0.);

        let signal = alloc.silence();
        let mut input = AudioRenderQuantum::from(signal);
        input.channel_data_mut(0)[0] = f32::NAN;
        let signal = alloc.silence();
        let mut output = AudioRenderQuantum::from(signal);

        render.mix_to_output(&input, &mut output);

        // clamp expected
        expected.iter_mut().for_each(|v| *v = v.clamp(min, max));
        // fix NAN at position 0
        expected[0] = 2.;

        assert_float_eq!(output.channel_data(0)[..], &expected[..], abs_all <= 0.);
    }
}
