use crate::conn::{BrowserContext, CdpConnection};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TargetHandlerAccessMode {
    Browser,
    Regular,
    AutoAttachOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CdpSessionRoute {
    Browser,
    TabTarget {
        browser_context_id: String,
        tab_target_id: String,
    },
    ActiveTarget {
        browser_context_id: String,
        target_id: Option<String>,
    },
    AuxiliaryTarget {
        browser_context_id: String,
        target_id: String,
    },
    BackgroundTarget {
        browser_context_id: String,
        target_id: String,
    },
    SharedWorkerTarget {
        browser_context_id: String,
        target_id: String,
    },
    DedicatedWorkerTarget {
        browser_context_id: String,
        target_id: String,
    },
    ServiceWorkerTarget {
        browser_context_id: String,
        target_id: String,
    },
}

impl CdpSessionRoute {
    pub(crate) fn browser_context_id(&self) -> Option<&str> {
        match self {
            Self::Browser => None,
            Self::TabTarget {
                browser_context_id, ..
            }
            | Self::ActiveTarget {
                browser_context_id, ..
            }
            | Self::AuxiliaryTarget {
                browser_context_id, ..
            }
            | Self::BackgroundTarget {
                browser_context_id, ..
            }
            | Self::SharedWorkerTarget {
                browser_context_id, ..
            }
            | Self::DedicatedWorkerTarget {
                browser_context_id, ..
            }
            | Self::ServiceWorkerTarget {
                browser_context_id, ..
            } => Some(browser_context_id),
        }
    }

    /// Whether Chromium installs a handler for this CDP domain on the routed
    /// DevToolsAgentHost.
    ///
    /// Page and worker capability checks remain in their domain handlers for
    /// now. Browser and Tab hosts are browser-only sessions in Chromium; they
    /// must not fall through to whichever Page happens to be active.
    pub(crate) fn supports_cdp_domain(&self, domain: &str) -> bool {
        match self {
            Self::Browser => matches!(
                domain,
                "Browser"
                    | "Fetch"
                    | "IO"
                    | "Security"
                    | "Storage"
                    | "SystemInfo"
                    | "Target"
                    | "Tracing"
            ),
            Self::TabTarget { .. } => matches!(domain, "IO" | "Target" | "Tracing"),
            Self::ActiveTarget { .. }
            | Self::AuxiliaryTarget { .. }
            | Self::BackgroundTarget { .. }
            | Self::SharedWorkerTarget { .. }
            | Self::DedicatedWorkerTarget { .. }
            | Self::ServiceWorkerTarget { .. } => true,
        }
    }

    pub(crate) fn target_handler_access_mode(&self) -> TargetHandlerAccessMode {
        match self {
            Self::Browser => TargetHandlerAccessMode::Browser,
            Self::TabTarget { .. }
            | Self::ActiveTarget { .. }
            | Self::AuxiliaryTarget { .. }
            | Self::BackgroundTarget { .. } => TargetHandlerAccessMode::Regular,
            Self::SharedWorkerTarget { .. }
            | Self::DedicatedWorkerTarget { .. }
            | Self::ServiceWorkerTarget { .. } => TargetHandlerAccessMode::AutoAttachOnly,
        }
    }
}
impl CdpConnection {
    pub(crate) fn target_handler_access_mode(
        &self,
        session_id: Option<&str>,
    ) -> TargetHandlerAccessMode {
        let Some(session_id) = session_id else {
            return TargetHandlerAccessMode::Browser;
        };
        self.session_route(Some(session_id))
            .map(|route| route.target_handler_access_mode())
            .unwrap_or(TargetHandlerAccessMode::Regular)
    }

    pub(crate) fn target_handler_may_get_target_info(
        &self,
        session_id: Option<&str>,
        target_id: Option<&str>,
    ) -> bool {
        if self.target_handler_access_mode(session_id) != TargetHandlerAccessMode::AutoAttachOnly {
            return true;
        }
        let Some(session_id) = session_id else {
            return false;
        };
        let owner_target_id = self.non_browser_target_id_for_session(Some(session_id));
        let Some(target_id) = target_id.or(owner_target_id.as_deref()) else {
            return false;
        };
        owner_target_id.as_deref() == Some(target_id)
    }

    pub(crate) fn target_handler_may_close_target(
        &self,
        session_id: Option<&str>,
        target_id: &str,
    ) -> bool {
        if self.target_handler_access_mode(session_id) != TargetHandlerAccessMode::AutoAttachOnly {
            return true;
        }
        let Some(session_id) = session_id else {
            return false;
        };
        self.non_browser_target_id_for_session(Some(session_id))
            .as_deref()
            == Some(target_id)
            || self
                .target_control
                .auto_attached_target_ids_for_owner(Some(session_id))
                .iter()
                .any(|attached_target_id| attached_target_id == target_id)
    }

    pub(crate) fn session_route(&self, session_id: Option<&str>) -> Option<CdpSessionRoute> {
        let session_id = session_id?;
        if self.browser_session_ids.contains(session_id) {
            return Some(CdpSessionRoute::Browser);
        }
        if let Some(tab_target_id) = self.tab_target_id_for_session_id(session_id)
            && let Some(browser_context_id) =
                self.browser_context_id_for_tab_target_id(tab_target_id)
        {
            return Some(CdpSessionRoute::TabTarget {
                browser_context_id,
                tab_target_id: tab_target_id.to_owned(),
            });
        }
        self.browser_contexts()
            .find_map(|bc| browser_context_session_route(bc, session_id))
    }

    pub(crate) fn target_session_route_for_target_id(
        &self,
        target_id: &str,
    ) -> Option<CdpSessionRoute> {
        if self
            .primary_page_target_id_for_tab_target_id(target_id)
            .is_some()
            && let Some(browser_context_id) = self.browser_context_id_for_tab_target_id(target_id)
        {
            return Some(CdpSessionRoute::TabTarget {
                browser_context_id,
                tab_target_id: target_id.to_owned(),
            });
        }
        self.browser_contexts().find_map(|browser_context| {
            if browser_context.active_target_id() == Some(target_id) {
                return Some(CdpSessionRoute::ActiveTarget {
                    browser_context_id: browser_context.id.clone(),
                    target_id: None,
                });
            }
            if browser_context.background_target(target_id).is_some() {
                return Some(CdpSessionRoute::BackgroundTarget {
                    browser_context_id: browser_context.id.clone(),
                    target_id: target_id.to_owned(),
                });
            }
            if browser_context.has_shared_worker_target(target_id) {
                return Some(CdpSessionRoute::SharedWorkerTarget {
                    browser_context_id: browser_context.id.clone(),
                    target_id: target_id.to_owned(),
                });
            }
            if browser_context.has_dedicated_worker_target(target_id) {
                return Some(CdpSessionRoute::DedicatedWorkerTarget {
                    browser_context_id: browser_context.id.clone(),
                    target_id: target_id.to_owned(),
                });
            }
            browser_context
                .has_service_worker_target(target_id)
                .then(|| CdpSessionRoute::ServiceWorkerTarget {
                    browser_context_id: browser_context.id.clone(),
                    target_id: target_id.to_owned(),
                })
        })
    }

    /// Returns the DevToolsAgentHost owned by a non-browser session.
    ///
    /// Chromium's Target domain uses its handler's `owner_target_id` when
    /// `Target.getTargetInfo` omits `targetId`. A tab session therefore owns
    /// the stable Tab target, while a page or worker session owns its exact
    /// execution target.
    pub(crate) fn non_browser_target_id_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<String> {
        match self.session_route(session_id)? {
            CdpSessionRoute::TabTarget {
                tab_target_id: target_id,
                ..
            }
            | CdpSessionRoute::AuxiliaryTarget { target_id, .. }
            | CdpSessionRoute::BackgroundTarget { target_id, .. }
            | CdpSessionRoute::SharedWorkerTarget { target_id, .. }
            | CdpSessionRoute::DedicatedWorkerTarget { target_id, .. }
            | CdpSessionRoute::ServiceWorkerTarget { target_id, .. }
            | CdpSessionRoute::ActiveTarget {
                target_id: Some(target_id),
                ..
            } => Some(target_id),
            CdpSessionRoute::ActiveTarget {
                browser_context_id,
                target_id: None,
            } => self
                .browser_context_by_id(&browser_context_id)
                .and_then(|browser_context| browser_context.active_target_id())
                .map(str::to_owned),
            CdpSessionRoute::Browser => None,
        }
    }

    pub fn worker_target_id_for_session(&self, session_id: Option<&str>) -> Option<String> {
        match self.session_route(session_id)? {
            CdpSessionRoute::SharedWorkerTarget { target_id, .. }
            | CdpSessionRoute::DedicatedWorkerTarget { target_id, .. }
            | CdpSessionRoute::ServiceWorkerTarget { target_id, .. } => Some(target_id),
            _ => None,
        }
    }

    pub(crate) fn target_session_route_for_child_frame_id(
        &self,
        frame_id: &str,
    ) -> Option<CdpSessionRoute> {
        self.browser_contexts().find_map(|browser_context| {
            if browser_context
                .active_target
                .owner_state
                .has_attached_child_frame_id(frame_id)
            {
                return Some(CdpSessionRoute::ActiveTarget {
                    browser_context_id: browser_context.id.clone(),
                    target_id: None,
                });
            }
            browser_context
                .background_targets
                .iter()
                .find_map(|target| {
                    browser_context
                        .parked_target_owner_state(target.target_id())
                        .is_some_and(|owner_state| {
                            owner_state.has_attached_child_frame_id(frame_id)
                        })
                        .then(|| CdpSessionRoute::BackgroundTarget {
                            browser_context_id: browser_context.id.clone(),
                            target_id: target.target_id().to_owned(),
                        })
                })
        })
    }

    pub(crate) fn has_attached_child_frame_id(&self, frame_id: &str) -> bool {
        self.browser_contexts()
            .any(|browser_context| browser_context.has_attached_child_frame_id(frame_id))
    }

    #[cfg(test)]
    pub(crate) fn has_background_target_session(&self, session_id: Option<&str>) -> bool {
        self.background_target_route(session_id).is_some()
    }

    #[cfg(test)]
    pub(crate) fn background_target_id_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<String> {
        self.background_target_route(session_id)
            .map(|(_, target_id)| target_id)
    }

    pub(crate) fn background_target_route(
        &self,
        session_id: Option<&str>,
    ) -> Option<(String, String)> {
        match self.session_route(session_id)? {
            CdpSessionRoute::BackgroundTarget {
                browser_context_id,
                target_id,
            } => Some((browser_context_id, target_id)),
            CdpSessionRoute::AuxiliaryTarget {
                browser_context_id,
                target_id,
            } => self
                .browser_context_by_id(&browser_context_id)?
                .background_target(&target_id)
                .is_some()
                .then_some((browser_context_id, target_id)),
            CdpSessionRoute::Browser
            | CdpSessionRoute::TabTarget { .. }
            | CdpSessionRoute::ActiveTarget { .. }
            | CdpSessionRoute::SharedWorkerTarget { .. }
            | CdpSessionRoute::DedicatedWorkerTarget { .. }
            | CdpSessionRoute::ServiceWorkerTarget { .. } => None,
        }
    }
}
fn browser_context_session_route(
    browser_context: &BrowserContext,
    session_id: &str,
) -> Option<CdpSessionRoute> {
    if browser_context.active_session_id() == Some(session_id) {
        return Some(CdpSessionRoute::ActiveTarget {
            browser_context_id: browser_context.id.clone(),
            target_id: browser_context.active_target_id().map(str::to_owned),
        });
    }

    if let Some(target_id) = browser_context.auxiliary_target_id_for_session(session_id) {
        return Some(CdpSessionRoute::AuxiliaryTarget {
            browser_context_id: browser_context.id.clone(),
            target_id: target_id.to_owned(),
        });
    }

    browser_context
        .background_targets
        .iter()
        .find(|target| target.is_session(session_id))
        .map(|target| CdpSessionRoute::BackgroundTarget {
            browser_context_id: browser_context.id.clone(),
            target_id: target.target_id().to_owned(),
        })
        .or_else(|| {
            browser_context
                .shared_worker_target_id_for_session(session_id)
                .map(|target_id| CdpSessionRoute::SharedWorkerTarget {
                    browser_context_id: browser_context.id.clone(),
                    target_id: target_id.to_owned(),
                })
                .or_else(|| {
                    browser_context
                        .dedicated_worker_target_id_for_session(session_id)
                        .map(|target_id| CdpSessionRoute::DedicatedWorkerTarget {
                            browser_context_id: browser_context.id.clone(),
                            target_id: target_id.to_owned(),
                        })
                        .or_else(|| {
                            browser_context
                                .service_worker_target_id_for_session(session_id)
                                .map(|target_id| CdpSessionRoute::ServiceWorkerTarget {
                                    browser_context_id: browser_context.id.clone(),
                                    target_id: target_id.to_owned(),
                                })
                        })
                })
        })
}
