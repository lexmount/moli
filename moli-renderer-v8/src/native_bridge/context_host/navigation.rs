use super::JsContextHost;
use crate::page_task_queue::RendererTopLevelNavigationHandoff;
use crate::runtime::{
    RendererBrowserContextRuntime, RendererDocumentLifecycleIdentity,
    RendererDocumentLifecycleJournalHandle, RendererPendingTopLevelHistoryTraversal,
    RendererRuntimeCommandCausalIdentity,
};
use crate::service_worker_runtime::{ServiceWorkerClientId, ServiceWorkerClientNavigateError};
use moli_fetch::BrowserNavigationRequestKind;
use moli_page_types::{NavigationHistoryEntrySeed, SameDocumentHistoryUpdate};
use url::Url;

pub(crate) struct PendingReservedServiceWorkerClient {
    client_id: Option<ServiceWorkerClientId>,
    browser_context_runtime: RendererBrowserContextRuntime,
}

impl PendingReservedServiceWorkerClient {
    fn new(
        client_id: ServiceWorkerClientId,
        browser_context_runtime: RendererBrowserContextRuntime,
    ) -> Self {
        Self {
            client_id: Some(client_id),
            browser_context_runtime,
        }
    }

    pub(crate) fn release(mut self) -> ServiceWorkerClientId {
        self.client_id
            .take()
            .expect("pending reserved service worker client should release at most once")
    }
}

impl Drop for PendingReservedServiceWorkerClient {
    fn drop(&mut self) {
        if let Some(client_id) = self.client_id.take() {
            self.browser_context_runtime
                .unregister_service_worker_client(client_id);
        }
    }
}

pub(crate) struct PendingLocationNavigation {
    /// Exact producer-to-owner handoff identity for this value of the Page's
    /// unique pending top-level navigation slot.
    pub(crate) handoff: RendererTopLevelNavigationHandoff,
    /// Exact Document that requested this navigation, retained as causal
    /// metadata across a possible `document.open()` replacement. Standalone
    /// ScriptVm fixtures may not install a Page lifecycle journal; PageVm
    /// command capture treats that absence as an invariant violation rather
    /// than guessing an origin after the fact. Protocol apply authorization is
    /// established separately by the target-local Page residence.
    pub(crate) source_document: Option<RendererDocumentLifecycleIdentity>,
    /// Exact Runtime command whose synchronous V8 dispatch requested this
    /// navigation. Later tasks on the same Page do not inherit the identity.
    pub(crate) runtime_command_cause: Option<RendererRuntimeCommandCausalIdentity>,
    /// The complete browser request. URL-only callers use the GET wrapper;
    /// form POST callers retain raw bytes and their generated Content-Type.
    pub(crate) url: Url,
    pub(crate) request_method: String,
    pub(crate) request_body: Option<Vec<u8>>,
    pub(crate) request_headers: Vec<(String, String)>,
    pub(crate) browser_navigation_kind: BrowserNavigationRequestKind,
    pub(crate) entry_seed: Option<NavigationHistoryEntrySeed>,
    pub(crate) reserved_service_worker_client: Option<PendingReservedServiceWorkerClient>,
    pub(crate) service_worker_client_navigate:
        Option<crate::types::ServiceWorkerClientNavigateContinuation>,
}

pub(crate) enum PendingTopLevelNavigation {
    Location(Box<PendingLocationNavigation>),
    #[cfg(test)]
    HistoryTraversal(RendererPendingTopLevelHistoryTraversal),
}

impl JsContextHost {
    /// Replace the dynamically scoped Runtime command cause and return the
    /// previous scope for exact restoration after V8 dispatch.
    ///
    /// The borrow never crosses V8 entry. Nested inspector work therefore
    /// restores its predecessor instead of leaking command identity into a
    /// later HTML task.
    pub(crate) fn replace_active_runtime_command_cause(
        &mut self,
        cause: Option<RendererRuntimeCommandCausalIdentity>,
    ) -> Option<RendererRuntimeCommandCausalIdentity> {
        std::mem::replace(&mut self.active_runtime_command_cause, cause)
    }

    pub(crate) fn replace_active_inspector_dispatch(&mut self, active: bool) -> bool {
        std::mem::replace(&mut self.active_inspector_dispatch, active)
    }

