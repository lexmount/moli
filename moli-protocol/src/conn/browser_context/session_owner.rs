use crate::conn::{BrowserContext, CdpConnection, CdpSessionRoute, TargetPageState};

pub(super) enum TargetSessionOwner {
    ActiveTarget {
        browser_context_id: String,
        is_auxiliary_target_session: bool,
    },
    PageTargetHost {
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
        f: impl FnOnce(&mut TargetPageState),
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
                    if !browser_context.has_active_target() {
                        return Some(TargetSessionOwner::NoLoadedBrowserContext);
                    }
                    return Some(TargetSessionOwner::ActiveTarget {
                        browser_context_id: browser_context.id.clone(),
                        is_auxiliary_target_session: false,
                    });
                }
            },
        };

        match route {
            CdpSessionRoute::Browser => Some(
                self.browser_context
                    .as_ref()
                    .filter(|browser_context| browser_context.has_active_target())
                    .map(|browser_context| TargetSessionOwner::ActiveTarget {
                        browser_context_id: browser_context.id.clone(),
                        is_auxiliary_target_session: false,
                    })
                    .unwrap_or(TargetSessionOwner::NoLoadedBrowserContext),
            ),
            CdpSessionRoute::ActiveTarget {
                browser_context_id,
                target_id,
            } => {
                let browser_context = self.browser_context_by_id(&browser_context_id)?;
                match target_id {
                    Some(target_id) if browser_context.is_active_target(&target_id) => {
                        Some(TargetSessionOwner::ActiveTarget {
                            browser_context_id,
                            is_auxiliary_target_session: false,
                        })
                    }
                    Some(target_id) if browser_context.page_target(&target_id).is_some() => {
                        Some(TargetSessionOwner::PageTargetHost {
                            browser_context_id,
                            target_id,
                            is_auxiliary_target_session: false,
                        })
                    }
                    None if browser_context.has_active_target() => {
                        Some(TargetSessionOwner::ActiveTarget {
                            browser_context_id,
                            is_auxiliary_target_session: false,
                        })
                    }
                    _ => Some(TargetSessionOwner::NoLoadedBrowserContext),
                }
            }
            CdpSessionRoute::AuxiliaryTarget {
                browser_context_id,
                target_id,
            } => {
                let is_background_target = self
                    .browser_context_by_id(&browser_context_id)?
                    .background_target(&target_id)
                    .is_some();
                if is_background_target {
                    Some(TargetSessionOwner::PageTargetHost {
                        browser_context_id,
                        target_id,
                        is_auxiliary_target_session: true,
                    })
                } else if self
                    .browser_context_by_id(&browser_context_id)?
                    .is_active_target(&target_id)
                {
                    Some(TargetSessionOwner::ActiveTarget {
                        browser_context_id,
                        is_auxiliary_target_session: true,
                    })
                } else {
                    Some(TargetSessionOwner::NoLoadedBrowserContext)
                }
            }
            CdpSessionRoute::PageTargetHost {
                browser_context_id,
                target_id,
            } => {
                let browser_context = self.browser_context_by_id(&browser_context_id)?;
                if browser_context.page_target(&target_id).is_some() {
                    Some(TargetSessionOwner::PageTargetHost {
                        browser_context_id,
                        target_id,
                        is_auxiliary_target_session: false,
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
