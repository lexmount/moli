//! Message passing from control to render node

use std::any::Any;

#[cfg(feature = "diagnostics")]
use crate::context::AudioBackendDiagnostics;
use crate::context::AudioNodeId;
use crate::node::{ChannelConfigInner, ChannelCountMode, ChannelInterpretation};
use crate::render::graph::Graph;
use crate::render::AudioProcessor;

/// Commands from the control thread to the render thread
pub(crate) enum ControlMessage {
    /// Register a new node in the audio graph
    RegisterNode {
        id: AudioNodeId,
        reclaim_id: llq::Node<AudioNodeId>,
        node: Box<dyn AudioProcessor>,
        inputs: usize,
        outputs: usize,
        channel_config: ChannelConfigInner,
    },

    /// Connect a node to another in the audio graph
    ConnectNode {
        from: AudioNodeId,
        to: AudioNodeId,
        input: usize,
        output: usize,
    },

    /// Clear the connection between two given nodes in the audio graph
    DisconnectNode {
        from: AudioNodeId,
        to: AudioNodeId,
        input: usize,
        output: usize,
    },

    /// Notify the render thread this node is dropped in the control thread
    ControlHandleDropped { id: AudioNodeId },

    /// Mark node as a cycle breaker (DelayNode only)
    MarkCycleBreaker { id: AudioNodeId },

    /// Shut down and recycle the audio graph
    CloseAndRecycle {
        sender: crossbeam_channel::Sender<Graph>,
    },

    /// Start rendering with given audio graph
    Startup { graph: Graph },

    /// Suspend and pause audio processing
    Suspend { notify: OneshotNotify },

    /// Resume audio processing after suspending
    Resume { notify: OneshotNotify },

    /// Stop audio processing
    Close { notify: OneshotNotify },

    /// Generic message to be handled by AudioProcessor
    NodeMessage {
        id: AudioNodeId,
        msg: llq::Node<Box<dyn Any + Send>>,
    },

    /// Request a diagnostic report of the audio graph
    #[cfg(feature = "diagnostics")]
    RunDiagnostics { backend: AudioBackendDiagnostics },

    /// Update the channel count of a node
    SetChannelCount { id: AudioNodeId, count: usize },

    /// Update the channel count mode of a node
    SetChannelCountMode {
        id: AudioNodeId,
        mode: ChannelCountMode,
    },

    /// Update the channel interpretation of a node
    SetChannelInterpretation {
        id: AudioNodeId,
        interpretation: ChannelInterpretation,
    },
}

/// Helper object to emit single notification
pub(crate) enum OneshotNotify {
    /// A synchronous oneshot sender
    Sync(crossbeam_channel::Sender<()>),
    /// An asynchronous oneshot sender
    Async(futures_channel::oneshot::Sender<()>),
}

impl OneshotNotify {
    /// Emit the notification
    pub fn send(self) {
        match self {
            Self::Sync(s) => s.send(()).ok(),
            Self::Async(s) => s.send(()).ok(),
        };
    }
}
