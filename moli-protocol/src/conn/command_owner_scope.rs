use super::{
    CdpConnection, CdpSessionRoute, NoneSessionOwnerRouteOverrideScope,
    SessionOwnerRouteOverrideScope,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandOwnerScope {
    session_id: Option<String>,
    session_owner_route: Option<CdpSessionRoute>,
}

impl CommandOwnerScope {
    pub(crate) fn capture(conn: &CdpConnection, session_id: Option<&str>) -> Self {
        let session_owner_route = match session_id {
            Some(session_id) => conn.session_route(Some(session_id)),
            None => conn.none_session_owner_route_override(),
        };
        Self {
            session_id: session_id.map(str::to_owned),
            session_owner_route,
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

    pub(crate) fn enter<'a>(
        &self,
        conn: &'a mut CdpConnection,
    ) -> CommandOwnerRouteOverrideScope<'a> {
        match (self.session_id.as_ref(), self.session_owner_route.clone()) {
            (Some(session_id), Some(route)) => CommandOwnerRouteOverrideScope::Session(
                conn.scoped_session_owner_route_override(session_id.clone(), route),
            ),
            (Some(_), None) => CommandOwnerRouteOverrideScope::Direct(conn),
            (None, route) => CommandOwnerRouteOverrideScope::NoneSession(
                conn.scoped_optional_none_session_owner_route_override(route),
            ),
        }
    }
}

pub(crate) enum CommandOwnerRouteOverrideScope<'a> {
    Direct(&'a mut CdpConnection),
    Session(SessionOwnerRouteOverrideScope<'a>),
    NoneSession(NoneSessionOwnerRouteOverrideScope<'a>),
}

impl CommandOwnerRouteOverrideScope<'_> {
    pub(crate) fn conn_mut(&mut self) -> &mut CdpConnection {
        match self {
            Self::Direct(conn) => conn,
            Self::Session(scope) => scope.conn_mut(),
            Self::NoneSession(scope) => scope.conn_mut(),
        }
    }
}
