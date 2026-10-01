use super::{CdpConnection, CdpSessionRoute, CommandOwnerScope};

/// Foreground selection for one already-created popup Page target.
///
/// Target creation and navigation have separate lifetimes. In particular, a
/// target waiting for `Runtime.runIfWaitingForDebugger` must still become the
/// selected target immediately. This move-only action freezes the exact
/// target identity while allowing that target to move between background and
/// active residence before the action completes.
#[derive(Debug)]
pub(crate) struct PopupTargetActivationAction {
    browser_context_id: String,
    target_id: String,
}

impl PopupTargetActivationAction {
    pub(crate) fn capture(
        conn: &CdpConnection,
        browser_context_id: &str,
        target_id: &str,
    ) -> Option<Self> {
        let route = conn.target_session_route_for_target_id(target_id)?;
        (route.browser_context_id() == Some(browser_context_id)).then(|| Self {
            browser_context_id: browser_context_id.to_owned(),
            target_id: target_id.to_owned(),
        })
    }

    pub(crate) fn browser_context_id(&self) -> &str {
        &self.browser_context_id
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    pub(crate) fn into_parts(self) -> (CommandOwnerScope, String, String) {
        (
            CommandOwnerScope::for_route(CdpSessionRoute::PageTarget {
                browser_context_id: self.browser_context_id.clone(),
                target_id: self.target_id.clone(),
                session_key: moli_page_types::DevToolsSessionKey::Primary,
            }),
            self.browser_context_id,
            self.target_id,
        )
    }
}