    pub(crate) fn record_pending_location_navigation(
        &mut self,
        url: Url,
        entry_seed: Option<NavigationHistoryEntrySeed>,
    ) {
        self.record_pending_location_navigation_with_kind(
            url,
            entry_seed,
            BrowserNavigationRequestKind::Navigate,
        );
    }

    pub(crate) fn record_pending_location_navigation_with_kind(
        &mut self,
        url: Url,
        entry_seed: Option<NavigationHistoryEntrySeed>,
        browser_navigation_kind: BrowserNavigationRequestKind,
    ) {
        self.record_pending_location_navigation_request(
            url,
            "GET".to_owned(),
            None,
            Vec::new(),
            entry_seed,
            browser_navigation_kind,
        );
    }

    pub(crate) fn record_pending_location_navigation_request(
        &mut self,
        url: Url,
        request_method: String,
        request_body: Option<Vec<u8>>,
        request_headers: Vec<(String, String)>,
        entry_seed: Option<NavigationHistoryEntrySeed>,
        browser_navigation_kind: BrowserNavigationRequestKind,
    ) {
        self.clear_pending_top_level_navigation();
        let handoff = self.top_level_navigation_handoff_tx.next_handoff();
        let reserved_service_worker_client =
            self.pending_reserved_service_worker_top_level_client_for_navigation(&url);
        self.pending_top_level_navigation = Some(PendingTopLevelNavigation::Location(Box::new(
            PendingLocationNavigation {
                handoff,
                source_document: self
                    .root_document_lifecycle
                    .as_ref()
                    .map(RendererDocumentLifecycleJournalHandle::identity),
                runtime_command_cause: self.active_runtime_command_cause.clone(),
                url,
                request_method,
                request_body,
                request_headers,
                browser_navigation_kind,
                entry_seed,
                reserved_service_worker_client,
                service_worker_client_navigate: None,
            },
        )));
        self.handoff_ordinary_page_turn_navigation(handoff);
    }

    pub(crate) fn record_pending_service_worker_client_navigation(
        &mut self,
        url: Url,
        continuation: crate::types::ServiceWorkerClientNavigateContinuation,
    ) {
        self.clear_pending_top_level_navigation();
        let handoff = self.top_level_navigation_handoff_tx.next_handoff();
        let reserved_service_worker_client =
            self.pending_reserved_service_worker_top_level_client_for_navigation(&url);
        self.pending_top_level_navigation = Some(PendingTopLevelNavigation::Location(Box::new(
            PendingLocationNavigation {
                handoff,
                source_document: self
                    .root_document_lifecycle
                    .as_ref()
                    .map(RendererDocumentLifecycleJournalHandle::identity),
                runtime_command_cause: None,
                url,
                request_method: "GET".to_owned(),
                request_body: None,
                request_headers: Vec::new(),
                browser_navigation_kind: BrowserNavigationRequestKind::Navigate,
                entry_seed: None,
                reserved_service_worker_client,
                service_worker_client_navigate: Some(continuation),
            },
        )));
        self.handoff_ordinary_page_turn_navigation(handoff);
    }

    pub(crate) fn has_pending_location_navigation(&self) -> bool {
        matches!(
            self.pending_top_level_navigation.as_ref(),
            Some(PendingTopLevelNavigation::Location(_))
        )
    }

    pub(crate) fn pending_location_navigation_scheme_is(&self, scheme: &str) -> bool {
        self.pending_top_level_navigation
            .as_ref()
            .is_some_and(|pending| {
                matches!(
                    pending,
                    PendingTopLevelNavigation::Location(pending)
                        if pending.url.scheme() == scheme
                )
            })
    }

    pub(crate) fn pending_location_navigation_runtime_command_cause(
        &self,
    ) -> Option<RendererRuntimeCommandCausalIdentity> {
        let Some(PendingTopLevelNavigation::Location(pending)) =
            self.pending_top_level_navigation.as_ref()
        else {
            return None;
        };
        pending.runtime_command_cause.clone()
    }

    pub(crate) fn pending_location_navigation_handoff(
        &self,
    ) -> Option<RendererTopLevelNavigationHandoff> {
        let Some(PendingTopLevelNavigation::Location(pending)) =
            self.pending_top_level_navigation.as_ref()
        else {
            return None;
        };
        Some(pending.handoff)
    }

