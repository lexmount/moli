use crate::conn::{BrowserContext, CdpConnection, CdpSessionRoute, ParkedPageSessionState};

pub(super) enum TargetSessionOwner {
    ActiveTarget {
        browser_context_id: String,
        is_auxiliary_target_session: bool,
    },
    BackgroundTarget {
        browser_context_id: String,
        target_id: String,
        is_auxiliary_target_session: bool,
    },
    NoLoadedBrowserContext,
}

impl CdpConnection {
    pub(crate) fn with_background_target_session<R>(
        &mut self,
        session_id: Option<&str>,
        f: impl FnOnce(&mut BrowserContext, &str) -> R,
    ) -> Option<R> {
        let (browser_context_id, target_id) = self.background_target_route(session_id)?;
        let browser_context = self.browser_context_by_id_mut(&browser_context_id)?;
        Some(f(browser_context, &target_id))
    }

    pub(crate) fn mutate_background_target_page_session_state(
        &mut self,
        session_id: Option<&str>,
        f: impl FnOnce(&mut ParkedPageSessionState),
    ) -> bool {
        self.with_background_target_session(session_id, |browser_context, target_id| {
            browser_context.mutate_parked_page_session_state(target_id, f);
        })
        .is_some()
    }

    pub(super) fn target_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Option<TargetSessionOwner> {
        let route = match session_id {
            Some(session_id) => self.session_route(Some(session_id))?,
            None => match self.none_session_owner_route_override() {
                Some(route) => route,
                None => {
                    let Some(browser_context_id) = self
                        .browser_context
                        .as_ref()
                        .map(|browser_context| browser_context.id.clone())
                    else {
                        return Some(TargetSessionOwner::NoLoadedBrowserContext);
                    };
                    return Some(TargetSessionOwner::ActiveTarget {
                        browser_context_id,
                        is_auxiliary_target_session: false,
                    });
                }
            },
        };

        match route {
            CdpSessionRoute::Browser => self
                .browser_context
                .as_ref()
                .map(|browser_context| TargetSessionOwner::ActiveTarget {
                    browser_context_id: browser_context.id.clone(),
                    is_auxiliary_target_session: false,
                })
                .or(Some(TargetSessionOwner::NoLoadedBrowserContext)),
            CdpSessionRoute::ActiveTarget {
                browser_context_id, ..
            } => Some(TargetSessionOwner::ActiveTarget {
                browser_context_id,
                is_auxiliary_target_session: false,
            }),
            CdpSessionRoute::AuxiliaryTarget {
                browser_context_id,
                target_id,
            } => {
                let is_background_target = self
                    .browser_context_by_id(&browser_context_id)?
                    .background_target(&target_id)
                    .is_some();
                if is_background_target {
                    Some(TargetSessionOwner::BackgroundTarget {
                        browser_context_id,
                        target_id,
                        is_auxiliary_target_session: true,
                    })
                } else {
                    Some(TargetSessionOwner::ActiveTarget {
                        browser_context_id,
                        is_auxiliary_target_session: true,
                    })
                }
            }
            CdpSessionRoute::BackgroundTarget {
                browser_context_id,
                target_id,
            } => Some(TargetSessionOwner::BackgroundTarget {
                browser_context_id,
                target_id,
                is_auxiliary_target_session: false,
            }),
            CdpSessionRoute::TabTarget { .. }
            | CdpSessionRoute::SharedWorkerTarget { .. }
            | CdpSessionRoute::DedicatedWorkerTarget { .. }
            | CdpSessionRoute::ServiceWorkerTarget { .. } => {
                Some(TargetSessionOwner::NoLoadedBrowserContext)
            }
        }
    }
}
