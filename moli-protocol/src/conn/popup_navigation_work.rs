use super::{CdpConnection, CdpSessionRoute, CommandOwnerScope, TargetPageResidenceIdentity};

/// Immutable authority for one popup destination navigation.
///
/// A target id is not sufficient authorization: CDP sessions and target-local
/// state can survive replacement of the installed renderer `Page`. Retaining
/// the exact residence generation makes an activation captured for the
/// initial empty Document incapable of navigating a later replacement Page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PopupTargetNavigationClaimIdentity {
    page_owner: TargetPageResidenceIdentity,
    browser_context_id: String,
    target_id: String,
    request: Box<moli_core::page::RendererTopLevelNavigationRequest>,
    referrer: Option<String>,
    document_referrer: Option<String>,
    navigation_history_entry_seed: Option<Box<moli_page_types::NavigationHistoryEntrySeed>>,
    kind: PopupTargetNavigationKind,
    drain_pending_javascript_tasks_before_commit: bool,
}

impl PopupTargetNavigationClaimIdentity {
    pub(crate) fn page_owner(&self) -> &TargetPageResidenceIdentity {
        &self.page_owner
    }

    pub(crate) fn browser_context_id(&self) -> &str {
        &self.browser_context_id
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    pub(crate) fn url(&self) -> &str {
        self.request.url()
    }

    pub(crate) fn request(&self) -> &moli_core::page::RendererTopLevelNavigationRequest {
        &self.request
    }

    pub(crate) fn referrer(&self) -> Option<&str> {
        self.referrer.as_deref()
    }

    pub(crate) fn document_referrer(&self) -> Option<&str> {
        self.document_referrer.as_deref()
    }

    pub(crate) fn navigation_history_entry_seed(
        &self,
    ) -> Option<&moli_page_types::NavigationHistoryEntrySeed> {
        self.navigation_history_entry_seed.as_deref()
    }

    pub(crate) fn kind(&self) -> PopupTargetNavigationKind {
        self.kind
    }

    pub(crate) fn drain_pending_javascript_tasks_before_commit(&self) -> bool {
        self.drain_pending_javascript_tasks_before_commit
    }
}

/// Navigation requested by an already-accepted auxiliary browsing-context
/// action.
///
/// Creating or resolving the target is part of the renderer output that
/// precedes the causing Runtime response. Loading the requested URL is not.
/// Blink's `LocalDOMWindow::open()` resolves the target, invokes `Navigate()`,
/// and returns the Window without waiting for the network load or Document
/// commit. Moli's navigation helper can itself become asynchronous, so
/// protocol projection must hand that work to the owner scheduler instead of
/// awaiting it while the opener's output cursor is being projected. Keeping
/// the frozen URL and exact target route in this move-only action makes that
/// boundary explicit.
#[derive(Debug)]
pub(crate) struct PopupTargetNavigationOwnerAction {
    owner_scope: CommandOwnerScope,
    claim: PopupTargetNavigationClaimIdentity,
    /// Move-only completion authority for a ServiceWorker `Clients.openWindow()`
    /// producer. This deliberately does not live in `claim`: consumed claims
    /// remain as target-local currentness tombstones for the lifetime of the
    /// Page and must not retain a worker Promise completion carrier.
    service_worker_clients_open_window_continuation:
        Option<moli_core::page::RendererServiceWorkerClientsOpenWindowContinuation>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PopupTargetNavigationKind {
    InitialDocument,
    NamedTargetReuse,
}

/// Target-local lifecycle of the initial popup navigation authority.
///
/// `Consumed` is deliberately retained as a tombstone. Once a captured
/// activation is stale, generic entry points such as `Page.enable` must not
/// reconstruct it from the mutable target URL and apply it to a newer Page.
#[derive(Debug)]
pub(crate) enum PopupTargetNavigationAuthorityState {
    Held(PopupTargetNavigationOwnerAction),
    Published(PopupTargetNavigationClaimIdentity),
    Consumed(PopupTargetNavigationClaimIdentity),
    /// The auxiliary target was created, but its producer explicitly carried
    /// no destination navigation. This tombstone prevents generic Page entry
    /// points from reconstructing a request from the target's observational
    /// URL.
    NoDestination(TargetPageResidenceIdentity),
}

impl PopupTargetNavigationAuthorityState {
    pub(crate) fn page_owner(&self) -> &TargetPageResidenceIdentity {
        match self {
            Self::Held(action) => action.page_owner(),
            Self::Published(claim) | Self::Consumed(claim) => claim.page_owner(),
            Self::NoDestination(page_owner) => page_owner,
        }
    }
}

impl PopupTargetNavigationOwnerAction {
    pub(crate) fn capture(
        conn: &mut CdpConnection,
        browser_context_id: &str,
        target_id: &str,
        request: moli_core::page::RendererTopLevelNavigationRequest,
        referrer: Option<String>,
        document_referrer: Option<String>,
        navigation_history_entry_seed: Option<moli_page_types::NavigationHistoryEntrySeed>,
        kind: PopupTargetNavigationKind,
        service_worker_clients_open_window_continuation: Option<
            moli_core::page::RendererServiceWorkerClientsOpenWindowContinuation,
        >,
        drain_pending_javascript_tasks_before_commit: bool,
    ) -> Option<Self> {
        let route = conn.target_session_route_for_target_id(target_id)?;
        if route.browser_context_id() != Some(browser_context_id) {
            return None;
        }
        // The Page identity below is the authorization boundary. Use the
        // auxiliary target capability as the residence route so a foreground
        // activation may move the same Page from the background slot to the
        // active slot while its navigation is in flight. A concrete
        // `BackgroundTarget` route would become stale at that move and strand
        // the already-accepted destination work.
        let owner_scope = CommandOwnerScope::from_session_and_owner_route(
            None,
            Some(CdpSessionRoute::AuxiliaryTarget {
                browser_context_id: browser_context_id.to_owned(),
                target_id: target_id.to_owned(),
            }),
        );
        let page_owner = {
            let mut route_scope = conn.scoped_none_session_owner_route_override(route);
            route_scope
                .conn_mut()
                .target_page_residence_identity_for_session(None)?
        };
        if page_owner.browser_context_id() != browser_context_id
            || page_owner.target_id() != Some(target_id)
        {
            return None;
        }
        Some(Self {
            owner_scope,
            claim: PopupTargetNavigationClaimIdentity {
                page_owner,
                browser_context_id: browser_context_id.to_owned(),
                target_id: target_id.to_owned(),
                request: Box::new(request),
                referrer,
                document_referrer,
                navigation_history_entry_seed: navigation_history_entry_seed.map(Box::new),
                kind,
                drain_pending_javascript_tasks_before_commit,
            },
            service_worker_clients_open_window_continuation,
        })
    }

