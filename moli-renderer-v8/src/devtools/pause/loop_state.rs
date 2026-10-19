use moli_page_types::RendererDevToolsAgentToken;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RendererInspectorPausePhase {
    Running,
    Entering,
    Paused,
}

/// Selects which DevTools receivers may be pumped by V8's nested pause loop.
///
/// Chromium runs a nestable Main-thread message loop for ordinary debugger
/// pauses, but instrumentation pauses only process interrupting Inspector
/// work. The policy is captured from the `Debugger.paused` notification before
/// V8 calls `run_message_loop_on_pause`, so the loop never needs to inspect a
/// command method or infer priority from its queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererInspectorPauseLoopPolicy {
    MainAndIo,
    IoOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererInspectorPauseExitReason {
    Resumed,
    SessionDetached,
    TargetClosed,
}

/// Identifies one reported pause, including pauses of the same Agent after
/// cancellation. A delayed navigation task must never retire a later pause.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct RendererInspectorPauseId {
    pub(crate) agent: RendererDevToolsAgentToken,
    pub(crate) generation: u64,
}
