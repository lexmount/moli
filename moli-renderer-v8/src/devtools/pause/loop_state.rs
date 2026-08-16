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
