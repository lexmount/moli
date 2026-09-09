use std::sync::{Arc, Mutex};

use crate::context::{AudioContextRegistration, ConcreteBaseAudioContext};
use crate::events::{ErrorEvent, EventHandler, EventPayload, EventType};
use crate::message::ControlMessage;

/// How channels must be matched between the node's inputs and outputs.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ChannelCountMode {
    /// `computedNumberOfChannels` is the maximum of the number of channels of all connections to an
    /// input. In this mode channelCount is ignored.
    Max,
    /// `computedNumberOfChannels` is determined as for "max" and then clamped to a maximum value of
    /// the given channelCount.
    ClampedMax,
    /// `computedNumberOfChannels` is the exact value as specified by the channelCount.
    Explicit,
}

impl From<u32> for ChannelCountMode {
    fn from(i: u32) -> Self {
        use ChannelCountMode::*;

        match i {
            0 => Max,
            1 => ClampedMax,
            2 => Explicit,
            _ => unreachable!(),
        }
    }
}

/// The meaning of the channels, defining how audio up-mixing and down-mixing will happen.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ChannelInterpretation {
    Speakers,
    Discrete,
}

impl From<u32> for ChannelInterpretation {
    fn from(i: u32) -> Self {
        use ChannelInterpretation::*;

        match i {
            0 => Speakers,
            1 => Discrete,
            _ => unreachable!(),
        }
    }
}

/// Options that can be used in constructing all AudioNodes.
#[derive(Clone, Debug)]
pub struct AudioNodeOptions {
    /// Desired number of channels for the [`AudioNode::channel_count`] attribute.
    pub channel_count: usize,
    /// Desired mode for the [`AudioNode::channel_count_mode`] attribute.
    pub channel_count_mode: ChannelCountMode,
    /// Desired mode for the [`AudioNode::channel_interpretation`] attribute.
    pub channel_interpretation: ChannelInterpretation,
}

impl Default for AudioNodeOptions {
    fn default() -> Self {
        Self {
            channel_count: 2,
            channel_count_mode: ChannelCountMode::Max,
            channel_interpretation: ChannelInterpretation::Speakers,
        }
    }
}

/// Config for up/down-mixing of input channels for audio nodes
///
/// Only when implementing the [`AudioNode`] trait manually, this struct is of any concern. The
/// methods `set_channel_count`, `set_channel_count_mode` and `set_channel_interpretation` from the
/// audio node interface will use this struct to sync the required info to the render thread.
///
/// The only way to construct an instance is with [`AudioNodeOptions`]
///
/// ```
/// use web_audio_api::node::{AudioNodeOptions, ChannelConfig, ChannelInterpretation, ChannelCountMode};
///
/// let opts = AudioNodeOptions {
///     channel_count: 1,
///     channel_count_mode: ChannelCountMode::Explicit,
///     channel_interpretation: ChannelInterpretation::Discrete,
/// };
/// let _: ChannelConfig = opts.into();
#[derive(Clone)]
pub struct ChannelConfig {
    inner: Arc<Mutex<ChannelConfigInner>>,
}

#[derive(Debug, Clone)]
pub(crate) struct ChannelConfigInner {
    pub(crate) count: usize,
    pub(crate) count_mode: ChannelCountMode,
    pub(crate) interpretation: ChannelInterpretation,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        AudioNodeOptions::default().into()
    }
}

impl std::fmt::Debug for ChannelConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelConfig")
            .field("count", &self.count())
            .field("count_mode", &self.count_mode())
            .field("interpretation", &self.interpretation())
            .finish()
    }
}

// All methods on this struct are marked `pub(crate)` because we don't want outside users to be
// able to change the values directly.  These methods are only accessible via the AudioNode
// interface, so AudioNode's that have channel count/mode constraints should be able to assert
// those.
//
// Uses the canonical ordering for handover of values, i.e. `Acquire` on load and `Release` on
// store.
impl ChannelConfig {
    /// Represents an enumerated value describing the way channels must be matched between the
    /// node's inputs and outputs.
    pub(crate) fn count_mode(&self) -> ChannelCountMode {
        self.inner.lock().unwrap().count_mode
    }

    pub(super) fn set_count_mode(
        &self,
        v: ChannelCountMode,
        registration: &AudioContextRegistration,
    ) {
        let mut guard = self.inner.lock().unwrap();
        guard.count_mode = v;

        let message = ControlMessage::SetChannelCountMode {
            id: registration.id(),
            mode: v,
        };
        registration.context().send_control_msg(message);

        drop(guard); // drop guard after sending message to prevent out of order arrivals on
                     // concurrent access
    }

    /// Represents an enumerated value describing the meaning of the channels. This interpretation
    /// will define how audio up-mixing and down-mixing will happen.
    pub(crate) fn interpretation(&self) -> ChannelInterpretation {
        self.inner.lock().unwrap().interpretation
    }

    pub(super) fn set_interpretation(
        &self,
        v: ChannelInterpretation,
        registration: &AudioContextRegistration,
    ) {
        let mut guard = self.inner.lock().unwrap();
        guard.interpretation = v;

        let message = ControlMessage::SetChannelInterpretation {
            id: registration.id(),
            interpretation: v,
        };
        registration.context().send_control_msg(message);

        drop(guard); // drop guard after sending message to prevent out of order arrivals on
                     // concurrent access
    }

    /// Represents an integer used to determine how many channels are used when up-mixing and
    /// down-mixing connections to any inputs to the node.
    pub(crate) fn count(&self) -> usize {
        self.inner.lock().unwrap().count
    }

