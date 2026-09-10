//! The `BaseAudioContext` interface and the `AudioContext` and `OfflineAudioContext` types

use std::{any::Any, ops::Range};

mod base;
pub use base::*;

mod concrete_base;
pub use concrete_base::*;

#[cfg(feature = "diagnostics")]
mod diagnostics;
#[cfg(feature = "diagnostics")]
pub use diagnostics::*;

mod offline;
pub use offline::*;

mod online;
pub use online::*;

// magic node values
/// Destination node id is always at index 0
pub(crate) const DESTINATION_NODE_ID: AudioNodeId = AudioNodeId(0);
/// listener node id is always at index 1
const LISTENER_NODE_ID: AudioNodeId = AudioNodeId(1);
/// listener audio parameters ids are always at index 2 through 10
const LISTENER_PARAM_IDS: Range<u64> = 2..11;
/// listener audio parameters ids are always at index 2 through 10
pub(crate) const LISTENER_AUDIO_PARAM_IDS: [AudioParamId; 9] = [
    AudioParamId(2),
    AudioParamId(3),
    AudioParamId(4),
    AudioParamId(5),
    AudioParamId(6),
    AudioParamId(7),
    AudioParamId(8),
    AudioParamId(9),
    AudioParamId(10),
];

/// Unique identifier for audio nodes.
///
/// Used for internal bookkeeping.
#[derive(Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) struct AudioNodeId(pub u64);

impl std::fmt::Debug for AudioNodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AudioNodeId({})", self.0)
    }
}

/// Unique identifier for audio params.
///
/// Store these in your `AudioProcessor` to get access to `AudioParam` values.
#[derive(Debug)]
pub struct AudioParamId(u64);

// bit contrived, but for type safety only the context mod can access the inner u64
impl From<&AudioParamId> for AudioNodeId {
    fn from(i: &AudioParamId) -> Self {
        Self(i.0)
    }
}

/// Describes the current state of the `AudioContext`
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AudioContextState {
    /// This context is currently suspended (context time is not proceeding,
    /// audio hardware may be powered down/released).
    Suspended,
    /// Audio is being processed.
    Running,
    /// This context has been released, and can no longer be used to process audio.
    /// All system audio resources have been released.
    Closed,
}

impl From<u8> for AudioContextState {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Suspended,
            1 => Self::Running,
            2 => Self::Closed,
            _ => unreachable!(),
        }
    }
}

/// Handle of the [`AudioNode`](crate::node::AudioNode) to its associated [`BaseAudioContext`].
///
/// Only when implementing the AudioNode trait manually, this struct is of any concern.
///
/// This object allows for communication with the render thread and dynamic lifetime management.
// The only way to construct this object is by calling [`BaseAudioContext::register`].
// This struct should not derive Clone because of the Drop handler.
pub struct AudioContextRegistration {
    /// the audio context in which nodes and connections lives
    context: ConcreteBaseAudioContext,
    /// identify a specific `AudioNode`
    id: AudioNodeId,
}

impl std::fmt::Debug for AudioContextRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioContextRegistration")
            .field("id", &self.id)
            .field(
                "context",
                &format!("BaseAudioContext@{}", self.context.address()),
            )
            .finish()
    }
}

impl AudioContextRegistration {
    /// Get the audio node id of the registration
    #[must_use]
    pub(crate) fn id(&self) -> AudioNodeId {
        self.id
    }

    /// Get the [`BaseAudioContext`] concrete type associated with this `AudioContext`
    #[must_use]
    pub(crate) fn context(&self) -> &ConcreteBaseAudioContext {
        &self.context
    }

    /// Send a message to the corresponding audio processor of this node
    ///
    /// The message will be handled by
    /// [`AudioProcessor::onmessage`](crate::render::AudioProcessor::onmessage).
    pub(crate) fn post_message<M: Any + Send + 'static>(&self, msg: M) {
        let wrapped = crate::message::ControlMessage::NodeMessage {
            id: self.id,
            msg: llq::Node::new(Box::new(msg)),
        };
        self.context.send_control_msg(wrapped);
    }
}

impl Drop for AudioContextRegistration {
    fn drop(&mut self) {
        self.context.mark_node_dropped(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::AudioNode;

    fn require_send_sync_static<T: Send + Sync + 'static>(_: T) {}

    #[test]
    fn test_audio_context_registration_traits() {
        let context = OfflineAudioContext::new(1, 1, 44100.);
        let registration = context.mock_registration();

        // we want to be able to ship AudioNodes to another thread, so the Registration should be
        // Send, Sync and 'static
        require_send_sync_static(registration);
    }

    #[test]
    fn test_offline_audio_context_send_sync() {
        let context = OfflineAudioContext::new(1, 1, 44100.);
        require_send_sync_static(context);
    }

    #[test]
    fn test_online_audio_context_send_sync() {
        let options = AudioContextOptions {
            sink_id: "none".into(),
            ..AudioContextOptions::default()
        };
        let context = AudioContext::new(options);
        require_send_sync_static(context);
    }

    #[test]
    fn test_context_equals() {
        let context = OfflineAudioContext::new(1, 48000, 96000.);
        let dest = context.destination();
        assert!(dest.context() == context.base());
    }
}
