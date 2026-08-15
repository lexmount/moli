use super::{
    CdpConnection, CommandOwnerScope, NoneSessionOwnerRouteOverrideScope,
    TargetPageProtocolAttachmentIdentity, state::PendingBidiChannelListener,
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
}

impl BidiChannelPageOwner {
    /// Captures the Page currently addressed by `session_id`.
    ///
    /// Callers using the implicit `None` session must invoke this while the
    /// target's owner-route override is installed. `owner_scope` freezes that
    /// route, while `attachment` freezes the target Page residence and exact
    /// protocol session.
    pub(crate) fn capture(conn: &CdpConnection, session_id: Option<&str>) -> Option<Self> {
        Some(Self {
            owner_scope: CommandOwnerScope::capture(conn, session_id),
            attachment: conn.target_page_protocol_attachment_identity_for_session(session_id)?,
        })
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.attachment.session_id()
    }

    pub(crate) fn target_id(&self) -> Option<&str> {
        self.attachment.page_owner().target_id()
    }

    pub(crate) fn enter<'a>(
        &self,
        conn: &'a mut CdpConnection,
    ) -> NoneSessionOwnerRouteOverrideScope<'a> {
        self.owner_scope.enter(conn)
    }

    /// Checks the captured Page attachment under its owner route.
    ///
    /// Callers handling an implicit session must first enter this binding with
    /// `enter`; otherwise a surrounding command's route could answer the
    /// currentness query for the wrong Page.
    pub(crate) fn is_current(&self, conn: &CdpConnection) -> bool {
        conn.target_page_protocol_attachment_identity_is_current(&self.attachment)
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

    pub(crate) fn from_boxed(
        owner: BidiChannelPageOwner,
        listener: Box<PendingBidiChannelListener>,
    ) -> Self {
        Self { owner, listener }
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
        conn.browser_context = Some(browser_context);
        conn.runtime_session_owner_slot_mut(Some("SID-owner"))
            .expect("test runtime slot")
            .set_page_attachment_id_for_test(1);
        conn
    }

    #[test]
    fn page_owner_rejects_replacement_attachment() {
        let mut conn = connection_with_page_session();
        let owner =
            BidiChannelPageOwner::capture(&conn, Some("SID-owner")).expect("test Page attachment");
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
        let owner =
            BidiChannelPageOwner::capture(&conn, Some("SID-owner")).expect("test Page attachment");

        assert_eq!(
            conn.browser_context
                .as_mut()
                .expect("test browser context")
                .detach_active_session()
                .as_deref(),
            Some("SID-owner")
        );

        assert!(
            !owner.is_current(&conn),
            "a channel action must not survive its exact protocol attachment"
        );
    }

    #[test]
    fn implicit_page_owner_reenters_its_frozen_route_and_restores_the_caller() {
        let mut conn = connection_with_page_session();
        let owner_route = crate::conn::CdpSessionRoute::ActiveTarget {
            browser_context_id: "BID-owner".to_owned(),
            target_id: Some("TID-owner".to_owned()),
        };
        let owner = {
            let mut scope = conn.scoped_none_session_owner_route_override(owner_route.clone());
            BidiChannelPageOwner::capture(scope.conn_mut(), None)
                .expect("implicit owner must capture the scoped Page")
        };
        let caller_route = crate::conn::CdpSessionRoute::Browser;
        conn.replace_none_session_owner_route_override(Some(caller_route.clone()));

        {
            let mut scope = owner.enter(&mut conn);
            assert_eq!(
                scope.conn_mut().none_session_owner_route_override(),
                Some(owner_route),
                "deferred work must not follow the caller's current implicit route"
            );
            assert!(
                owner.is_current(scope.conn_mut()),
                "the frozen route must still resolve the captured Page attachment"
            );
        }

        assert_eq!(
            conn.none_session_owner_route_override(),
            Some(caller_route),
            "executing exact owner work must restore the surrounding route"
        );
    }
}
