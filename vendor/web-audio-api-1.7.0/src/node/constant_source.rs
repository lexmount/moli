use std::any::Any;

use crate::context::{AudioContextRegistration, AudioParamId, BaseAudioContext};
use crate::param::{AudioParam, AudioParamDescriptor, AutomationRate};
use crate::render::{
    AudioParamValues, AudioProcessor, AudioRenderQuantum, AudioWorkletGlobalScope,
};
use crate::{assert_valid_time_value, RENDER_QUANTUM_SIZE};

use super::{AudioNode, AudioScheduledSourceNode, ChannelConfig};

/// Options for constructing an [`ConstantSourceNode`]
// dictionary ConstantSourceOptions {
//   float offset = 1;
// };
// https://webaudio.github.io/web-audio-api/#ConstantSourceOptions
//
// @note - Does not extend AudioNodeOptions because AudioNodeOptions are
// useless for source nodes, because they instruct how to upmix the inputs.
// This is a common source of confusion, see e.g. mdn/content#18472
#[derive(Clone, Debug)]
pub struct ConstantSourceOptions {
    /// Initial parameter value of the constant signal
    pub offset: f32,
}

impl Default for ConstantSourceOptions {
    fn default() -> Self {
        Self { offset: 1. }
    }
}

/// Instructions to start or stop processing
#[derive(Debug, Copy, Clone)]
enum Schedule {
    Start(f64),
    Stop(f64),
}

/// Audio source whose output is nominally a constant value.
///
/// Can be used as a constructible `AudioParam` by automating the value of its offset.
///
/// - MDN documentation: <https://developer.mozilla.org/en-US/docs/Web/API/ConstantSourceNode>
/// - specification: <https://webaudio.github.io/web-audio-api/#ConstantSourceNode>
/// - see also: [`BaseAudioContext::create_constant_source`]
///
/// # Usage
///
/// ```no_run
/// use web_audio_api::context::{BaseAudioContext, AudioContext};
/// use web_audio_api::node::AudioNode;
///
/// let audio_context = AudioContext::default();
///
/// let gain1 = audio_context.create_gain();
/// gain1.gain().set_value(0.);
///
/// let gain2 = audio_context.create_gain();
/// gain2.gain().set_value(0.);
///
/// let automation = audio_context.create_constant_source();
/// automation.offset().set_value(0.);
/// automation.connect(gain1.gain());
/// automation.connect(gain2.gain());
///
/// // control both `GainNode`s with 1 automation
/// automation.offset().set_target_at_time(1., audio_context.current_time(), 0.1);
/// ```
///
/// # Example
///
/// - `cargo run --release --example constant_source`
///
#[derive(Debug)]
pub struct ConstantSourceNode {
    registration: AudioContextRegistration,
    channel_config: ChannelConfig,
    offset: AudioParam,
    has_start: bool,
}

impl AudioNode for ConstantSourceNode {
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

impl AudioScheduledSourceNode for ConstantSourceNode {
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

impl ConstantSourceNode {
    /// Constructs a new `ConstantSourceNode` from explicit options.
    ///
    /// [`BaseAudioContext::create_constant_source`] is an alternative that
    /// applies the spec defaults (`offset = 1.0`).
    ///
    /// # Arguments
    ///
    /// * `context` - audio context in which the audio node will live
    /// * `options` - initial value of the offset parameter
    pub fn new<C: BaseAudioContext>(context: &C, options: ConstantSourceOptions) -> Self {
        context.base().register(move |registration| {
            let ConstantSourceOptions { offset } = options;

            let param_options = AudioParamDescriptor {
                name: String::new(),
                min_value: f32::MIN,
                max_value: f32::MAX,
                default_value: 1.,
                automation_rate: AutomationRate::A,
            };
            let (param, proc) = context.create_audio_param(param_options, &registration);
            param.set_value(offset);

            let render = ConstantSourceRenderer {
                offset: proc,
                start_time: f64::MAX,
                stop_time: f64::MAX,
                ended_triggered: false,
            };

            let node = ConstantSourceNode {
                registration,
                channel_config: ChannelConfig::default(),
                offset: param,
                has_start: false,
            };

            (node, Box::new(render))
        })
    }

