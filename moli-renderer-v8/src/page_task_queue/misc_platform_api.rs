//! Exact Window/Document tasks from the HTML miscellaneous platform API
//! task source.
//!
//! This source is intentionally separate from timers, DOM manipulation, and
//! storage events. A Web API binding may publish an immutable task envelope,
//! but only the Page owner may authorize its exact Window/Document and the
//! selected-task dispatcher remains the sole task-completion authority.

use crate::runtime::PageOwnerTurnOutcome;

use super::{
    RendererPageWindowDocumentTask, RendererPageWindowDocumentTaskOwner,
    window_document_task_source::{
        RendererPageWindowDocumentTaskRoute, RendererPageWindowDocumentTaskSender,
        RendererPageWindowDocumentTaskSource,
    },
};

/// Host-local identity for one admitted miscellaneous-platform callback.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RendererPageMiscPlatformApiTaskId(u64);

impl RendererPageMiscPlatformApiTaskId {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// Concrete result already selected by the owning Web API algorithm.
///
/// The kind exists for exact envelope/payload matching and diagnostics. It
/// does not decide callback presence, owner currentness, or completion policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageMiscPlatformApiTaskKind {
    LegacyStorageUsageAndQuota,
    LegacyStorageGrantedQuota,
    LegacyStorageError,
    MediaTrackConstraints,
    EncryptedMediaRequest,
}

pub(crate) type RendererPageMiscPlatformApiOwner = RendererPageWindowDocumentTaskOwner;
pub(crate) type RendererPageMiscPlatformApiTask = RendererPageWindowDocumentTask<
    RendererPageMiscPlatformApiTaskId,
    RendererPageMiscPlatformApiTaskKind,
>;
pub(super) type RendererPageMiscPlatformApiRoute = RendererPageWindowDocumentTaskRoute<
    RendererPageMiscPlatformApiTaskId,
    RendererPageMiscPlatformApiTaskKind,
>;
pub(crate) type RendererPageMiscPlatformApiSender = RendererPageWindowDocumentTaskSender<
    RendererPageMiscPlatformApiTaskId,
    RendererPageMiscPlatformApiTaskKind,
>;
pub(super) type RendererPageMiscPlatformApiSource = RendererPageWindowDocumentTaskSource<
    RendererPageMiscPlatformApiTaskId,
    RendererPageMiscPlatformApiTaskKind,
>;

/// Execution fact produced after exact Window/Document authorization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageMiscPlatformApiTargetEffect {
    CallbackInvokedForCurrentOwner,
    CurrentOwnerCallbackRetired,
    DiscardedStaleOwner {
        current_owner: Option<RendererPageMiscPlatformApiOwner>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PageMiscPlatformApiTurnAction {
    pub(crate) owner: RendererPageMiscPlatformApiOwner,
    pub(crate) task_id: RendererPageMiscPlatformApiTaskId,
    pub(crate) kind: RendererPageMiscPlatformApiTaskKind,
    pub(crate) target_effect: PageMiscPlatformApiTargetEffect,
}

pub(crate) type PageMiscPlatformApiTurnOutcome =
    PageOwnerTurnOutcome<PageMiscPlatformApiTurnAction>;
