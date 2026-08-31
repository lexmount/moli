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
/// the frozen URL and exact target route in this move-only action makes that
/// boundary explicit.
#[derive(Debug)]
pub(crate) struct PopupTargetNavigationOwnerAction {
    owner_scope: CommandOwnerScope,
    browser_context_id: String,
    target_id: String,
    url: String,
    kind: PopupTargetNavigationKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PopupTargetNavigationKind {
    InitialDocument,
    InitialDocumentAfterDebuggerResume,
    NamedTargetReuse,
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
            // Activation has an independent owner action. Unlike a frozen
            // Stable PageTarget route keeps this exact target addressable if
            // the independent activation action changes its residence first.
            owner_scope: CommandOwnerScope::from_session_and_owner_route(
                None,
                Some(CdpSessionRoute::PageTarget {
                    browser_context_id: browser_context_id.to_owned(),
                    target_id: target_id.to_owned(),
                    is_attached_session: false,
                }),
            ),
            browser_context_id: browser_context_id.to_owned(),
            target_id: target_id.to_owned(),
            url,
            kind,
        })
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
    ) {
        (
            self.owner_scope,
            self.browser_context_id,
            self.target_id,
            self.url,
            self.kind,
        )
    }
}
