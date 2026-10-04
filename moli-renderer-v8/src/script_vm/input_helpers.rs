use super::ScriptVm;
use std::collections::HashSet;

use crate::document_runtime::{DomHandle, EventTargetHandle};
use crate::host::ModuleFailurePolicy;
use crate::module_runtime::{ModuleGraphHandle, ModuleLoadError, ModuleLoadStage};
use crate::native_bridge::JsContextHost;
use crate::types::ScriptErrorValue;

/// Native hover ancestry retained independently of later author DOM mutations.
#[derive(Clone)]
pub(super) struct HoverTarget {
    pub(super) handle: DomHandle,
    document: Option<DomHandle>,
    pub(super) root_to_frame: moli_layout::LayoutTransform2D,
    path: Vec<DomHandle>,
}

#[derive(Clone, Copy)]
pub(super) enum HoverBoundaryKind {
    Out,
    Leave,
    Over,
    Enter,
}

impl HoverBoundaryKind {
    pub(super) fn event_names(self) -> (&'static str, &'static str) {
        match self {
            Self::Out => ("pointerout", "mouseout"),
            Self::Leave => ("pointerleave", "mouseleave"),
            Self::Over => ("pointerover", "mouseover"),
            Self::Enter => ("pointerenter", "mouseenter"),
        }
    }
}

pub(super) struct HoverBoundary {
    pub(super) kind: HoverBoundaryKind,
    pub(super) handle: DomHandle,
    pub(super) related: Option<DomHandle>,
    pub(super) root_to_frame: moli_layout::LayoutTransform2D,
}

impl HoverTarget {
    pub(super) fn capture(
        runtime: &JsContextHost,
        handle: DomHandle,
        root_to_frame: moli_layout::LayoutTransform2D,
    ) -> Self {
        let path = runtime
            .build_propagation_path(EventTargetHandle::Node(handle), true)
            .into_iter()
            .filter_map(|target| {
                let EventTargetHandle::Node(handle) = target else {
                    return None;
                };
                runtime
                    .dom_host()
                    .node(handle)
                    .is_some_and(|node| node.is_element())
                    .then_some(handle)
            })
            .collect();
        Self {
            handle,
            document: runtime.dom_host().owner_document_handle(handle),
            root_to_frame,
            path,
        }
    }

    pub(super) fn boundaries(
        previous: Option<&Self>,
        current: Option<&Self>,
    ) -> Vec<HoverBoundary> {
        if previous.map(|target| target.handle) == current.map(|target| target.handle) {
            return Vec::new();
        }
        let mut boundaries = Vec::new();
        let same_document = previous.is_some_and(|previous| {
            previous.document.is_some()
                && current.is_some_and(|current| previous.document == current.document)
        });
        if let Some(previous) = previous {
            boundaries.push(HoverBoundary {
                kind: HoverBoundaryKind::Out,
                handle: previous.handle,
                related: current
                    .filter(|_| same_document)
                    .map(|current| current.handle),
                root_to_frame: previous.root_to_frame,
            });
            for &handle in &previous.path {
                if current.is_none_or(|current| !current.path.contains(&handle)) {
                    boundaries.push(HoverBoundary {
                        kind: HoverBoundaryKind::Leave,
                        handle,
                        related: current
                            .filter(|_| same_document)
                            .map(|current| current.handle),
                        root_to_frame: previous.root_to_frame,
                    });
                }
            }
        }
        let Some(current) = current else {
            return boundaries;
        };
        let related = previous
            .filter(|_| same_document)
            .map(|previous| previous.handle);
        boundaries.push(HoverBoundary {
            kind: HoverBoundaryKind::Over,
            handle: current.handle,
            related,
            root_to_frame: current.root_to_frame,
        });
        for &handle in current.path.iter().rev() {
            if previous.is_none_or(|previous| !previous.path.contains(&handle)) {
                boundaries.push(HoverBoundary {
                    kind: HoverBoundaryKind::Enter,
                    handle,
                    related,
                    root_to_frame: current.root_to_frame,
                });
            }
        }
        boundaries
    }
}

/// The pointer event's original ancestry, before author listeners mutate it.
pub(super) struct CompatibilityMouseTarget {
    document: DomHandle,
    path: Vec<EventTargetHandle>,
}

