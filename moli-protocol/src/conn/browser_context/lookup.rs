use super::*;

impl CdpConnection {
    pub fn browser_contexts(&self) -> impl Iterator<Item = &BrowserContext> {
        self.browser_context
            .iter()
            .chain(self.inactive_browser_contexts.iter())
    }

    pub fn browser_context_by_id(&self, browser_context_id: &str) -> Option<&BrowserContext> {
        self.browser_context
            .as_ref()
            .filter(|bc| bc.id == browser_context_id)
            .or_else(|| {
                self.inactive_browser_contexts
                    .iter()
                    .find(|bc| bc.id == browser_context_id)
            })
    }

    pub fn browser_context_by_id_mut(
        &mut self,
        browser_context_id: &str,
    ) -> Option<&mut BrowserContext> {
        if self
            .browser_context
            .as_ref()
            .is_some_and(|bc| bc.id == browser_context_id)
        {
            return self.browser_context.as_mut();
        }
        self.inactive_browser_contexts
            .iter_mut()
            .find(|bc| bc.id == browser_context_id)
    }

    pub(crate) fn browser_context_for_command_session_mut(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<&mut BrowserContext, (i32, &'static str)> {
        let route = match session_id {
            Some(session_id) => self
                .session_route(Some(session_id))
                .ok_or((-32001, "Unknown sessionId"))?,
            None => CdpSessionRoute::Browser,
        };
        match route {
            CdpSessionRoute::Browser => self
                .browser_context
                .as_mut()
                .ok_or((-31998, "BrowserContextNotLoaded")),
            CdpSessionRoute::TabTarget { .. } => Err((-31998, "DirectSessionRouteRequired")),
            CdpSessionRoute::ActiveTarget {
                browser_context_id, ..
            }
            | CdpSessionRoute::AuxiliaryTarget {
                browser_context_id, ..
            }
            | CdpSessionRoute::PageTargetHost {
                browser_context_id, ..
            }
            | CdpSessionRoute::SharedWorkerTarget {
                browser_context_id, ..
            }
            | CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id, ..
            }
            | CdpSessionRoute::ServiceWorkerTarget {
                browser_context_id, ..
            } => self
                .browser_context_by_id_mut(&browser_context_id)
                .ok_or((-31998, "UnknownBrowserContextId")),
        }
    }

    pub fn has_browser_context_id(&self, browser_context_id: &str) -> bool {
        self.browser_contexts()
            .any(|bc| bc.id == browser_context_id)
    }

    pub(crate) fn browser_context_id_for_target(&self, target_id: &str) -> Option<&str> {
        self.browser_contexts()
            .find(|bc| bc.is_active_target(target_id) || bc.background_target(target_id).is_some())
            .map(|bc| bc.id.as_str())
    }

    pub(crate) async fn apply_parked_target_surface_overrides_async(
        &mut self,
        target_id: &str,
    ) -> Result<bool, String> {
        let browser_context_id = self
            .browser_context_id_for_target(target_id)
            .map(str::to_owned)
            .ok_or_else(|| "UnknownTargetId".to_owned())?;
        self.browser_context_by_id_mut(&browser_context_id)
            .ok_or_else(|| "UnknownBrowserContextId".to_owned())?
            .apply_parked_target_surface_overrides_async(target_id)
            .await
    }
}
