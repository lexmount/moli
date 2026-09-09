use std::{cell::Cell, rc::Rc};

#[derive(Default)]
struct ScriptExecutionDepth(Rc<Cell<usize>>);

/// HTML realm execution contexts outlive the JavaScript frames of a script or
/// callback, including callback resolution, return conversion, and script error
/// reporting. Keep that part of the execution stack on the isolate/Agent.
pub(crate) struct ScriptExecutionScope(Rc<Cell<usize>>);

impl ScriptExecutionScope {
    pub(crate) fn enter(isolate: &mut v8::Isolate) -> Self {
        if isolate.get_slot::<ScriptExecutionDepth>().is_none() {
            isolate.set_slot(ScriptExecutionDepth::default());
        }
        let depth = isolate
            .get_slot::<ScriptExecutionDepth>()
            .expect("script execution depth was initialized")
            .0
            .clone();
        depth.set(depth.get() + 1);
        Self(depth)
    }
}

impl Drop for ScriptExecutionScope {
    fn drop(&mut self) {
        self.0.set(
            self.0
                .get()
                .checked_sub(1)
                .expect("script execution scope must be active"),
        );
    }
}

#[derive(Default)]
struct MicrotaskCheckpointState(Rc<Cell<bool>>);

/// HTML's performing-a-microtask-checkpoint flag also covers rejection
/// notification and checkpoint-end cleanup after V8 finishes draining jobs.
pub(crate) struct MicrotaskCheckpointScope(Rc<Cell<bool>>);

impl MicrotaskCheckpointScope {
    pub(crate) fn enter(scope: &mut v8::PinScope<'_, '_>) -> Option<Self> {
        if scope
            .get_current_context()
            .get_microtask_queue()
            .is_some_and(v8::MicrotaskQueue::is_running_microtasks)
        {
            return None;
        }
        if scope.get_slot::<MicrotaskCheckpointState>().is_none() {
            scope.set_slot(MicrotaskCheckpointState::default());
        }
        let active = scope
            .get_slot::<MicrotaskCheckpointState>()
            .expect("microtask checkpoint state was initialized")
            .0
            .clone();
        if active.replace(true) {
            return None;
        }
        Some(Self(active))
    }
}

impl Drop for MicrotaskCheckpointScope {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

pub(crate) fn can_perform_script_cleanup_checkpoint(scope: &mut v8::PinScope<'_, '_>) -> bool {
    scope
        .get_slot::<ScriptExecutionDepth>()
        .is_none_or(|depth| depth.0.get() == 0)
        && scope
            .get_slot::<MicrotaskCheckpointState>()
            .is_none_or(|state| !state.0.get())
        && !scope
            .get_current_context()
            .get_microtask_queue()
            .is_some_and(v8::MicrotaskQueue::is_running_microtasks)
        && v8::StackTrace::current_stack_trace(scope, 1)
            .is_some_and(|stack| stack.get_frame_count() == 0)
}

pub(crate) fn perform_callback_cleanup_checkpoint(scope: &mut v8::PinScope<'_, '_>) {
    if !can_perform_script_cleanup_checkpoint(scope) {
        return;
    }
    if crate::worker::perform_callback_cleanup_checkpoint_if_worker(scope) {
        return;
    }
    if let Err(error) = crate::script_vm::ScriptVm::perform_microtask_checkpoints(scope, None) {
        tracing::warn!(%error, "callback cleanup microtask checkpoint failed");
    }
}

pub(crate) fn perform_parser_script_preparation_checkpoint(
    scope: &mut v8::PinScope<'_, '_>,
) -> anyhow::Result<()> {
    if can_perform_script_cleanup_checkpoint(scope) {
        crate::script_vm::ScriptVm::perform_microtask_checkpoints(scope, None)?;
    }
    Ok(())
}
