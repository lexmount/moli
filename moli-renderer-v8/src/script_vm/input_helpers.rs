use super::ScriptVm;
use crate::document_runtime::DomHandle;
use crate::host::ModuleFailurePolicy;
use crate::module_runtime::{ModuleGraphHandle, ModuleLoadError, ModuleLoadStage};
use crate::types::ScriptErrorConstructorKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PendingMousePress {
    pub(super) handle: DomHandle,
    pub(super) button: i32,
}

#[derive(Clone, Copy)]
pub(super) enum MouseReleaseFollowUp {
    ActivateViaClick,
    DispatchEvent(&'static str),
}

pub(super) fn mouse_button_mask(button: i32) -> i32 {
    match button {
        0 => 1,
        1 => 4,
        2 => 2,
        3 => 8,
        4 => 16,
        _ => 0,
    }
}

pub(super) fn mouse_button_from_mask(mask: i32) -> Option<i32> {
    match mask {
        1 => Some(0),
        4 => Some(1),
        2 => Some(2),
        8 => Some(3),
        16 => Some(4),
        _ => None,
    }
}

pub(super) fn single_changed_mouse_button(mask: i32) -> Option<i32> {
    if mask.count_ones() == 1 {
        mouse_button_from_mask(mask)
    } else {
        None
    }
}

pub(super) fn clear_input_dispatch_state(vm: &mut ScriptVm) {
    vm.pressed_mouse_buttons = 0;
    vm.pending_mouse_press = None;
    vm.hovered_mouse_handle = None;
    vm.hovered_mouse_root_to_client = moli_layout::LayoutTransform2D::IDENTITY;
    vm._context_host
        .borrow()
        .dom_host()
        .clear_hovered_element_handles();
    vm.active_touch_pointer_handle = None;
    vm.active_touch_pointer_handles.clear();
    vm.active_touch_event_handle = None;
    vm.active_touch_point = None;
    vm.active_touch_points.clear();
    vm.active_drag_session = None;
    vm.active_scrollbar_drag = None;
    vm.suppress_compat_mouse_events = false;
    vm._context_host.borrow_mut().clear_pointer_capture_state();
}

pub(crate) struct PreparedScriptRunInput {
    pub(crate) current_script: Option<DomHandle>,
    pub(crate) parser_write_insertion_point_active: bool,
    pub(crate) body: PreparedScriptRunBody,
}

pub(crate) enum PreparedScriptRunBody {
    LoadedSource {
        source: String,
        source_bytes: Option<Vec<u8>>,
    },
    ExternalModuleGraph,
}

#[derive(Debug)]
pub(crate) struct PreparedScriptExecutionError {
    message: String,
    module_load_stage: Option<ModuleLoadStage>,
    module_failure_policy: Option<ModuleFailurePolicy>,
    error_constructor: Option<ScriptErrorConstructorKind>,
    body_activity: PreparedScriptBodyActivity,
}

impl PreparedScriptExecutionError {
    pub(crate) fn from_message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            module_load_stage: None,
            module_failure_policy: None,
            error_constructor: None,
            body_activity: PreparedScriptBodyActivity::NotEntered,
        }
    }

    pub(crate) fn from_entered_script_message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            module_load_stage: None,
            module_failure_policy: None,
            error_constructor: None,
            body_activity: PreparedScriptBodyActivity::Entered,
        }
    }

    pub(crate) fn with_body_activity(mut self, body_activity: PreparedScriptBodyActivity) -> Self {
        self.body_activity = body_activity;
        self
    }

    pub(crate) fn from_module_load_error(error: ModuleLoadError) -> Self {
        let module_failure_policy = ModuleFailurePolicy::for_module_load_error(&error);
        Self {
            message: error.message().to_owned(),
            module_load_stage: Some(error.stage()),
            module_failure_policy: Some(module_failure_policy),
            error_constructor: error.error_constructor(),
            body_activity: PreparedScriptBodyActivity::NotEntered,
        }
    }

    pub(crate) fn from_top_level_module_source_load_failure(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            module_load_stage: Some(ModuleLoadStage::Fetch),
            module_failure_policy: Some(ModuleFailurePolicy::TopLevelLoadFailure),
            error_constructor: None,
            body_activity: PreparedScriptBodyActivity::NotEntered,
        }
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn module_load_stage(&self) -> Option<ModuleLoadStage> {
        self.module_load_stage
    }

    pub(crate) fn module_failure_policy(&self) -> Option<ModuleFailurePolicy> {
        self.module_failure_policy
    }

    pub(crate) fn error_constructor(&self) -> Option<ScriptErrorConstructorKind> {
        self.error_constructor
    }

    pub(crate) fn body_activity(&self) -> PreparedScriptBodyActivity {
        self.body_activity
    }

    pub(crate) fn into_message(self) -> String {
        self.message
    }
}

