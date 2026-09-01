use super::{
    CdpConnection, CdpSessionRoute, TargetPageProtocolAttachmentIdentity,
    TargetPageResidenceIdentity,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandOwnerScope {
    identity: CommandOwnerIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CommandOwnerIdentity {
    Session(String),
    Route(CdpSessionRoute),
}

impl CommandOwnerScope {
    pub(crate) fn capture(conn: &CdpConnection, session_id: Option<&str>) -> Self {
        match session_id {
            Some(session_id) => Self::for_session(session_id),
            None => Self::for_route(Self::capture_default_route(conn)),
        }
    }

    pub(crate) fn capture_for_route(
        conn: &CdpConnection,
        session_id: Option<&str>,
        owner_route: Option<&CdpSessionRoute>,
    ) -> Self {
        match (session_id, owner_route) {
            (Some(session_id), _) => Self::for_session(session_id),
            (None, Some(route)) => Self::for_route(route.clone()),
            (None, None) => Self::capture(conn, None),
        }
    }

    fn capture_default_route(conn: &CdpConnection) -> CdpSessionRoute {
        let Some(browser_context) = conn.browser_context.as_ref() else {
            return CdpSessionRoute::Browser;
        };
        match browser_context.active_target_id_owned() {
            Some(target_id) => CdpSessionRoute::PageTarget {
                browser_context_id: browser_context.id.clone(),
                target_id,
                is_attached_session: false,
            },
            None => CdpSessionRoute::BrowserContext {
                browser_context_id: browser_context.id.clone(),
            },
        }
    }

    pub(crate) fn for_session(session_id: &str) -> Self {
        Self {
            identity: CommandOwnerIdentity::Session(session_id.to_owned()),
        }
    }

    pub(crate) fn for_route(route: CdpSessionRoute) -> Self {
        Self {
            identity: CommandOwnerIdentity::Route(route),
        }
    }

    pub(crate) fn for_page_attachment(attachment: &TargetPageProtocolAttachmentIdentity) -> Self {
        if let Some(session_id) = attachment.session_id() {
            return Self::for_session(session_id);
        }
        Self::for_page_residence(attachment.page_owner())
    }

    pub(crate) fn for_page_residence(page: &TargetPageResidenceIdentity) -> Self {
        let route = match page.target_id() {
            Some(target_id) => CdpSessionRoute::PageTarget {
                browser_context_id: page.browser_context_id().to_owned(),
                target_id: target_id.to_owned(),
                is_attached_session: false,
            },
            None => CdpSessionRoute::BrowserContext {
                browser_context_id: page.browser_context_id().to_owned(),
            },
        };
        Self::for_route(route)
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        match &self.identity {
            CommandOwnerIdentity::Session(session_id) => Some(session_id),
            CommandOwnerIdentity::Route(_) => None,
        }
    }

    /// Returns the explicit route captured for a command without a session.
    ///
    /// A concrete CDP session remains authoritative through `session_id`; the
    /// route freezes Chromium's primary Page attachment at command admission
    /// whenever no more explicit route was supplied. `Browser` and
    /// `BrowserContext` remain explicit authorities rather than a missing
    /// owner that can silently acquire a later active Page.
    pub(crate) fn session_owner_route(&self) -> Option<&CdpSessionRoute> {
        match &self.identity {
            CommandOwnerIdentity::Route(route) => Some(route),
            CommandOwnerIdentity::Session(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::BrowserContext;

    #[test]
    fn root_page_scope_freezes_the_concrete_active_target() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-scope".to_owned());
        browser_context.set_active_target_id("TID-original");
        conn.browser_context = Some(browser_context);

        let scope = CommandOwnerScope::capture(&conn, None);
        conn.browser_context
            .as_mut()
            .expect("browser context")
            .set_active_target_id("TID-replacement");

        assert_eq!(
            conn.target_owner_identity_for_route(scope.session_id(), scope.session_owner_route(),),
            Some(("BID-scope".to_owned(), Some("TID-original".to_owned())))
        );
    }

    #[test]
    fn root_scope_without_a_browser_context_has_explicit_browser_authority() {
        let conn = CdpConnection::default();

        let scope = CommandOwnerScope::capture(&conn, None);

        assert_eq!(scope.session_owner_route(), Some(&CdpSessionRoute::Browser));
    }

    #[test]
    fn root_scope_without_a_page_has_explicit_browser_context_authority() {
        let mut conn = CdpConnection::default();
        conn.browser_context = Some(BrowserContext::new("BID-empty".to_owned()));

        let scope = CommandOwnerScope::capture(&conn, None);

        assert_eq!(
            scope.session_owner_route(),
            Some(&CdpSessionRoute::BrowserContext {
                browser_context_id: "BID-empty".to_owned(),
            })
        );
    }
}