    /// Returns the offset `AudioParam`. Default is `1.0`.
    ///
    /// Useful as a constructible `AudioParam`: connect this once to several
    /// sink params and automate it to drive them all in lockstep.
    #[must_use]
    pub fn offset(&self) -> &AudioParam {
        &self.offset
    }
}

struct ConstantSourceRenderer {
    offset: AudioParamId,
    start_time: f64,
    stop_time: f64,
    ended_triggered: bool,
}

impl AudioProcessor for ConstantSourceRenderer {
    fn process(
        &mut self,
        _inputs: &[AudioRenderQuantum],
        outputs: &mut [AudioRenderQuantum],
        params: AudioParamValues<'_>,
        scope: &AudioWorkletGlobalScope,
    ) -> bool {
        // single output node
        let output = &mut outputs[0];

        let dt = 1. / scope.sample_rate as f64;
        let next_block_time = scope.current_time + dt * RENDER_QUANTUM_SIZE as f64;

        if self.start_time >= next_block_time {
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

        output.force_mono();

        let offset = params.get(&self.offset);
        let output_channel = output.channel_data_mut(0);

        // fast path
        if offset.len() == 1
            && self.start_time <= scope.current_time
            && self.stop_time >= next_block_time
        {
            output_channel.fill(offset[0]);
        } else {
            // sample accurate path
            let mut current_time = scope.current_time;

            output_channel
                .iter_mut()
                .zip(offset.iter().cycle())
                .for_each(|(o, &value)| {
                    if current_time < self.start_time || current_time >= self.stop_time {
                        *o = 0.;
                    } else {
                        // as we pick values directly from the offset param which is already
                        // computed at sub-sample accuracy, we don't need to do more than
                        // copying the values to their right place.
                        *o = value;
                    }

                    current_time += dt;
                });
        }

        // tail_time false when output has ended this quantum
        let still_running = self.stop_time > next_block_time;

        if !still_running {
            // @note: we need this check because this is called a until the program
            // ends, such as if the node was never removed from the graph
            if !self.ended_triggered {
                scope.send_ended_event();
                self.ended_triggered = true;
            }
        }

        still_running
    }

    fn onmessage(&mut self, msg: &mut dyn Any) {
        if let Some(schedule) = msg.downcast_ref::<Schedule>() {
            match *schedule {
                Schedule::Start(v) => self.start_time = v,
                Schedule::Stop(v) => self.stop_time = v,
            }
            return;
        }

        log::warn!("ConstantSourceRenderer: Dropping incoming message {msg:?}");
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

#[cfg(test)]
mod tests {
    use crate::context::{BaseAudioContext, OfflineAudioContext};
    use crate::node::{AudioNode, AudioScheduledSourceNode};

    use float_eq::assert_float_eq;

    use super::*;

    #[test]
    fn test_audioparam_value_applies_immediately() {
        let context = OfflineAudioContext::new(1, 128, 48000.);
        let options = ConstantSourceOptions { offset: 12. };
        let src = ConstantSourceNode::new(&context, options);
        assert_float_eq!(src.offset.value(), 12., abs_all <= 0.);
    }

    #[test]
    fn test_start_stop() {
        let sample_rate = 48000.;
        let start_in_samples = (128 + 1) as f64; // start rendering in 2d block
        let stop_in_samples = (256 + 1) as f64; // stop rendering of 3rd block
        let mut context = OfflineAudioContext::new(1, 128 * 4, sample_rate);

        let mut src = context.create_constant_source();
        src.connect(&context.destination());

        src.start_at(start_in_samples / sample_rate as f64);
        src.stop_at(stop_in_samples / sample_rate as f64);

        let buffer = context.start_rendering_sync();
        let channel = buffer.get_channel_data(0);

        // 1rst block should be silence
        assert_float_eq!(channel[0..128], vec![0.; 128][..], abs_all <= 0.);

        // 2d block - start at second frame
        let mut res = vec![1.; 128];
        res[0] = 0.;
        assert_float_eq!(channel[128..256], res[..], abs_all <= 0.);

        // 3rd block - stop at second frame
        let mut res = vec![0.; 128];
        res[0] = 1.;
        assert_float_eq!(channel[256..384], res[..], abs_all <= 0.);

        // 4th block is silence
        assert_float_eq!(channel[384..512], vec![0.; 128][..], abs_all <= 0.);
    }

    #[test]
    fn test_start_in_the_past() {
        let sample_rate = 48000.;
        let mut context = OfflineAudioContext::new(1, 2 * 128, sample_rate);

        context.suspend_sync((128. / sample_rate).into(), |context| {
            let mut src = context.create_constant_source();
            src.connect(&context.destination());
            src.start_at(0.);
        });

        let buffer = context.start_rendering_sync();
        let channel = buffer.get_channel_data(0);

        // 1rst block should be silence
        assert_float_eq!(channel[0..128], vec![0.; 128][..], abs_all <= 0.);
        assert_float_eq!(channel[128..], vec![1.; 128][..], abs_all <= 0.);
    }

    #[test]
    fn test_start_in_the_future_while_dropped() {
        let sample_rate = 48000.;
        let mut context = OfflineAudioContext::new(1, 4 * 128, sample_rate);

        let mut src = context.create_constant_source();
        src.connect(&context.destination());
        src.start_at(258. / sample_rate as f64); // in 3rd block
        drop(src); // explicit drop

        let buffer = context.start_rendering_sync();
        let channel = buffer.get_channel_data(0);

        assert_float_eq!(channel[0..258], vec![0.; 258][..], abs_all <= 0.);
        assert_float_eq!(channel[258..], vec![1.; 254][..], abs_all <= 0.);
    }
}