impl From<String> for PreparedScriptExecutionError {
    fn from(message: String) -> Self {
        Self::from_message(message)
    }
}

/// Whether the prepared-script algorithm actually entered script evaluation.
///
/// This is an execution fact, not queued task policy. Import-map registration,
/// module-graph startup, CSP rejection, and preparation failure can consume a
/// selected DocumentScript action without entering script code. Classic script
/// evaluation remains `Entered` even when it replaces the Document or ends in
/// an engine error.
#[must_use = "prepared-script activity determines the enclosing task completion"]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreparedScriptBodyActivity {
    NotEntered,
    Entered,
}

/// Whether synchronous script-terminal processing attempted an event dispatch.
///
/// The terminal algorithm can legitimately find no observable load/error
/// target, and the event target can have no listener. This type therefore does
/// not claim that a callback ran. It records the narrower fact needed by the
/// enclosing DocumentScript completion: an event-dispatch body was attempted
/// and can have produced callback consequences before returning.
#[must_use = "terminal dispatch activity determines the enclosing task completion"]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScriptTerminalBodyActivity {
    NoEventDispatch,
    EventDispatchAttempted,
}

pub(crate) enum PreparedScriptExecutionOutcome {
    Completed(PreparedScriptBodyActivity),
    DeferredModuleCompletion,
    Dropped(PreparedScriptBodyActivity),
}

pub(crate) enum LoadedScriptExecutionOutcome {
    Completed(PreparedScriptBodyActivity),
    CompletedModuleGraph(ModuleGraphHandle),
    SuspendedModuleFetches(Box<crate::module_runtime::ModuleScriptGraphFetchBatch>),
}

impl LoadedScriptExecutionOutcome {
    pub(crate) fn body_activity(&self) -> PreparedScriptBodyActivity {
        match self {
            Self::Completed(activity) => *activity,
            Self::CompletedModuleGraph(_) | Self::SuspendedModuleFetches(_) => {
                PreparedScriptBodyActivity::NotEntered
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn drain_internal_runtime_binding_calls(vm: &mut ScriptVm) {
    let warnings = vm
        .document_runtime
        .absorb_runtime_binding_calls_from_host(&mut vm._context_host.borrow_mut());
    for warning in warnings {
        vm.record_runtime_warning(format_args!("{warning}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_load_error_preserves_top_level_load_failure_policy() {
        let error = ModuleLoadError::new(ModuleLoadStage::Fetch, "failed to fetch script")
            .with_top_level_module_load_failure();

        let prepared = PreparedScriptExecutionError::from_module_load_error(error);

        assert_eq!(prepared.module_load_stage(), Some(ModuleLoadStage::Fetch));
        assert_eq!(
            prepared.module_failure_policy(),
            Some(ModuleFailurePolicy::TopLevelLoadFailure)
        );
    }

    #[test]
    fn module_load_error_defaults_to_graph_failure_policy() {
        let error = ModuleLoadError::new(ModuleLoadStage::Resolve, "module graph failed");

        let prepared = PreparedScriptExecutionError::from_module_load_error(error);

        assert_eq!(
            prepared.module_failure_policy(),
            Some(ModuleFailurePolicy::GraphFailure)
        );
    }

    #[test]
    fn module_evaluate_error_uses_evaluation_failure_policy() {
        let error = ModuleLoadError::new(ModuleLoadStage::Evaluate, "module evaluation failed");

        let prepared = PreparedScriptExecutionError::from_module_load_error(error);

        assert_eq!(
            prepared.module_failure_policy(),
            Some(ModuleFailurePolicy::EvaluationFailure)
        );
    }

    #[test]
    fn typed_module_evaluate_error_uses_graph_failure_policy() {
        let error = ModuleLoadError::new(ModuleLoadStage::Evaluate, "wasm link failed")
            .with_error_constructor(crate::types::ScriptErrorConstructorKind::WebAssemblyLinkError);

        let prepared = PreparedScriptExecutionError::from_module_load_error(error);

        assert_eq!(
            prepared.module_failure_policy(),
            Some(ModuleFailurePolicy::GraphFailure)
        );
    }

    #[test]
    fn module_fetch_error_uses_graph_fetch_failure_policy() {
        let error = ModuleLoadError::new(ModuleLoadStage::Fetch, "dependency fetch failed");

        let prepared = PreparedScriptExecutionError::from_module_load_error(error);

        assert_eq!(
            prepared.module_failure_policy(),
            Some(ModuleFailurePolicy::ModuleTreeLoadFailure)
        );
    }
}
