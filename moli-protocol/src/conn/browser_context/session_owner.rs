#[cfg(test)]
use crate::conn::BrowserContext;
use crate::conn::{CdpConnection, CdpSessionRoute, PageTargetHost};
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
    pub(super) fn accepts_unmaterialized_page_command(
        &self,
        session_id: Option<&str>,
        owner_route: Option<&CdpSessionRoute>,
    ) -> bool {
        if session_id.is_some() {
            return false;
        }
        match owner_route {
            Some(CdpSessionRoute::Browser | CdpSessionRoute::BrowserContext { .. }) => true,
            Some(_) => false,
            None => self
                .browser_context
                .as_ref()
                .and_then(|browser_context| browser_context.active_target_id())
                .is_none(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_background_target_session<R>(
        &mut self,
        session_id: Option<&str>,
        f: impl FnOnce(&mut BrowserContext, &str) -> R,
    ) -> Option<R> {
        let (browser_context_id, target_id) = self.background_target_route(session_id)?;
        let browser_context = self.browser_context_by_id_mut(&browser_context_id)?;
        Some(f(browser_context, &target_id))
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
        self.target_session_owner_for_route(session_id, None)
    }

    /// Resolves target ownership from an explicit command route. A concrete
    /// session always resolves through the session registry; `owner_route`
    /// selects the owner of an internal command without a CDP session.
    pub(super) fn target_session_owner_for_route(
        &self,
        session_id: Option<&str>,
        owner_route: Option<&CdpSessionRoute>,
    ) -> Option<TargetSessionOwner> {
        let route = match session_id {
            Some(session_id) => self.session_route(Some(session_id))?,
            None => match owner_route {
                Some(route) => route.clone(),
                None => {
                    let browser_context = self.browser_context.as_ref()?;
                    let target_id = browser_context.active_target_id()?;
                    return Some(TargetSessionOwner {
                        browser_context_id: browser_context.id.clone(),
                        target_id: target_id.to_owned(),
                        session_key: DevToolsSessionKey::Primary,
                    });
                }
            },
        };

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

    #[test]
    fn empty_browser_context_does_not_materialize_a_page_owner() {
        let mut conn = CdpConnection::default();
        conn.browser_context = Some(BrowserContext::new("BID-empty-owner".to_owned()));

        assert!(conn.target_session_owner(None).is_none());
        assert!(conn.accepts_unmaterialized_page_command(None, None));
        assert!(!conn.accepts_unmaterialized_page_command(Some("SID-missing"), None));
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

        assert!(
            conn.target_session_owner_for_route(None, Some(&stale_route))
                .is_none()
        );
        assert!(!conn.accepts_unmaterialized_page_command(None, Some(&stale_route)));
    }
}
