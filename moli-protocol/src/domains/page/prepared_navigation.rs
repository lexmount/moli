use moli_core::RendererRuntimeCommandCausalIdentity;
use moli_core::page::{
    RendererDocumentSourcedSameDocumentNavigation,
    RendererDocumentSourcedTopLevelLocationNavigation, RendererPendingSameDocumentNavigation,
};

use crate::conn::TargetPageResidenceIdentity;

/// A same-Document protocol handoff bound to the exact target-local Page
/// residence from which it was captured.
///
/// `source_document` inside `navigation` is causal metadata. The Page
/// residence is the apply authority because `document.open()` replaces the
/// Document without undoing an already-applied history mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PagePreparedSameDocumentNavigation {
    owner: TargetPageResidenceIdentity,
    navigation: RendererDocumentSourcedSameDocumentNavigation,
}

impl PagePreparedSameDocumentNavigation {
    pub(super) fn new(
        owner: TargetPageResidenceIdentity,
        navigation: RendererDocumentSourcedSameDocumentNavigation,
    ) -> Self {
        Self { owner, navigation }
    }

    pub(super) fn owner(&self) -> &TargetPageResidenceIdentity {
        &self.owner
    }

    pub(super) fn into_navigation(self) -> RendererPendingSameDocumentNavigation {
        self.navigation.into_navigation()
    }
}

/// A renderer-requested top-level navigation bound to the exact target-local
/// Page residence that produced the prepared action.
///
/// Keeping the source Document and Page residence distinct is intentional: the
/// request survives `document.open()` in the same Page, but must not navigate a
/// target after that Page has been retired or replaced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PagePreparedTopLevelLocationNavigation {
    owner: TargetPageResidenceIdentity,
    navigation: RendererDocumentSourcedTopLevelLocationNavigation,
}

impl PagePreparedTopLevelLocationNavigation {
    pub(super) fn new(
        owner: TargetPageResidenceIdentity,
        navigation: RendererDocumentSourcedTopLevelLocationNavigation,
    ) -> Self {
        Self { owner, navigation }
    }

    pub(super) fn into_parts(
        self,
    ) -> (
        TargetPageResidenceIdentity,
        RendererDocumentSourcedTopLevelLocationNavigation,
    ) {
        (self.owner, self.navigation)
    }

    pub(super) fn runtime_command_cause(&self) -> Option<&RendererRuntimeCommandCausalIdentity> {
        self.navigation.runtime_command_cause()
    }
}
