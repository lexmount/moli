use anyhow::{Context as _, Result};

use super::*;

/// Authority for one internal Runtime operation on an existing Page. Literal
/// session/target ids and host-local attachment counters can be reused; retain
/// the original binding lifetime and weak host identity as well as the Page.
#[derive(Clone)]
pub(super) struct RuntimePageScope {
    owner: CommandOwnerScope,
    route: RuntimeProtocolMessagePageRoute,
    page: TargetPageProtocolAttachmentIdentity,
    renderer_page: RendererPageResidenceIdentity,
    host: crate::conn::PageTargetHostIdentity,
    binding: Option<crate::conn::SessionDisposalWait>,
}

impl RuntimePageScope {
    pub(super) fn capture(conn: &CdpConnection, owner: &CommandOwnerScope) -> Result<Self> {
        let page = conn
            .target_page_protocol_attachment_identity_for_owner(owner)
            .context(NoDocumentLoaded)?;
        let session = owner
            .session_id()
            .map(str::to_owned)
            .or_else(|| conn.target_renderer_runtime_inspector_session_id_for_owner(owner));
        let binding = session
            .as_deref()
            .and_then(|id| conn.session_binding_lifetime(id));
        let owner = match owner.resolve_route(conn).context(NoDocumentLoaded)? {
            route @ CdpSessionRoute::PageTarget { .. } => CommandOwnerScope::for_route(route),
            _ => CommandOwnerScope::for_page_residence(page.page_owner()),
        };
        Ok(Self {
            route: conn
                .runtime_protocol_message_page_route_for_owner(&owner)
                .context(NoDocumentLoaded)?,
            renderer_page: conn
                .renderer_page_residence_identity_for_owner(&owner)
                .context(NoDocumentLoaded)?,
            host: conn
                .page_target_host_identity_for_owner(&owner)
                .context(NoDocumentLoaded)?,
            owner,
            page,
            binding,
        })
    }

    pub(super) fn owner(&self) -> &CommandOwnerScope {
        &self.owner
    }

    pub(super) fn target_id(&self) -> Option<&str> {
        self.route.target_id.as_deref()
    }

    pub(super) fn binding_is_current(&self) -> bool {
        self.binding
            .as_ref()
            .is_none_or(|binding| !binding.is_closed())
    }

    /// Journal cancellation may retire the original call after its Page has
    /// been replaced, but must never mutate a successor host or binding.
    pub(super) fn owns_current_host(&self, conn: &CdpConnection) -> bool {
        self.binding_is_current()
            && conn
                .page_target_host_identity_for_owner(&self.owner)
                .as_ref()
                .is_some_and(|host| self.host.matches(host))
    }

    pub(super) fn is_current(&self, conn: &CdpConnection) -> bool {
        self.owns_current_host(conn)
            && conn.target_page_protocol_attachment_identity_is_current(&self.page)
            && conn.renderer_page_residence_identity_for_owner(&self.owner)
                == Some(self.renderer_page)
            && conn.current_renderer_agent_attachment_id_for_owner(&self.owner)
                == Some(self.route.renderer_agent_attachment_id)
    }

    pub(super) fn page_mut<'a>(&self, conn: &'a mut CdpConnection) -> Result<&'a mut Page> {
        anyhow::ensure!(self.is_current(conn), NoDocumentLoaded);
        conn.runtime_protocol_message_started_page_mut(&self.route)
    }
}
