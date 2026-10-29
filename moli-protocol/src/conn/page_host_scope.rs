use super::*;
use anyhow::{Context as _, Result};

/// Original target host and binding, independent of Document replacements.
/// Public script results and cancellation can outlive their starting Document.
#[derive(Clone, Debug)]
pub(crate) struct ProtocolPageHostScope {
    owner: CommandOwnerScope,
    host: crate::conn::PageTargetHostIdentity,
    binding: Option<crate::conn::SessionDisposalWait>,
    session_id: Option<String>,
}

impl ProtocolPageHostScope {
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
            _ => {
                let (browser_context_id, target_id) = conn
                    .target_owner_identity_for_owner(owner)
                    .context(NoDocumentLoaded)?;
                let target_id = target_id
                    .or_else(|| {
                        conn.browser_context_by_id(&browser_context_id)
                            .and_then(|context| context.active_target_id().map(str::to_owned))
                    })
                    .context(NoDocumentLoaded)?;
                CommandOwnerScope::for_route(CdpSessionRoute::PageTarget {
                    browser_context_id,
                    target_id,
                    session_key: moli_page_types::DevToolsSessionKey::Primary,
                })
            }
        };
        Ok(Self {
            owner,
            host,
            binding,
            session_id: session,
        })
    }

    pub(crate) fn capture_for_operation(
        conn: &CdpConnection,
        owner: &CommandOwnerScope,
    ) -> Result<Self> {
        let scope = Self::capture(conn, owner)?;
        anyhow::ensure!(scope.can_start_operation(conn), NoDocumentLoaded);
        Ok(scope)
    }

    pub(crate) fn owner(&self) -> &CommandOwnerScope {
        &self.owner
    }

    #[cfg(test)]
    pub(crate) fn binding_is_current(&self) -> bool {
        self.binding
            .as_ref()
            .is_none_or(|binding| !binding.is_closed())
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

    pub(crate) fn can_start_operation(&self, conn: &CdpConnection) -> bool {
        if !self.is_current(conn) || conn.target_is_closing_for_owner(&self.owner) {
            return false;
        }
        self.session_id
            .as_deref()
            .is_none_or(|session| !conn.session_is_closing(session))
    }
}