    pub(crate) fn browser_context_id(&self) -> &str {
        self.claim.browser_context_id()
    }

    pub(crate) fn target_id(&self) -> &str {
        self.claim.target_id()
    }

    pub(crate) fn url(&self) -> &str {
        self.claim.url()
    }

    pub(crate) fn kind(&self) -> PopupTargetNavigationKind {
        self.claim.kind()
    }

    pub(crate) fn page_owner(&self) -> &TargetPageResidenceIdentity {
        self.claim.page_owner()
    }

    pub(crate) fn claim_identity(&self) -> &PopupTargetNavigationClaimIdentity {
        &self.claim
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        CommandOwnerScope,
        PopupTargetNavigationClaimIdentity,
        Option<moli_core::page::RendererServiceWorkerClientsOpenWindowContinuation>,
    ) {
        (
            self.owner_scope,
            self.claim,
            self.service_worker_clients_open_window_continuation,
        )
    }
}

impl CdpConnection {
    pub(crate) fn stage_popup_target_without_destination_navigation(
        &mut self,
        target_id: &str,
    ) -> bool {
        let Some(route) = self.target_session_route_for_target_id(target_id) else {
            return false;
        };
        let mut route_scope = self.scoped_none_session_owner_route_override(route);
        let conn = route_scope.conn_mut();
        let Some(page_owner) = conn.target_page_residence_identity_for_session(None) else {
            return false;
        };
        if page_owner.target_id() != Some(target_id) {
            return false;
        }
        conn.runtime_session_owner_slot_mut(None)
            .is_ok_and(|slot| slot.stage_popup_target_without_destination_navigation(page_owner))
    }

    /// Installs the initial popup destination as target-local held authority.
    /// The action is accepted only while its exact Page residence is current.
    pub(crate) fn stage_initial_popup_target_navigation_owner_action(
        &mut self,
        action: PopupTargetNavigationOwnerAction,
    ) -> bool {
        if action.kind() != PopupTargetNavigationKind::InitialDocument {
            return false;
        }
        let owner_scope = action.owner_scope.clone();
        let page_owner = action.page_owner().clone();
        let mut route_scope = owner_scope.enter(self);
        let conn = route_scope.conn_mut();
        if !conn.target_page_residence_identity_is_current_for_session(None, &page_owner) {
            return false;
        }
        conn.runtime_session_owner_slot_mut(None)
            .is_ok_and(|slot| slot.stage_initial_popup_target_navigation_owner_action(action))
    }

    /// Releases a held action through the exact target route used at target
    /// admission. The target slot retains a published claim until completion.
    pub(crate) fn take_held_popup_target_navigation_owner_action_for_target(
        &mut self,
        target_id: &str,
    ) -> Option<PopupTargetNavigationOwnerAction> {
        let route = self.target_session_route_for_target_id(target_id)?;
        let mut route_scope = self.scoped_none_session_owner_route_override(route);
        route_scope
            .conn_mut()
            .take_held_popup_target_navigation_owner_action_for_session_owner(None)
    }

    /// Releases the held action addressed by a concrete Runtime/Page session.
    pub(crate) fn take_held_popup_target_navigation_owner_action_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<PopupTargetNavigationOwnerAction> {
        self.runtime_session_owner_slot_mut(session_id)
            .ok()?
            .take_held_initial_popup_target_navigation_owner_action()
    }

    pub(crate) fn consume_published_popup_target_navigation_claim_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        expected: &PopupTargetNavigationClaimIdentity,
    ) -> bool {
        self.runtime_session_owner_slot_mut(session_id)
            .is_ok_and(|slot| slot.consume_published_popup_target_navigation_claim(expected))
    }

    pub(crate) fn runtime_session_owner_has_popup_target_navigation_authority(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        self.runtime_session_owner_slot(session_id)
            .is_ok_and(|slot| slot.has_popup_target_navigation_authority())
    }
}