    pub(super) fn set_count(&self, v: usize, registration: &AudioContextRegistration) {
        crate::assert_valid_number_of_channels(v);

        let mut guard = self.inner.lock().unwrap();
        guard.count = v;

        let message = ControlMessage::SetChannelCount {
            id: registration.id(),
            count: v,
        };
        registration.context().send_control_msg(message);

        drop(guard); // drop guard after sending message to prevent out of order arrivals on
                     // concurrent access
    }

    pub(crate) fn inner(&self) -> ChannelConfigInner {
        self.inner.lock().unwrap().clone()
    }
}

impl From<AudioNodeOptions> for ChannelConfig {
    fn from(opts: AudioNodeOptions) -> Self {
        crate::assert_valid_number_of_channels(opts.channel_count);

        let inner = ChannelConfigInner {
            count: opts.channel_count,
            count_mode: opts.channel_count_mode,
            interpretation: opts.channel_interpretation,
        };
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }
}

/// This interface represents audio sources, the audio destination, and intermediate processing
/// modules.
///
/// These modules can be connected together to form processing graphs for rendering audio
/// to the audio hardware. Each node can have inputs and/or outputs.
///
/// Note that the AudioNode is typically constructed together with an `AudioWorkletProcessor`
/// (the object that lives the render thread). See the [`crate::worklet`] mod.
pub trait AudioNode {
    /// Handle of the associated [`BaseAudioContext`](crate::context::BaseAudioContext).
    ///
    /// Only when implementing the AudioNode trait manually, this struct is of any concern.
    fn registration(&self) -> &AudioContextRegistration;

    /// Config for up/down-mixing of input channels for this node.
    ///
    /// Only when implementing the [`AudioNode`] trait manually, this struct is of any concern.
    fn channel_config(&self) -> &ChannelConfig;

    /// The [`BaseAudioContext`](crate::context::BaseAudioContext) concrete type which owns this
    /// AudioNode.
    fn context(&self) -> &ConcreteBaseAudioContext {
        self.registration().context()
    }

    /// Connect the output of this AudioNode to the input of another node.
    ///
    /// # Panics
    ///
    /// This function will panic when
    /// - the AudioContext of the source and destination does not match
    fn connect<'a>(&self, dest: &'a dyn AudioNode) -> &'a dyn AudioNode {
        self.connect_from_output_to_input(dest, 0, 0)
    }

    /// Connect a specific output of this AudioNode to a specific input of another node.
    ///
    /// # Panics
    ///
    /// This function will panic when
    /// - the AudioContext of the source and destination does not match
    /// - if the input port is out of bounds for the destination node
    /// - if the output port is out of bounds for the source node
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
            self.registration().id(),
            dest.registration().id(),
            output,
            input,
        );
        dest
    }

    /// Disconnects all outgoing connections from the AudioNode.
    fn disconnect(&self) {
        self.context()
            .disconnect(self.registration().id(), None, None, None);
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
            self.registration().id(),
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
            .disconnect(self.registration().id(), Some(output), None, None);
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
            self.registration().id(),
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
            self.registration().id(),
            Some(output),
            Some(dest.registration().id()),
            Some(input),
        );
    }

    /// The number of inputs feeding into the AudioNode. For source nodes, this will be 0.
    fn number_of_inputs(&self) -> usize;

    /// The number of outputs coming out of the AudioNode.
    fn number_of_outputs(&self) -> usize;

    /// Represents an enumerated value describing the way channels must be matched between the
    /// node's inputs and outputs.
    fn channel_count_mode(&self) -> ChannelCountMode {
        self.channel_config().count_mode()
    }

    /// Update the `channel_count_mode` attribute
    fn set_channel_count_mode(&self, v: ChannelCountMode) {
        self.channel_config().set_count_mode(v, self.registration())
    }

    /// Represents an enumerated value describing the meaning of the channels. This interpretation
    /// will define how audio up-mixing and down-mixing will happen.
    fn channel_interpretation(&self) -> ChannelInterpretation {
        self.channel_config().interpretation()
    }

    /// Update the `channel_interpretation` attribute
    fn set_channel_interpretation(&self, v: ChannelInterpretation) {
        self.channel_config()
            .set_interpretation(v, self.registration())
    }
    /// Represents an integer used to determine how many channels are used when up-mixing and
    /// down-mixing connections to any inputs to the node.
    fn channel_count(&self) -> usize {
        self.channel_config().count()
    }

    /// Update the `channel_count` attribute
    fn set_channel_count(&self, v: usize) {
        self.channel_config().set_count(v, self.registration())
    }

    /// Register callback to run when an unhandled exception occurs in the audio processor.
    ///
    /// Note that once a unhandled exception is thrown, the processor will output silence throughout its lifetime.
    ///
    /// Only a single event handler is active at any time. Calling this method multiple times will
    /// override the previous event handler.
    fn set_onprocessorerror(&self, callback: Box<dyn FnOnce(ErrorEvent) + Send + 'static>) {
        let callback = move |v| match v {
            EventPayload::ProcessorError(v) => callback(v),
            _ => unreachable!(),
        };

        self.context().set_event_handler(
            EventType::ProcessorError(self.registration().id()),
            EventHandler::Once(Box::new(callback)),
        );
    }

    /// Unset the callback to run when an unhandled exception occurs in the audio processor.
    fn clear_onprocessorerror(&self) {
        self.context()
            .clear_event_handler(EventType::ProcessorError(self.registration().id()));
    }
}
