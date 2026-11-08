use super::{
    CdpConnection, CommandOwnerScope, TargetPageProtocolAttachmentIdentity,
    state::PendingBidiChannelListener,
};

/// Exact Page attachment that owns one WebDriver BiDi channel listener.
///
/// BiDi channel arguments currently require a browsing-context target and a
/// realm with a `window_context_id`; Worker realms are rejected before a
/// listener is created. Keeping that restriction in this Page-specific type
/// prevents a later deferred turn from following a bare session id to a
/// replacement Page. If Worker-realm channels are added, they need their own
/// exact attachment variant rather than weakening this binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BidiChannelPageOwner {
    owner_scope: CommandOwnerScope,
    attachment: TargetPageProtocolAttachmentIdentity,
    host: crate::conn::PageTargetHostIdentity,
    binding: Option<crate::conn::SessionDisposalWait>,
    renderer_page: Option<crate::conn::RendererPageResidenceIdentity>,
    renderer_attachment: Option<moli_core::page::RendererAgentAttachmentId>,
}

impl BidiChannelPageOwner {
    pub(crate) fn capture_for_owner(
        conn: &CdpConnection,
        owner_scope: CommandOwnerScope,
    ) -> Option<Self> {
        let attachment = conn.target_page_protocol_attachment_identity_for_owner(&owner_scope)?;
        // A named binding is already exact. Implicit Browser/Context routes
        // must be narrowed once so later active-target selection cannot move
        // a listener or suppress cleanup of its original Page.
        let owner_scope = if owner_scope.session_id().is_none()
            && !matches!(
                owner_scope.explicit_route(),
                Some(crate::conn::CdpSessionRoute::PageTarget { .. })
            ) {
            CommandOwnerScope::for_page_residence(attachment.page_owner())
        } else {
            owner_scope
        };
        let session = owner_scope
            .session_id()
            .map(str::to_owned)
            .or_else(|| conn.target_renderer_runtime_inspector_session_id_for_owner(&owner_scope));
        let binding = session
            .as_deref()
            .and_then(|id| conn.session_binding_lifetime(id));
        Some(Self {
            host: conn.page_target_host_identity_for_owner(&owner_scope)?,
            binding,
            renderer_page: conn.renderer_page_residence_identity_for_owner(&owner_scope),
            renderer_attachment: conn.current_renderer_agent_attachment_id_for_owner(&owner_scope),
            attachment,
            owner_scope,
        })
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.attachment.session_id()
    }

    pub(crate) fn target_id(&self) -> Option<&str> {
        self.attachment.page_owner().target_id()
    }

    pub(crate) fn command_owner(&self) -> &CommandOwnerScope {
        &self.owner_scope
    }

    /// New listeners and proxy materialization share one lifecycle admission
    /// boundary. Existing receipts and object-group cleanup retain is_current
    /// authority while the original session is Closing.
    pub(crate) fn can_start_listener(&self, conn: &CdpConnection) -> bool {
        if !self.is_current(conn) || conn.target_is_closing_for_owner(&self.owner_scope) {
            return false;
        }
        let session = self
            .owner_scope
            .session_id()
            .map(str::to_owned)
            .or_else(|| {
                conn.target_renderer_runtime_inspector_session_id_for_owner(&self.owner_scope)
            });
        session
            .as_deref()
            .is_none_or(|id| !conn.session_is_closing(id))
    }

    /// Journal cancellation may retire an old call on the same host, but
    /// cannot follow a reused binding or a recreated target host.
    pub(crate) fn owns_current_host(&self, conn: &CdpConnection) -> bool {
        self.binding
            .as_ref()
            .is_none_or(|binding| !binding.is_closed())
            && conn
                .page_target_host_identity_for_owner(&self.owner_scope)
                .as_ref()
                .is_some_and(|host| self.host.matches(host))
    }

