use super::{CdpConnection, CdpSessionRoute, NoneSessionOwnerRouteOverrideScope};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandOwnerScope {
    identity: CommandOwnerIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CommandOwnerIdentity {
    Session(String),
    ImplicitRoute(CdpSessionRoute),
    Implicit,
}

impl CommandOwnerScope {
    pub(crate) fn capture(conn: &CdpConnection, session_id: Option<&str>) -> Self {
        let none_session_owner_route = session_id
            .is_none()
            .then(|| {
                conn.none_session_owner_route_override().or_else(|| {
                    let browser_context = conn.browser_context.as_ref()?;
                    let target_id = browser_context.active_target_id_owned()?;
                    Some(CdpSessionRoute::PageTarget {
                        browser_context_id: browser_context.id.clone(),
                        target_id,
                        is_attached_session: false,
                    })
                })
            })
            .flatten();
        match session_id {
            Some(session_id) => Self::for_session(session_id),
            None => Self::for_implicit_route(none_session_owner_route),
        }
    }

    pub(crate) fn for_session(session_id: &str) -> Self {
        Self {
            identity: CommandOwnerIdentity::Session(session_id.to_owned()),
        }
    }

    pub(crate) fn for_implicit_route(session_owner_route: Option<CdpSessionRoute>) -> Self {
        Self {
            identity: match session_owner_route {
                Some(route) => CommandOwnerIdentity::ImplicitRoute(route),
                None => CommandOwnerIdentity::Implicit,
            },
        }
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        match &self.identity {
            CommandOwnerIdentity::Session(session_id) => Some(session_id),
            CommandOwnerIdentity::ImplicitRoute(_) | CommandOwnerIdentity::Implicit => None,
        }
    }

    /// Returns the exact route captured for an implicit-session command.
    ///
    /// A concrete CDP session remains authoritative through `session_id`; the
    /// route freezes Chromium's implicit primary Page attachment at command
    /// admission, so deferred completion cannot follow a later foreground
    /// selection.
    pub(crate) fn session_owner_route(&self) -> Option<&CdpSessionRoute> {
        match &self.identity {
            CommandOwnerIdentity::ImplicitRoute(route) => Some(route),
            CommandOwnerIdentity::Session(_) | CommandOwnerIdentity::Implicit => None,
        }
    }

    pub(crate) fn enter<'a>(
        &self,
        conn: &'a mut CdpConnection,
    ) -> NoneSessionOwnerRouteOverrideScope<'a> {
        conn.scoped_optional_none_session_owner_route_override(self.session_owner_route().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::BrowserContext;

    #[test]
    fn implicit_scope_freezes_the_concrete_active_target() {
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
}
