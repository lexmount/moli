use super::CdpConnection;
use moli_core::browser::{WebContentsHandle, web_contents::NavigationInterceptionPermit};

/// A debugger-barrier release runs after its command response and rechecks
/// late attachments. It carries only the original Browser decision, never a
/// URL or permission to start a replacement navigation.
#[derive(Debug)]
pub(crate) struct TargetStartupOwnerAction {
    target_id: String,
    contents: WebContentsHandle,
    permit: NavigationInterceptionPermit,
}

impl TargetStartupOwnerAction {
    pub(crate) fn capture(
        conn: &CdpConnection,
        browser_context_id: &str,
        target_id: &str,
    ) -> Option<Self> {
        let route = conn.target_session_route_for_target_id(target_id)?;
        if route.browser_context_id() != Some(browser_context_id) {
            return None;
        }
        let (contents, paused) = conn.native_navigation_decision_for_target(target_id)?;
        Some(Self {
            target_id: target_id.to_owned(),
            contents,
            permit: paused.permit,
        })
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }
    pub(crate) fn decision(&self) -> (WebContentsHandle, NavigationInterceptionPermit) {
        (self.contents, self.permit)
    }
}
