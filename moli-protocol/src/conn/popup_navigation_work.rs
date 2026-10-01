use super::{CdpConnection, CdpSessionRoute, CommandOwnerScope};

/// Navigation requested by an already-accepted auxiliary browsing-context
/// action.
///
/// Creating or resolving the target is part of the renderer output that
/// precedes the causing Runtime response. Loading the requested URL is not.
/// Blink's `LocalDOMWindow::open()` resolves the target, invokes `Navigate()`,
/// and returns the Window without waiting for the network load or Document
/// commit. Moli's navigation helper can itself become asynchronous, so
/// protocol projection must hand that work to the owner scheduler instead of
/// awaiting it while the opener's output cursor is being projected. Keeping
/// the frozen URL and exact target identity in this move-only action makes that
/// boundary explicit.
#[derive(Debug)]
pub(crate) struct PopupTargetNavigationOwnerAction {
    browser_context_id: String,
    target_id: String,
    url: String,
    kind: PopupTargetNavigationKind,
    document_response: Option<moli_core::page::RendererAuxiliaryDocumentResponse>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PopupTargetNavigationKind {
    InitialDocument,
    InitialDocumentAfterDebuggerResume,
    NamedTargetReuse { replace_current: bool },
    WindowReference(moli_core::page::RendererAuxiliaryNavigationKind),
}

impl PopupTargetNavigationOwnerAction {
    pub(crate) fn capture(
        conn: &CdpConnection,
        browser_context_id: &str,
        target_id: &str,
        url: String,
        kind: PopupTargetNavigationKind,
    ) -> Option<Self> {
        let route = conn.target_session_route_for_target_id(target_id)?;
        (route.browser_context_id() == Some(browser_context_id)).then(|| Self {
            browser_context_id: browser_context_id.to_owned(),
            target_id: target_id.to_owned(),
            url,
            kind,
            document_response: None,
        })
    }

    pub(crate) fn with_document_response(
        mut self,
        response: Option<moli_core::page::RendererAuxiliaryDocumentResponse>,
    ) -> Self {
        self.document_response = response;
        self
    }

    pub(crate) fn browser_context_id(&self) -> &str {
        &self.browser_context_id
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    pub(crate) fn kind(&self) -> PopupTargetNavigationKind {
        self.kind
    }

    pub(crate) fn replaces_pending_load(&self) -> bool {
        matches!(
            self.kind,
            PopupTargetNavigationKind::NamedTargetReuse { .. }
                | PopupTargetNavigationKind::WindowReference(_)
        )
    }

    pub(crate) fn is_command_followup(&self) -> bool {
        self.kind != PopupTargetNavigationKind::InitialDocumentAfterDebuggerResume
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        CommandOwnerScope,
        String,
        String,
        String,
        PopupTargetNavigationKind,
        Option<moli_core::page::RendererAuxiliaryDocumentResponse>,
    ) {
        (
            CommandOwnerScope::for_route(CdpSessionRoute::PageTarget {
                browser_context_id: self.browser_context_id.clone(),
                target_id: self.target_id.clone(),
                session_key: moli_page_types::DevToolsSessionKey::Primary,
            }),
            self.browser_context_id,
            self.target_id,
            self.url,
            self.kind,
            self.document_response,
        )
    }
}