    /// Authorizes this original Page, native attachment and binding, using
    /// its frozen route rather than the connection's current active target.
    pub(crate) fn is_current(&self, conn: &CdpConnection) -> bool {
        self.owns_current_host(conn)
            && conn.target_page_protocol_attachment_identity_is_current(&self.attachment)
            && conn.renderer_page_residence_identity_for_owner(&self.owner_scope)
                == self.renderer_page
            && conn.current_renderer_agent_attachment_id_for_owner(&self.owner_scope)
                == self.renderer_attachment
    }
}

/// Long-lived listener state paired with the Page attachment that created it.
///
/// The binding travels through the pending inspector await and every listener
/// restart. A late reply therefore cannot capture whichever Page happens to
/// own the same session when the reply is routed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BidiChannelListenerResidence {
    owner: BidiChannelPageOwner,
    listener: Box<PendingBidiChannelListener>,
}

impl BidiChannelListenerResidence {
    pub(crate) fn new(owner: BidiChannelPageOwner, listener: PendingBidiChannelListener) -> Self {
        Self {
            owner,
            listener: Box::new(listener),
        }
    }

    pub(crate) fn owner(&self) -> &BidiChannelPageOwner {
        &self.owner
    }

    pub(crate) fn listener(&self) -> &PendingBidiChannelListener {
        self.listener.as_ref()
    }

    pub(crate) fn into_parts(self) -> (BidiChannelPageOwner, Box<PendingBidiChannelListener>) {
        (self.owner, self.listener)
    }

    pub(crate) fn channel_object_group(&self) -> &str {
        self.listener.channel_object_group()
    }
}

/// Concrete protocol-owner action published into `ProtocolSchedulerWork`.
///
/// This value owns its payload and exact route. It never asks a later turn to
/// scan a session-local queue. Each action receives one protocol publication
/// sequence, so listener starts and object-group releases preserve causal
/// publication order instead of being regrouped by action type.
#[derive(Debug)]
pub(crate) struct BidiChannelOwnerAction {
    owner: BidiChannelPageOwner,
    body: BidiChannelOwnerActionBody,
}

