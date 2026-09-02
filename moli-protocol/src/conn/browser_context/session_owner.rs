use crate::conn::{CdpConnection, CdpSessionRoute, CommandOwnerScope, PageTargetHost};
use moli_page_types::DevToolsSessionKey;

pub(super) struct TargetSessionOwner {
    pub(super) browser_context_id: String,
    pub(super) target_id: String,
    pub(super) session_key: DevToolsSessionKey,
}

impl CdpConnection {
    /// Some generated Page-domain agents acknowledge enable/disable commands
    /// before a Page host exists. Keep that protocol behavior at the command
    /// boundary instead of manufacturing an owner that does not own a Page.
    pub(super) fn accepts_unmaterialized_page_command(&self, owner: &CommandOwnerScope) -> bool {
        if owner.session_id().is_some() {
            return false;
        }
        match owner.explicit_route() {
            Some(CdpSessionRoute::Browser | CdpSessionRoute::BrowserContext { .. }) => true,
            Some(_) => false,
            None => false,
        }
    }

    pub(super) fn accepts_unmaterialized_page_command_for_session(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.accepts_unmaterialized_page_command(&owner)
    }

    pub(crate) fn mutate_target_page_state_for_session(
        &mut self,
        session_id: Option<&str>,
        f: impl FnOnce(&mut PageTargetHost),
    ) -> bool {
        self.target_session_owner_mut(session_id)
            .map(|mut owner| owner.mutate_page_state(|state, _| f(state)))
            .is_some()
    }

    pub(super) fn target_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Option<TargetSessionOwner> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.target_session_owner_for_owner(&owner)
    }

    /// Resolves target ownership from the command's single authority.
    pub(super) fn target_session_owner_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Option<TargetSessionOwner> {
        let route = owner.resolve_route(self)?;

        match route {
            CdpSessionRoute::Browser => {
                let browser_context = self.browser_context.as_ref()?;
                Some(TargetSessionOwner {
                    browser_context_id: browser_context.id.clone(),
                    target_id: browser_context.active_target_id()?.to_owned(),
                    session_key: DevToolsSessionKey::Primary,
                })
            }
            CdpSessionRoute::BrowserContext { browser_context_id } => {
                let browser_context = self.browser_context_by_id(&browser_context_id)?;
                Some(TargetSessionOwner {
                    browser_context_id,
                    target_id: browser_context.active_target_id()?.to_owned(),
                    session_key: DevToolsSessionKey::Primary,
                })
            }
            CdpSessionRoute::PageTarget {
                browser_context_id,
                target_id,
                session_key,
            } => {
                let browser_context = self.browser_context_by_id(&browser_context_id)?;
                if browser_context.page_target(&target_id).is_some() {
                    Some(TargetSessionOwner {
                        browser_context_id,
                        target_id,
                        session_key,
                    })
                } else {
                    None
                }
            }
            CdpSessionRoute::TabTarget { .. }
            | CdpSessionRoute::SharedWorkerTarget { .. }
            | CdpSessionRoute::DedicatedWorkerTarget { .. }
            | CdpSessionRoute::ServiceWorkerTarget { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::BrowserContext;

    #[test]
    fn empty_browser_context_does_not_materialize_a_page_owner() {
        let mut conn = CdpConnection::default();
        conn.browser_context = Some(BrowserContext::new("BID-empty-owner".to_owned()));

        assert!(conn.target_session_owner(None).is_none());
        assert!(conn.accepts_unmaterialized_page_command(&CommandOwnerScope::capture(&conn, None)));
        assert!(
            !conn.accepts_unmaterialized_page_command(&CommandOwnerScope::for_session(
                "SID-missing"
            ))
        );
    }

    #[test]
    fn stale_page_route_is_not_reinterpreted_as_the_active_page() {
        let mut conn = CdpConnection::default();
        conn.browser_context = Some(BrowserContext::new_with_page_for_test(
            "BID-stale-owner",
            "TID-live",
        ));
        let stale_route = CdpSessionRoute::PageTarget {
            browser_context_id: "BID-stale-owner".to_owned(),
            target_id: "TID-stale".to_owned(),
            session_key: DevToolsSessionKey::Primary,
        };

        let owner = CommandOwnerScope::for_route(stale_route);
        assert!(conn.target_session_owner_for_owner(&owner).is_none());
        assert!(!conn.accepts_unmaterialized_page_command(&owner));
    }
}
