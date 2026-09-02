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

    fn capture_default_route(conn: &CdpConnection) -> CdpSessionRoute {
        let Some(browser_context) = conn.browser_context.as_ref() else {
            return CdpSessionRoute::Browser;
        };
        match browser_context.active_target_id_owned() {
            Some(target_id) => CdpSessionRoute::PageTarget {
                browser_context_id: browser_context.id.clone(),
                target_id,
                session_key: moli_page_types::DevToolsSessionKey::Primary,
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
                session_key: moli_page_types::DevToolsSessionKey::Primary,
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

    /// Resolves this command's single authority to its current CDP route.
    ///
    /// Session owners resolve through the attachment registry. Route owners
    /// retain the exact route captured at command admission. Callers therefore
    /// cannot express the old invalid combinations of an absent authority or
    /// both a session and an explicit route.
    pub(crate) fn resolve_route(&self, conn: &CdpConnection) -> Option<CdpSessionRoute> {
        match &self.identity {
            CommandOwnerIdentity::Session(session_id) => conn.session_route(Some(session_id)),
            CommandOwnerIdentity::Route(route) => Some(route.clone()),
        }
    }

    pub(crate) fn explicit_route(&self) -> Option<&CdpSessionRoute> {
        match &self.identity {
            CommandOwnerIdentity::Session(_) => None,
            CommandOwnerIdentity::Route(route) => Some(route),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{BrowserContext, PageTargetHost};
    use crate::devtools_runtime::{
        DevToolsCommandContext, DevToolsProtocol, DevToolsSessionId, DevToolsTargetId,
    };

    fn page_context(session_id: &str, target_id: &str) -> DevToolsCommandContext {
        DevToolsCommandContext {
            protocol: DevToolsProtocol::Cdp,
            session_id: Some(DevToolsSessionId::from(session_id)),
            target_id: Some(DevToolsTargetId::from(target_id)),
            browser_context_id: None,
        }
    }

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
            conn.target_owner_identity_for_owner(&scope,),
            Some(("BID-scope".to_owned(), Some("TID-original".to_owned())))
        );
    }

    #[test]
    fn root_scope_without_a_browser_context_has_explicit_browser_authority() {
        let conn = CdpConnection::default();

        let scope = CommandOwnerScope::capture(&conn, None);

        assert_eq!(scope.explicit_route(), Some(&CdpSessionRoute::Browser));
    }

    #[test]
    fn root_scope_without_a_page_has_explicit_browser_context_authority() {
        let mut conn = CdpConnection::default();
        conn.browser_context = Some(BrowserContext::new("BID-empty".to_owned()));

        let scope = CommandOwnerScope::capture(&conn, None);

        assert_eq!(
            scope.explicit_route(),
            Some(&CdpSessionRoute::BrowserContext {
                browser_context_id: "BID-empty".to_owned(),
            })
        );
    }

    #[test]
    fn explicit_target_preserves_matching_attached_session_authority() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new_with_page_for_test("BID-owner", "TID-owner");
        browser_context.attach_active_session("SID-primary");
        assert!(
            browser_context
                .assign_attached_session_to_target("TID-owner", "SID-attached".to_owned(),)
        );
        conn.browser_context = Some(browser_context);

        let owner = conn
            .command_owner_scope_for_devtools_context(&page_context("SID-attached", "TID-owner"))
            .expect("matching attached attachment should own the command");

        assert_eq!(owner.session_id(), Some("SID-attached"));
        assert!(owner.explicit_route().is_none());
    }

    #[test]
    fn explicit_target_routes_protocol_neutral_session_and_rejects_wrong_cdp_attachment() {
        let mut conn = CdpConnection::default();
        let mut browser_context =
            BrowserContext::new_with_page_for_test("BID-owner", "TID-primary");
        browser_context.attach_active_session("SID-primary");
        assert!(
            browser_context.insert_page_target_host(PageTargetHost::with_url(
                "TID-background".to_owned(),
                Some("SID-background".to_owned()),
                "about:blank".to_owned(),
            ))
        );
        conn.browser_context = Some(browser_context);

        let protocol_neutral = conn
            .command_owner_scope_for_devtools_context(&page_context(
                "webdriver-session",
                "TID-background",
            ))
            .expect("non-CDP session should use its explicit target");
        assert_eq!(protocol_neutral.session_id(), None);
        assert_eq!(
            protocol_neutral.explicit_route(),
            Some(&CdpSessionRoute::PageTarget {
                browser_context_id: "BID-owner".to_owned(),
                target_id: "TID-background".to_owned(),
                session_key: moli_page_types::DevToolsSessionKey::Primary,
            })
        );

        assert!(
            conn.command_owner_scope_for_devtools_context(&page_context(
                "SID-primary",
                "TID-background",
            ))
            .is_none(),
            "a real CDP attachment cannot address another target",
        );
    }
}