#[derive(Debug)]
pub(crate) enum BidiChannelOwnerActionBody {
    StartListener(Box<PendingBidiChannelListener>),
    ReleaseObjectGroup(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BidiChannelOwnerActionKind {
    StartListener,
    ReleaseObjectGroup,
}

impl BidiChannelOwnerAction {
    pub(crate) fn start_listener(residence: BidiChannelListenerResidence) -> Self {
        let (owner, listener) = residence.into_parts();
        Self {
            owner,
            body: BidiChannelOwnerActionBody::StartListener(listener),
        }
    }

    pub(crate) fn release_object_group(
        owner: BidiChannelPageOwner,
        object_group: impl Into<String>,
    ) -> Self {
        Self {
            owner,
            body: BidiChannelOwnerActionBody::ReleaseObjectGroup(object_group.into()),
        }
    }

    pub(crate) fn kind(&self) -> BidiChannelOwnerActionKind {
        match self.body {
            BidiChannelOwnerActionBody::StartListener(_) => {
                BidiChannelOwnerActionKind::StartListener
            }
            BidiChannelOwnerActionBody::ReleaseObjectGroup(_) => {
                BidiChannelOwnerActionKind::ReleaseObjectGroup
            }
        }
    }

    pub(crate) fn owner(&self) -> &BidiChannelPageOwner {
        &self.owner
    }

    pub(crate) fn into_parts(self) -> (BidiChannelPageOwner, BidiChannelOwnerActionBody) {
        (self.owner, self.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::BrowserContext;

    fn connection_with_page_session() -> CdpConnection {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-owner".to_owned());
        browser_context.set_active_target_id("TID-owner");
        browser_context.attach_active_session("SID-owner".to_owned());
        conn.install_browser_context_fixture_for_test(browser_context);
        conn.runtime_session_owner_slot_mut(Some("SID-owner"))
            .expect("test runtime slot")
            .set_page_attachment_id_for_test(1);
        conn
    }

    #[test]
    fn page_owner_rejects_rebound_session_and_recreated_host_with_the_same_ids() {
        for recreate_host in [false, true] {
            let mut conn = connection_with_page_session();
            let owner = BidiChannelPageOwner::capture_for_owner(
                &conn,
                CommandOwnerScope::for_session("SID-owner"),
            )
            .unwrap();
            if recreate_host {
                let context = conn.browser_context.as_mut().unwrap();
                let _old = context.take_page_target_for_close("TID-owner").unwrap();
                assert!(
                    context.insert_page_target_host(crate::conn::PageTargetHost::with_url(
                        "TID-owner".into(),
                        Some("SID-owner".into()),
                        "about:blank".into(),
                    ))
                );
                conn.runtime_session_owner_slot_mut(Some("SID-owner"))
                    .unwrap()
                    .set_page_attachment_id_for_test(1);
            } else {
                let route = conn.session_route(Some("SID-owner")).unwrap();
                conn.register_session_route_for_test("SID-owner", route);
            }
            assert!(
                !owner.is_current(&conn),
                "literal ids and Page counters do not authorize a successor binding or host (recreated host: {recreate_host})"
            );
        }
    }

    #[test]
    fn page_owner_rejects_replacement_attachment() {
        let mut conn = connection_with_page_session();
        let owner = BidiChannelPageOwner::capture_for_owner(
            &conn,
            CommandOwnerScope::for_session("SID-owner"),
        )
        .expect("test Page attachment");
        assert!(owner.is_current(&conn));

        conn.runtime_session_owner_slot_mut(Some("SID-owner"))
            .expect("test runtime slot")
            .replace_page_attachment_id_for_test();

        assert!(
            !owner.is_current(&conn),
            "a channel action must not follow its session to a replacement Page"
        );
    }

    #[test]
    fn page_owner_rejects_detached_session() {
        let mut conn = connection_with_page_session();
        let owner = BidiChannelPageOwner::capture_for_owner(
            &conn,
            CommandOwnerScope::for_session("SID-owner"),
        )
        .expect("test Page attachment");

        assert_eq!(
            conn.browser_context
                .as_mut()
                .expect("test browser context")
                .detach_active_session()
                .as_deref(),
            Some("SID-owner")
        );
        conn.rollback_attached_session_without_event("SID-owner");

        assert!(
            !owner.is_current(&conn),
            "a channel action must not survive its exact protocol attachment"
        );
    }

    #[test]
    fn browser_route_owner_keeps_its_original_target_after_selection_changes() {
        let mut conn = connection_with_page_session();
        let owner = BidiChannelPageOwner::capture_for_owner(
            &conn,
            CommandOwnerScope::for_route(crate::conn::CdpSessionRoute::Browser),
        )
        .unwrap();
        conn.browser_context
            .as_mut()
            .unwrap()
            .stage_background_target(
                "TID-next".into(),
                Some("SID-next".into()),
                "about:blank".into(),
                None,
                None,
            );
        conn.browser_context
            .as_mut()
            .unwrap()
            .set_active_target_id("TID-next");
        assert!(
            owner.is_current(&conn),
            "active selection cannot move or retire the original channel owner"
        );
        assert_eq!(owner.target_id(), Some("TID-owner"));
    }

    #[test]
    fn implicit_page_owner_retains_its_frozen_route() {
        let conn = connection_with_page_session();
        let owner_route = crate::conn::CdpSessionRoute::PageTarget {
            browser_context_id: "BID-owner".to_owned(),
            target_id: "TID-owner".to_owned(),
            session_key: moli_page_types::DevToolsSessionKey::Primary,
        };
        let owner = BidiChannelPageOwner::capture_for_owner(
            &conn,
            CommandOwnerScope::for_route(owner_route.clone()),
        )
        .expect("implicit owner must capture the scoped Page");

        assert_eq!(
            owner.command_owner().explicit_route(),
            Some(&owner_route),
            "deferred work must retain its exact Page route"
        );
        assert!(owner.is_current(&conn));
    }
}
