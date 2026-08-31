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
            .then(|| conn.none_session_owner_route_override())
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
    /// route is only needed for protocol-neutral and deferred work which uses
    /// Chromium's implicit primary Page attachment.
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
