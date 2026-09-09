//! Structured diagnostics for realtime audio contexts.

/// Snapshot of an [`AudioContext`](crate::context::AudioContext), its backend, render thread, and
/// audio graph.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct AudioContextDiagnostics {
    /// Audio backend details collected on the control thread.
    pub backend: AudioBackendDiagnostics,
    /// Render thread details collected on the render thread.
    pub render_thread: AudioRenderThreadDiagnostics,
    /// Audio graph details collected on the render thread.
    pub graph: AudioGraphDiagnostics,
}

/// Snapshot of the active audio backend.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct AudioBackendDiagnostics {
    /// Backend implementation name.
    pub name: String,
    /// Current audio output device id.
    pub sink_id: String,
    /// Current output latency in seconds, if the backend can report it.
    pub output_latency: Option<f64>,
}

/// Snapshot of the active render thread.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct AudioRenderThreadDiagnostics {
    /// Render thread sample rate in Hz.
    pub sample_rate: f32,
    /// Backend callback buffer size in frames.
    pub buffer_size: usize,
    /// Number of frames played by the backend.
    pub frames_played: u64,
    /// Number of output channels used by the backend stream.
    pub number_of_channels: usize,
    /// Whether rendering is currently suspended.
    pub suspended: bool,
}

/// Snapshot of the audio graph.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct AudioGraphDiagnostics {
    /// Whether the graph is loaded
    pub active: bool,
    /// Number of registered nodes.
    pub node_count: usize,
    /// Number of outgoing graph connections.
    pub edge_count: usize,
    /// Node ids in the current render ordering.
    pub ordered: Vec<u64>,
    /// Node ids currently excluded from rendering because they are in an unbreakable cycle.
    pub in_cycle: Vec<u64>,
    /// Node ids marked as eligible cycle breakers.
    pub cycle_breakers: Vec<u64>,
    /// Registered audio nodes.
    pub nodes: Vec<AudioNodeDiagnostics>,
}

/// Snapshot of a single audio graph node.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct AudioNodeDiagnostics {
    /// Internal audio node id.
    pub id: u64,
    /// Renderer type name.
    pub processor: String,
    /// Number of input ports.
    pub inputs: usize,
    /// Number of output ports.
    pub outputs: usize,
    /// Current channel count for each input port.
    pub input_channels: Vec<usize>,
    /// Current channel count for each output port.
    pub output_channels: Vec<usize>,
    /// Channel configuration formatted for inspection.
    pub channel_config: String,
    /// Outgoing graph connections.
    pub outgoing_edges: Vec<AudioGraphEdgeDiagnostics>,
    /// Whether the control-side handle for this node has been dropped.
    pub control_handle_dropped: bool,
    /// Whether this node had incoming connections during the latest render quantum.
    pub has_inputs_connected: bool,
    /// Whether this node can break render cycles.
    pub cycle_breaker: bool,
    /// Whether this node reports side effects.
    pub has_side_effects: bool,
}

/// Snapshot of a graph connection from one node output to another node input.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct AudioGraphEdgeDiagnostics {
    /// Source output port index.
    pub output: usize,
    /// Destination node id.
    pub destination: u64,
    /// Destination input port index, or `None` for hidden internal connections.
    pub input: Option<usize>,
}
