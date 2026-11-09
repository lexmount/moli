use std::sync::Arc;

use crate::frame_owner_model::MainDocumentStyleLoadEventBinding;

use super::load::StylesheetLinkClient;

/// Settlement state of one required phase of a linked stylesheet load.
///
/// `Pending` means only that the phase has not settled. A stylesheet with no
/// imports therefore starts its import phase as `Succeeded`, not `Pending`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::document_runtime) enum StylesheetCompletionState {
    Pending,
    Succeeded,
    Failed,
}

impl StylesheetCompletionState {
    pub(super) fn from_successful(successful: bool) -> Self {
        if successful {
            Self::Succeeded
        } else {
            Self::Failed
        }
    }

    fn followed_by(self, dependent: Self) -> Self {
        // Imports exist only after a usable root resource. A root failure is
        // therefore immediately terminal; import state matters only after the
        // root succeeds.
        match self {
            Self::Pending => Self::Pending,
            Self::Succeeded => dependent,
            Self::Failed => Self::Failed,
        }
    }

    fn settle(&mut self, successful: bool) -> bool {
        if *self != Self::Pending {
            return false;
        }
        *self = Self::from_successful(successful);
        true
    }
}

#[derive(Debug)]
pub(in crate::document_runtime) struct LinkStyleState {
    active_load: Arc<StylesheetLinkClient>,
    resource_completion: StylesheetCompletionState,
    import_completion: StylesheetCompletionState,
    event_phase: LinkLoadEventPhase,
}

/// Element-local consequence of settling one linked stylesheet phase.
///
/// Resource completion can publish an element event while the client remains
/// in the document's script-blocking set through that event's dispatch.
#[derive(Debug)]
pub(in crate::document_runtime) struct LinkStyleSettlement {
    ready_event: Option<(Arc<StylesheetLinkClient>, bool)>,
}

impl LinkStyleSettlement {
    pub(super) fn into_ready_event(self) -> Option<(Arc<StylesheetLinkClient>, bool)> {
        self.ready_event
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkLoadEventPhase {
    WaitingForCompletion,
    Posted,
    Dispatched,
}

impl LinkStyleState {
    pub(super) fn new(
        active_load: Arc<StylesheetLinkClient>,
        import_completion: StylesheetCompletionState,
    ) -> Self {
        Self {
            active_load,
            resource_completion: StylesheetCompletionState::Pending,
            import_completion,
            event_phase: LinkLoadEventPhase::WaitingForCompletion,
        }
    }

    pub(super) fn active_load(&self) -> &Arc<StylesheetLinkClient> {
        &self.active_load
    }

    pub(super) fn is_pending(&self) -> bool {
        self.completion() == StylesheetCompletionState::Pending
    }

    /// HTML removes the script blocker after firing load/error, including its
    /// synchronous event handlers. Resource readiness remains independent.
    pub(super) fn blocks_parser_scripts(&self) -> bool {
        self.event_phase != LinkLoadEventPhase::Dispatched
    }

    pub(super) fn cancelable_load_event_binding(
        &self,
    ) -> Option<MainDocumentStyleLoadEventBinding> {
        (self.event_phase == LinkLoadEventPhase::WaitingForCompletion)
            .then(|| self.active_load.load_event_binding())
            .flatten()
    }

    fn completion(&self) -> StylesheetCompletionState {
        self.resource_completion.followed_by(self.import_completion)
    }

    fn take_ready_event(&mut self) -> Option<(Arc<StylesheetLinkClient>, bool)> {
        if self.event_phase != LinkLoadEventPhase::WaitingForCompletion {
            return None;
        }
        let successful = match self.completion() {
            StylesheetCompletionState::Pending => return None,
            StylesheetCompletionState::Succeeded => true,
            StylesheetCompletionState::Failed => false,
        };
        self.event_phase = LinkLoadEventPhase::Posted;
        Some((Arc::clone(&self.active_load), successful))
    }

    pub(super) fn consume_posted_event(&mut self, load: &Arc<StylesheetLinkClient>) -> bool {
        if self.event_phase != LinkLoadEventPhase::Posted
            || !StylesheetLinkClient::ptr_eq(&self.active_load, load)
        {
            return false;
        }
        self.event_phase = LinkLoadEventPhase::Dispatched;
        true
    }

    pub(super) fn accept_resource_completion(
        &mut self,
        load: &Arc<StylesheetLinkClient>,
        successful: bool,
    ) -> Option<LinkStyleSettlement> {
        if !StylesheetLinkClient::ptr_eq(&self.active_load, load) {
            return None;
        }
        self.resource_completion
            .settle(successful)
            .then(|| self.settlement_after_phase_change())
    }

    pub(super) fn accept_import_completion(
        &mut self,
        successful: bool,
    ) -> Option<LinkStyleSettlement> {
        self.import_completion
            .settle(successful)
            .then(|| self.settlement_after_phase_change())
    }

    fn settlement_after_phase_change(&mut self) -> LinkStyleSettlement {
        LinkStyleSettlement {
            ready_event: self.take_ready_event(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StylesheetCompletionState::{Failed, Pending, Succeeded};

    #[test]
    fn combined_completion_preserves_resource_then_import_dependency() {
        let cases = [
            ((Pending, Pending), Pending),
            ((Pending, Succeeded), Pending),
            ((Pending, Failed), Pending),
            ((Succeeded, Pending), Pending),
            ((Succeeded, Succeeded), Succeeded),
            ((Succeeded, Failed), Failed),
            ((Failed, Pending), Failed),
            ((Failed, Succeeded), Failed),
            ((Failed, Failed), Failed),
        ];
        for ((resource, imports), expected) in cases {
            assert_eq!(resource.followed_by(imports), expected);
        }
    }

    #[test]
    fn completion_state_settles_only_once() {
        let mut completion = Pending;
        assert!(completion.settle(true));
        assert_eq!(completion, Succeeded);
        assert!(!completion.settle(false));
        assert_eq!(completion, Succeeded);
    }
}
