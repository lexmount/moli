use super::{CdpConnection, CdpSessionRoute, NoneSessionOwnerRouteOverrideScope};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandOwnerScope {
    session_id: Option<String>,
    session_owner_route: Option<CdpSessionRoute>,
}

impl CommandOwnerScope {
    pub(crate) fn capture(conn: &CdpConnection, session_id: Option<&str>) -> Self {
        let none_session_owner_route = session_id
            .is_none()
            .then(|| {
                conn.none_session_owner_route_override().or_else(|| {
                    let browser_context = conn.browser_context.as_ref()?;
                    let target_id = browser_context.active_target_id_owned()?;
                    Some(CdpSessionRoute::ActiveTarget {
                        browser_context_id: browser_context.id.clone(),
                        target_id: Some(target_id),
                    })
                })
            })
            .flatten();
        Self {
            session_id: session_id.map(str::to_owned),
            session_owner_route: none_session_owner_route,
        }
    }

    pub(crate) fn from_session_and_owner_route(
        session_id: Option<&str>,
        session_owner_route: Option<CdpSessionRoute>,
    ) -> Self {
        Self {
            session_id: session_id.map(str::to_owned),
            session_owner_route,
        }
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Returns the exact route captured for an implicit-session command.
    ///
    /// A concrete CDP session remains authoritative through `session_id`; the
    /// route freezes Chromium's implicit primary Page attachment at command
    /// admission, so deferred completion cannot follow a later foreground
    /// selection.
    pub(crate) fn session_owner_route(&self) -> Option<&CdpSessionRoute> {
        self.session_owner_route.as_ref()
    }

    pub(crate) fn enter<'a>(
        &self,
        conn: &'a mut CdpConnection,
    ) -> NoneSessionOwnerRouteOverrideScope<'a> {
        conn.scoped_optional_none_session_owner_route_override(self.session_owner_route.clone())
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
