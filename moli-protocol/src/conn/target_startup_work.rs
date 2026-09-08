use super::{CdpConnection, CdpSessionRoute, CommandOwnerScope};
use moli_core::browser::{WebContentsHandle, web_contents::NavigationInterceptionPermit};

/// A debugger-barrier release runs after its command response and rechecks
/// late attachments. Native popups carry only an exact Browser decision;
/// explicit Target.createTarget still uses its existing initial-URL command.
#[derive(Debug)]
pub(crate) struct TargetStartupOwnerAction {
    browser_context_id: String,
    target_id: String,
    work: TargetStartupWork,
}

#[derive(Debug)]
enum TargetStartupWork {
    Decision {
        contents: WebContentsHandle,
        permit: NavigationInterceptionPermit,
    },
    InitialTargetNavigation {
        owner: CommandOwnerScope,
        url: String,
    },
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
        let work = if let Some((contents, paused)) =
            conn.native_navigation_decision_for_target(target_id)
        {
            TargetStartupWork::Decision {
                contents,
                permit: paused.permit,
            }
        } else {
            let context = conn.browser_context_by_id(browser_context_id)?;
            if !context.target_needs_initial_document_navigation(target_id) {
                return None;
            }
            TargetStartupWork::InitialTargetNavigation {
                owner: CommandOwnerScope::for_route(CdpSessionRoute::PageTarget {
                    browser_context_id: browser_context_id.to_owned(),
                    target_id: target_id.to_owned(),
                    session_key: moli_page_types::DevToolsSessionKey::Primary,
                }),
                url: context.devtools_target_info(target_id)?.url,
            }
        };
        Some(Self {
            browser_context_id: browser_context_id.to_owned(),
            target_id: target_id.to_owned(),
            work,
        })
    }

    pub(crate) fn browser_context_id(&self) -> &str {
        &self.browser_context_id
    }
    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }
    pub(crate) fn url(&self) -> Option<&str> {
        match &self.work {
            TargetStartupWork::InitialTargetNavigation { url, .. } => Some(url),
            TargetStartupWork::Decision { .. } => None,
        }
    }
    pub(crate) fn kind(&self) -> &'static str {
        match self.work {
            TargetStartupWork::Decision { .. } => "navigation-decision",
            TargetStartupWork::InitialTargetNavigation { .. } => "initial-target-navigation",
        }
    }
    #[cfg(test)]
    pub(crate) fn requires_background_navigation_scheduler(&self) -> bool {
        matches!(self.work, TargetStartupWork::InitialTargetNavigation { .. })
    }
    pub(crate) fn native_decision(
        &self,
    ) -> Option<(WebContentsHandle, NavigationInterceptionPermit)> {
        match self.work {
            TargetStartupWork::Decision { contents, permit } => Some((contents, permit)),
            TargetStartupWork::InitialTargetNavigation { .. } => None,
        }
    }
    pub(crate) fn into_initial_navigation(
        self,
    ) -> Option<(CommandOwnerScope, String, String, String)> {
        match self.work {
            TargetStartupWork::InitialTargetNavigation { owner, url } => {
                Some((owner, self.browser_context_id, self.target_id, url))
            }
            TargetStartupWork::Decision { .. } => None,
        }
    }
}
