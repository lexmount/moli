use crate::conn::CdpConnection;
use moli_page_types::DevToolsSessionKey;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TargetHandlerAccessMode {
    Browser,
    Regular,
    AutoAttachOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CdpSessionRoute {
    Browser,
    /// Internal owner route for context-scoped work that may run before the
    /// context has a Page target. External CDP sessions never use this route.
    BrowserContext {
        browser_context_id: String,
    },
    TabTarget {
        browser_context_id: String,
        tab_target_id: String,
    },
    PageTarget {
        browser_context_id: String,
        target_id: String,
        session_key: DevToolsSessionKey,
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
    pub(crate) fn addresses_same_target_as(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::PageTarget {
                    browser_context_id: left_context,
                    target_id: left_target,
                    ..
                },
                Self::PageTarget {
                    browser_context_id: right_context,
                    target_id: right_target,
                    ..
                },
            ) => left_context == right_context && left_target == right_target,
            _ => self == other,
        }
    }

    pub(crate) fn browser_context_id(&self) -> Option<&str> {
        match self {
            Self::Browser => None,
            Self::BrowserContext { browser_context_id }
            | Self::TabTarget {
                browser_context_id, ..
            }
            | Self::PageTarget {
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

    #[cfg(test)]
    pub(crate) fn target_id(&self) -> Option<&str> {
        match self {
            Self::TabTarget { tab_target_id, .. } => Some(tab_target_id),
            Self::PageTarget { target_id, .. }
            | Self::SharedWorkerTarget { target_id, .. }
            | Self::DedicatedWorkerTarget { target_id, .. }
            | Self::ServiceWorkerTarget { target_id, .. } => Some(target_id),
            Self::Browser | Self::BrowserContext { .. } => None,
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
            Self::BrowserContext { .. } | Self::PageTarget { .. } => true,
            Self::TabTarget { .. } => matches!(domain, "IO" | "Target" | "Tracing"),
            Self::SharedWorkerTarget { .. }
            | Self::DedicatedWorkerTarget { .. }
            | Self::ServiceWorkerTarget { .. } => true,
        }
    }

    pub(crate) fn target_handler_access_mode(&self) -> TargetHandlerAccessMode {
        match self {
            Self::Browser => TargetHandlerAccessMode::Browser,
            Self::BrowserContext { .. } | Self::TabTarget { .. } | Self::PageTarget { .. } => {
                TargetHandlerAccessMode::Regular
            }
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
        self.target_control
            .attached_session_route(session_id)
            .cloned()
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
            if browser_context.page_target(target_id).is_some() {
                return Some(CdpSessionRoute::PageTarget {
                    browser_context_id: browser_context.id.clone(),
                    target_id: target_id.to_owned(),
                    session_key: DevToolsSessionKey::Primary,
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
            | CdpSessionRoute::PageTarget { target_id, .. }
            | CdpSessionRoute::SharedWorkerTarget { target_id, .. }
            | CdpSessionRoute::DedicatedWorkerTarget { target_id, .. }
            | CdpSessionRoute::ServiceWorkerTarget { target_id, .. } => Some(target_id),
            CdpSessionRoute::Browser | CdpSessionRoute::BrowserContext { .. } => None,
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
            browser_context.page_targets.iter().find_map(|target| {
                target
                    .owner_state
                    .has_attached_child_frame_id(frame_id)
                    .then(|| CdpSessionRoute::PageTarget {
                        browser_context_id: browser_context.id.clone(),
                        target_id: target.target_id().to_owned(),
                        session_key: DevToolsSessionKey::Primary,
                    })
            })
        })
    }

    pub(crate) fn has_attached_child_frame_id(&self, frame_id: &str) -> bool {
        self.browser_contexts()
            .any(|browser_context| browser_context.has_attached_child_frame_id(frame_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{BrowserContext, PageTargetHost};

    #[test]
    fn committed_page_session_route_is_stable_across_foreground_selection() {
        let mut connection = CdpConnection::new();
        let mut browser_context = BrowserContext::new("BID-route".to_owned());
        browser_context.set_active_target_id("TID-a");
        browser_context.attach_active_session("SID-a");
        assert!(browser_context.insert_page_target_host(PageTargetHost::empty("TID-b".to_owned())));
        connection.install_browser_context_fixture_for_test(browser_context);

        let route = CdpSessionRoute::PageTarget {
            browser_context_id: "BID-route".to_owned(),
            target_id: "TID-a".to_owned(),
            session_key: DevToolsSessionKey::Primary,
        };
        connection.target_control.commit_attached_session(
            "SID-a".to_owned(),
            None,
            "TID-a",
            route.clone(),
            false,
            false,
        );

        connection
            .browser_context
            .as_mut()
            .expect("browser context")
            .set_active_target_id("TID-b");

        assert_eq!(connection.session_route(Some("SID-a")), Some(route));
        assert_eq!(
            connection
                .browser_context
                .as_ref()
                .and_then(BrowserContext::active_target_id),
            Some("TID-b")
        );
    }

    #[test]
    fn target_binding_is_not_globally_routable_before_session_commit() {
        let mut connection = CdpConnection::new();
        let mut browser_context = BrowserContext::new_with_page_for_test("BID-route", "TID-page");
        browser_context.attach_active_session("SID-prepared");
        connection.browser_context = Some(browser_context);

        assert_eq!(connection.session_route(Some("SID-prepared")), None);
    }
}
