#[cfg(test)]
use crate::conn::BrowserContext;
use crate::conn::{CdpConnection, CdpSessionRoute, PageTargetHost};

pub(super) enum TargetSessionOwner {
    PageTarget {
        browser_context_id: String,
        target_id: String,
        is_auxiliary_target_session: bool,
    },
    NoLoadedBrowserContext,
}

impl CdpConnection {
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
            .and_then(|mut owner| owner.mutate_page_state(|state, _, _| f(state)))
            .is_some()
    }

    pub(super) fn target_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Option<TargetSessionOwner> {
        let none_session_owner_route = self.none_session_owner_route_override();
        self.target_session_owner_for_route(session_id, none_session_owner_route.as_ref())
    }

    /// Resolves target ownership from an explicit command route.
    ///
    /// This is the non-ambient owner boundary used while migrating deferred
    /// commands away from `none_session_owner_route_override`. A concrete
    /// session always resolves through the session registry; `owner_route`
    /// only selects the owner of an implicit primary Page attachment.
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
                    let Some(browser_context) = self.browser_context.as_ref() else {
                        return Some(TargetSessionOwner::NoLoadedBrowserContext);
                    };
                    let Some(target_id) = browser_context.active_target_id() else {
                        return Some(TargetSessionOwner::NoLoadedBrowserContext);
                    };
                    return Some(TargetSessionOwner::PageTarget {
                        browser_context_id: browser_context.id.clone(),
                        target_id: target_id.to_owned(),
                        is_auxiliary_target_session: false,
                    });
                }
            },
        };

        match route {
            CdpSessionRoute::Browser => Some(
                self.browser_context
                    .as_ref()
                    .and_then(|browser_context| {
                        Some(TargetSessionOwner::PageTarget {
                            browser_context_id: browser_context.id.clone(),
                            target_id: browser_context.active_target_id()?.to_owned(),
                            is_auxiliary_target_session: false,
                        })
                    })
                    .unwrap_or(TargetSessionOwner::NoLoadedBrowserContext),
            ),
            CdpSessionRoute::BrowserContext { browser_context_id } => Some(
                self.browser_context_by_id(&browser_context_id)
                    .and_then(|browser_context| {
                        Some(TargetSessionOwner::PageTarget {
                            browser_context_id,
                            target_id: browser_context.active_target_id()?.to_owned(),
                            is_auxiliary_target_session: false,
                        })
                    })
                    .unwrap_or(TargetSessionOwner::NoLoadedBrowserContext),
            ),
            CdpSessionRoute::PageTarget {
                browser_context_id,
                target_id,
                is_attached_session,
            } => {
                let browser_context = self.browser_context_by_id(&browser_context_id)?;
                if browser_context.page_target(&target_id).is_some() {
                    Some(TargetSessionOwner::PageTarget {
                        browser_context_id,
                        target_id,
                        is_auxiliary_target_session: is_attached_session,
                    })
                } else {
                    Some(TargetSessionOwner::NoLoadedBrowserContext)
                }
            }
            CdpSessionRoute::TabTarget { .. }
            | CdpSessionRoute::SharedWorkerTarget { .. }
            | CdpSessionRoute::DedicatedWorkerTarget { .. }
            | CdpSessionRoute::ServiceWorkerTarget { .. } => {
                Some(TargetSessionOwner::NoLoadedBrowserContext)
            }
        }
    }
}
