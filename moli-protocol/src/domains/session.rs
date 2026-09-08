use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, DevToolsSessionHandlerSet, SessionDisposalPlan,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DevToolsSessionDomainHandler {
    Tracing,
    Browser,
    Target,
    Fetch,
    Runtime,
    Emulation,
    Network,
    PrimaryPageTargetState,
}

const CONNECTION_HANDLERS: &[DevToolsSessionDomainHandler] = &[
    DevToolsSessionDomainHandler::Tracing,
    DevToolsSessionDomainHandler::Browser,
    DevToolsSessionDomainHandler::Target,
];

const PAGE_HANDLERS: &[DevToolsSessionDomainHandler] = &[
    DevToolsSessionDomainHandler::Tracing,
    DevToolsSessionDomainHandler::Browser,
    DevToolsSessionDomainHandler::Target,
    DevToolsSessionDomainHandler::Fetch,
    DevToolsSessionDomainHandler::Runtime,
    DevToolsSessionDomainHandler::Emulation,
    DevToolsSessionDomainHandler::Network,
    DevToolsSessionDomainHandler::PrimaryPageTargetState,
];

const WORKER_HANDLERS: &[DevToolsSessionDomainHandler] = &[
    DevToolsSessionDomainHandler::Tracing,
    DevToolsSessionDomainHandler::Browser,
    DevToolsSessionDomainHandler::Target,
    DevToolsSessionDomainHandler::Runtime,
];

/// Browser-side handlers installed for one DevTools session.
///
/// Target teardown iterates this collection without knowing any domain's
/// state or disable operation. Page Inspector resources have already retired
/// through the renderer session lifecycle; these handlers revoke service
/// contributions and reconcile Browser-owned policy without that Inspector.
impl DevToolsSessionHandlerSet {
    fn handlers(self) -> &'static [DevToolsSessionDomainHandler] {
        match self {
            Self::Connection => CONNECTION_HANDLERS,
            Self::Page => PAGE_HANDLERS,
            Self::Worker => WORKER_HANDLERS,
        }
    }
}

pub(crate) struct DevToolsSessionHandlerDisposal {
    first_error: Option<anyhow::Error>,
    renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
}

impl DevToolsSessionHandlerDisposal {
    pub(crate) fn first_error(&self) -> Option<&anyhow::Error> {
        self.first_error.as_ref()
    }

    pub(crate) fn into_renderer_output_predecessor(self) -> Option<moli_core::RendererOutputFence> {
        self.renderer_output_predecessor
    }
}

/// Invokes every installed browser-side handler while the service route is
/// still retained for cleanup. One failure never prevents the remaining handlers
/// from receiving their disposal callback.
pub(crate) async fn dispose_live_handlers_async(
    conn: &mut CdpConnection,
    background_events: &mut Vec<BackgroundProtocolEvent>,
    protocol_events: &mut Vec<BackgroundProtocolEvent>,
    plan: &SessionDisposalPlan,
    renderer_policy_reconciled: bool,
) -> DevToolsSessionHandlerDisposal {
    let mut disposal = DevToolsSessionHandlerDisposal {
        first_error: None,
        renderer_output_predecessor: None,
    };
    for handler in plan.handler_set().handlers() {
        let result = handler
            .dispose_async(
                conn,
                background_events,
                protocol_events,
                plan,
                renderer_policy_reconciled,
            )
            .await;
        match result {
            Ok(Some(predecessor)) => {
                disposal.renderer_output_predecessor = Some(predecessor);
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(
                    session_id = plan.session_id(),
                    domain = handler.name(),
                    %error,
                    "failed to dispose DevTools session domain"
                );
                disposal.first_error.get_or_insert_with(|| {
                    anyhow::anyhow!("failed to dispose {} domain: {error:#}", handler.name())
                });
            }
        }
    }
    disposal
}

/// Runs the handlers whose state survives target destruction. Renderer-owned
/// and target-owned handlers have already disappeared with the closed target.
pub(crate) async fn dispose_closed_handlers_async(conn: &mut CdpConnection, session_id: &str) {
    super::tracing::dispose_session_handler_async(conn, session_id).await;
    super::browser::dispose_session_handler(conn, session_id);
    super::target::dispose_session_handler(conn, session_id);
}

pub(crate) fn dispose_closed_handlers_sync(conn: &mut CdpConnection, session_id: &str) {
    conn.cancel_tracing_for_session_owner(Some(session_id));
    super::browser::dispose_session_handler(conn, session_id);
    super::target::dispose_session_handler(conn, session_id);
}

impl DevToolsSessionDomainHandler {
    fn name(self) -> &'static str {
        match self {
            Self::Tracing => "Tracing",
            Self::Browser => "Browser",
            Self::Target => "Target",
            Self::Fetch => "Fetch",
            Self::Runtime => "Runtime",
            Self::Emulation => "Emulation",
            Self::Network => "Network",
            Self::PrimaryPageTargetState => "primary Page target",
        }
    }

    async fn dispose_async(
        self,
        conn: &mut CdpConnection,
        background_events: &mut Vec<BackgroundProtocolEvent>,
        protocol_events: &mut Vec<BackgroundProtocolEvent>,
        plan: &SessionDisposalPlan,
        renderer_policy_reconciled: bool,
    ) -> anyhow::Result<Option<moli_core::RendererOutputFence>> {
        let session_id = plan.session_id();
        match self {
            Self::Tracing => {
                super::tracing::dispose_session_handler_async(conn, session_id).await;
                Ok(None)
            }
            Self::Browser => {
                super::browser::dispose_session_handler(conn, session_id);
                Ok(None)
            }
            Self::Target => {
                super::target::dispose_session_handler(conn, session_id);
                Ok(None)
            }
            Self::Fetch => {
                Box::pin(super::fetch::dispose_session_async(
                    conn,
                    background_events,
                    session_id,
                    renderer_policy_reconciled,
                ))
                .await
            }
            Self::Runtime => {
                super::runtime::dispose_session_handler_async(
                    conn,
                    background_events,
                    protocol_events,
                    plan,
                )
                .await?;
                Ok(None)
            }
            Self::Emulation => {
                super::emulation::dispose_page_session_async(conn, session_id).await?;
                Ok(None)
            }
            Self::Network => {
                super::network::dispose_session_policy_async(conn, session_id).await?;
                Ok(None)
            }
            Self::PrimaryPageTargetState => {
                super::page::dispose_primary_session_target_state_async(conn, plan).await?;
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handler_sets_are_owned_by_session_target_kind() {
        assert_eq!(DevToolsSessionHandlerSet::Page.handlers(), PAGE_HANDLERS);
        assert_eq!(
            DevToolsSessionHandlerSet::Worker.handlers(),
            WORKER_HANDLERS
        );
        assert_eq!(
            DevToolsSessionHandlerSet::Connection.handlers(),
            CONNECTION_HANDLERS
        );
    }
}