impl CompatibilityMouseTarget {
    pub(super) fn capture(runtime: &JsContextHost, target: DomHandle) -> Option<Self> {
        Some(Self {
            document: runtime.dom_host().owner_document_handle(target)?,
            path: runtime.build_propagation_path(EventTargetHandle::Node(target), true),
        })
    }

    pub(super) fn resolve(&self, runtime: &JsContextHost) -> Option<DomHandle> {
        let dom = runtime.dom_host();
        self.path.iter().find_map(|target| {
            let EventTargetHandle::Node(handle) = *target else {
                return None;
            };
            // ShadowRoot participates in propagation, but a native mouse
            // target must be a surviving element or its Document.
            (dom.node(handle)
                .is_some_and(|node| node.is_element() || node.is_document())
                && dom.owner_document_handle(handle) == Some(self.document)
                && dom.is_connected_to_document(handle))
            .then_some(handle)
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PendingMousePress {
    pub(super) handle: DomHandle,
    pub(super) button: i32,
}

impl PendingMousePress {
    /// Resolve against the current DOM after release listeners have run. A
    /// captured release keeps its dispatch target even after capture is lost.
    pub(super) fn click_target(
        self,
        runtime: &JsContextHost,
        released_handle: DomHandle,
        button: i32,
        was_captured: bool,
    ) -> Option<DomHandle> {
        if self.button != button {
            return None;
        }
        if was_captured || self.handle == released_handle {
            return Some(released_handle);
        }
        let dom = runtime.dom_host().dom();
        let mut ancestors = HashSet::new();
        let mut current = Some(self.handle);
        while let Some(handle) = current {
            ancestors.insert(handle);
            current = dom.parent_node(handle);
        }
        let mut current = Some(released_handle);
        while let Some(handle) = current {
            if ancestors.contains(&handle) {
                return Some(handle);
            }
            current = dom.parent_node(handle);
        }
        None
    }
}

#[derive(Clone, Copy)]
pub(super) struct PendingMouseDrag {
    pub(super) document: DomHandle,
    pub(super) position: moli_layout::LayoutPoint,
    pub(super) root_to_frame: moli_layout::LayoutTransform2D,
}

impl PendingMouseDrag {
    pub(super) fn threshold_exceeded(self, position: moli_layout::LayoutPoint) -> bool {
        // A held move without displacement must never begin a drag. Chromium
        // uses this platform threshold for both mouse and stylus initiation.
        let threshold = if cfg!(target_os = "macos") { 3.0 } else { 4.0 };
        (position.x.floor() - self.position.x.floor()).abs() >= threshold
            || (position.y.floor() - self.position.y.floor()).abs() >= threshold
    }
}

#[derive(Clone, Copy)]
pub(super) struct MouseFrameCapture {
    pub(super) frame: DomHandle,
    // A navigation replaces the Document, but retains this browsing context.
    // Removing and reattaching the same iframe creates a different lane.
    pub(super) owner: crate::frame_owner_model::FrameLaneTaskOwner,
    pub(super) root_to_frame: moli_layout::LayoutTransform2D,
}

#[derive(Clone, Copy)]
pub(super) enum MouseReleaseFollowUp {
    ActivateViaClick,
    Auxiliary,
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
    vm.pending_mouse_drags.clear();
    vm.mouse_frame_captures.clear();
    vm.hovered_mouse = None;
    vm.hovered_pointers.clear();
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
    vm.suppressed_drag_pointer = None;
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
    error_value: Option<ScriptErrorValue>,
    body_activity: PreparedScriptBodyActivity,
}

impl PreparedScriptExecutionError {
    pub(crate) fn from_message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            module_load_stage: None,
            module_failure_policy: None,
            error_value: None,
            body_activity: PreparedScriptBodyActivity::NotEntered,
        }
    }

    pub(crate) fn from_entered_script_message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            module_load_stage: None,
            module_failure_policy: None,
            error_value: None,
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
            error_value: error.error_value(),
            body_activity: PreparedScriptBodyActivity::NotEntered,
        }
    }

    pub(crate) fn from_top_level_module_source_load_failure(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            module_load_stage: Some(ModuleLoadStage::Fetch),
            module_failure_policy: Some(ModuleFailurePolicy::TopLevelLoadFailure),
            error_value: None,
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

    pub(crate) fn error_value(&self) -> Option<ScriptErrorValue> {
        self.error_value
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