    pub(crate) fn take_pending_location_navigation(&mut self) -> Option<PendingLocationNavigation> {
        if !self.has_pending_location_navigation() {
            return None;
        }
        let Some(PendingTopLevelNavigation::Location(pending)) =
            self.pending_top_level_navigation.take()
        else {
            unreachable!("pending top-level navigation kind changed without an intervening call");
        };
        Some(*pending)
    }

    pub(crate) fn clear_pending_location_navigation(&mut self) {
        if self.has_pending_location_navigation() {
            self.clear_pending_top_level_navigation();
        }
    }

    pub(crate) fn record_pending_top_level_history_traversal(&mut self, delta: i64) {
        self.clear_pending_top_level_navigation();
        let traversal = RendererPendingTopLevelHistoryTraversal { delta };
        if self.append_live_turn_owner_action(
            crate::runtime::RendererOwnerAction::TopLevelHistoryTraversal(traversal),
        ) {
            return;
        }
        #[cfg(test)]
        {
            self.pending_top_level_navigation =
                Some(PendingTopLevelNavigation::HistoryTraversal(traversal));
        }
        #[cfg(not(test))]
        panic!("a production history traversal must have a concrete renderer output sink");
    }

    #[cfg(test)]
    pub(crate) fn take_pending_top_level_history_traversal(
        &mut self,
    ) -> Option<RendererPendingTopLevelHistoryTraversal> {
        if !matches!(
            self.pending_top_level_navigation.as_ref(),
            Some(PendingTopLevelNavigation::HistoryTraversal(_))
        ) {
            return None;
        }
        let Some(PendingTopLevelNavigation::HistoryTraversal(pending)) =
            self.pending_top_level_navigation.take()
        else {
            unreachable!("pending top-level navigation kind changed without an intervening call");
        };
        Some(pending)
    }

    pub(crate) fn clear_pending_top_level_navigation(&mut self) {
        let pending = self.pending_top_level_navigation.take();
        let Some(PendingTopLevelNavigation::Location(pending)) = pending else {
            return;
        };
        let pending = *pending;
        let Some(continuation) = pending.service_worker_client_navigate else {
            return;
        };
        self.browser_context_runtime
            .service_worker_runtime()
            .enqueue_client_navigate_completed(
                crate::types::ServiceWorkerClientNavigateCompletion {
                    request_id: continuation.request_id,
                    source_version_id: continuation.source_version_id,
                    source_run: continuation.source_run,
                    result: Err(ServiceWorkerClientNavigateError::type_error(
                        "The navigation was canceled.",
                    )),
                },
            );
    }

    fn pending_reserved_service_worker_top_level_client_for_navigation(
        &mut self,
        url: &Url,
    ) -> Option<PendingReservedServiceWorkerClient> {
        self.register_reserved_service_worker_top_level_client_for_navigation(url)
            .map(|client_id| {
                PendingReservedServiceWorkerClient::new(
                    client_id,
                    self.browser_context_runtime.clone(),
                )
            })
    }

    pub(crate) fn record_same_document_navigation(
        &mut self,
        url: &Url,
        navigation_type: &str,
        history_update: SameDocumentHistoryUpdate,
    ) {
        let Some(source_document) = self
            .root_document_lifecycle
            .as_ref()
            .map(RendererDocumentLifecycleJournalHandle::identity)
        else {
            // Standalone ScriptVm fixtures intentionally have no Page
            // lifecycle owner. Their History/Location mutation still applies,
            // but they cannot publish a Page-scoped protocol handoff without
            // inventing a Document identity.
            tracing::debug!(
                url = url.as_str(),
                navigation_type,
                "same-document navigation has no Page lifecycle owner for protocol publication"
            );
            return;
        };
        let navigation = crate::runtime::RendererDocumentSourcedSameDocumentNavigation::new(
            source_document,
            crate::runtime::RendererPendingSameDocumentNavigation {
                url: url.to_string(),
                navigation_type: navigation_type.to_owned(),
                history_update,
            },
        );
        if !self.append_live_turn_owner_action(
            crate::runtime::RendererOwnerAction::SameDocumentNavigation(navigation),
        ) {
            // Standalone ScriptVm fixtures deliberately have no renderer
            // output stream. The history mutation is already complete; there
            // is no protocol owner to notify and no compatibility queue to
            // populate. Production Page turns always have either a command
            // recorder or their Page journal.
            tracing::debug!(
                url = url.as_str(),
                navigation_type,
                "same-document navigation has no renderer output sink"
            );
        }
    }
}
