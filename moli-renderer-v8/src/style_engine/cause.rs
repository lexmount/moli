use dom::ElementState;

use crate::document_runtime::DomHandle;

pub(super) enum PendingStyleInvalidationCause {
    StateChange {
        element: DomHandle,
        state: ElementState,
        old_state: Option<ElementState>,
    },
    CustomStateChange {
        element: DomHandle,
        old_custom_states: Vec<String>,
    },
    FocusChange {
        previous: Option<DomHandle>,
        next: Option<DomHandle>,
        previous_focus_within: Vec<DomHandle>,
    },
    TargetChange {
        previous: Option<DomHandle>,
        next: Option<DomHandle>,
    },
}
