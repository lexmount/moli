use anyhow::{Context as _, Result};

use super::*;

/// Authority for one internal Runtime operation on an existing Page. Literal
/// session/target ids and host-local attachment counters can be reused; retain
/// the original binding lifetime and weak host identity as well as the Page.
#[derive(Clone, Debug)]
pub(crate) struct RuntimePageScope {
    owner: CommandOwnerScope,
    route: RuntimeProtocolMessagePageRoute,
    page: TargetPageProtocolAttachmentIdentity,
    renderer_page: RendererPageResidenceIdentity,
}

impl RuntimePageScope {
    pub(crate) fn capture(conn: &CdpConnection, owner: &CommandOwnerScope) -> Result<Self> {
        let page = conn
            .target_page_protocol_attachment_identity_for_owner(owner)
            .context(NoDocumentLoaded)?;
        let route = conn
            .runtime_protocol_message_page_route_for_owner(owner)
            .context(NoDocumentLoaded)?;
        let owner = route.host.owner.clone();
        Ok(Self {
            route,
            renderer_page: conn
                .renderer_page_residence_identity_for_owner(&owner)
                .context(NoDocumentLoaded)?,
            owner,
            page,
        })
    }

    pub(crate) fn owner(&self) -> &CommandOwnerScope {
        &self.owner
    }

    pub(crate) fn target_id(&self) -> Option<&str> {
        self.route.target_id.as_deref()
    }

    pub(crate) fn host_scope(&self) -> RuntimePageHostScope {
        self.route.host.clone()
    }

    #[cfg(test)]
    pub(crate) fn binding_is_current(&self) -> bool {
        self.route
            .host
            .binding
            .as_ref()
            .is_none_or(|binding| !binding.is_closed())
    }

    /// Cancellation may retire an original call after its Document is replaced,
    /// but must never follow a reused binding or target host.
    pub(crate) fn owns_current_host(&self, conn: &CdpConnection) -> bool {
        self.route.host.is_current(conn)
    }

    pub(crate) fn is_current(&self, conn: &CdpConnection) -> bool {
        self.owns_current_host(conn)
            && conn.target_page_protocol_attachment_identity_is_current(&self.page)
            && conn.renderer_page_residence_identity_for_owner(&self.owner)
                == Some(self.renderer_page)
            && conn.current_renderer_agent_attachment_id_for_owner(&self.owner)
                == Some(self.route.renderer_agent_attachment_id)
    }

    pub(crate) fn can_start_operation(&self, conn: &CdpConnection) -> bool {
        self.is_current(conn) && self.route.host.can_start_operation(conn)
    }

    pub(crate) fn page_mut<'a>(&self, conn: &'a mut CdpConnection) -> Result<&'a mut Page> {
        anyhow::ensure!(self.is_current(conn), NoDocumentLoaded);
        conn.runtime_protocol_message_started_page_mut(&self.route)
    }
}

/// Original target host and binding, independent of Document replacements.
/// Public script results and cancellation can outlive their starting Document.
#[derive(Clone, Debug)]
pub(crate) struct RuntimePageHostScope {
    owner: CommandOwnerScope,
    host: crate::conn::PageTargetHostIdentity,
    binding: Option<crate::conn::SessionDisposalWait>,
    session_id: Option<String>,
}

impl RuntimePageHostScope {
    pub(crate) fn capture(conn: &CdpConnection, owner: &CommandOwnerScope) -> Result<Self> {
        let session = owner
            .session_id()
            .map(str::to_owned)
            .or_else(|| conn.target_renderer_runtime_inspector_session_id_for_owner(owner));
        let binding = session
            .as_deref()
            .and_then(|id| conn.session_binding_lifetime(id));
        let host = conn
            .page_target_host_identity_for_owner(owner)
            .context(NoDocumentLoaded)?;
        let owner = match owner.resolve_route(conn).context(NoDocumentLoaded)? {
            route @ CdpSessionRoute::PageTarget { .. } => CommandOwnerScope::for_route(route),
            _ => CommandOwnerScope::for_page_residence(
                &conn
                    .target_page_residence_identity_for_owner(owner)
                    .context(NoDocumentLoaded)?,
            ),
        };
        Ok(Self {
            owner,
            host,
            binding,
            session_id: session,
        })
    }

    pub(crate) fn is_current(&self, conn: &CdpConnection) -> bool {
        self.binding
            .as_ref()
            .is_none_or(|binding| !binding.is_closed())
            && conn
                .page_target_host_identity_for_owner(&self.owner)
                .as_ref()
                .is_some_and(|host| self.host.matches(host))
    }

    fn can_start_operation(&self, conn: &CdpConnection) -> bool {
        if !self.is_current(conn) || conn.target_is_closing_for_owner(&self.owner) {
            return false;
        }
        self.session_id
            .as_deref()
            .is_none_or(|session| !conn.session_is_closing(session))
    }
}
