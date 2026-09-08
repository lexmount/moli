use crate::{
    browser::{
        MainFrameSlotId, WebContentsHandle, WebContentsId,
        web_contents::{
            EmulationPolicy, EmulationPolicyChange, InitialDocumentSnapshot, NetworkRequestPolicy,
            WebContents, WindowOpener, WindowSurface, WindowSurfaceState,
        },
    },
    runtime::{NavigationEngine, NavigationRuntimeConfig},
};

use super::BrowserContext;

impl BrowserContext {
    pub fn bind_page_navigation_engines(
        &mut self,
        config: NavigationRuntimeConfig,
        renderer_output_transport_sender: Option<crate::RendererOutputTransportSender>,
    ) {
        self.page_navigation_runtime_config = Some(config.clone());
        if let Some(sender) = renderer_output_transport_sender {
            self.set_renderer_output_transport_sender(sender);
        }

        let sender = self.renderer_output_transport_sender.clone();
        let runtime = self.renderer_runtime_owner_access();
        for contents in self.web_contents.values_mut() {
            if contents.has_navigation_engine() {
                continue;
            }
            let engine = NavigationEngine::new_with_runtime_config_and_browser_context_access(
                config.clone(),
                runtime.clone(),
            )
            .expect("live BrowserContext owner must accept a page engine");
            if let Some(sender) = sender.clone() {
                engine.set_renderer_output_transport_sender(sender);
            }
            contents.install_navigation_engine(engine);
        }
    }

    pub fn set_renderer_output_transport_sender(
        &mut self,
        sender: crate::RendererOutputTransportSender,
    ) {
        self.renderer_output_transport_sender = Some(sender.clone());
        for contents in self.web_contents.values() {
            contents.set_renderer_output_transport_sender(sender.clone());
        }
    }

    pub fn register_web_contents(
        &mut self,
        mut contents: WebContents,
    ) -> Result<(WebContentsHandle, MainFrameSlotId), String> {
        let id = contents.id();
        let main_frame = contents.main_frame.id();
        if self.web_contents.contains_key(&id) {
            return Err("WebContents identity must be unique".into());
        }
        if let Some(config) = self.page_navigation_runtime_config.clone() {
            contents.install_navigation_engine(self.new_page_navigation_engine(config));
        }
        self.web_contents.insert(id, contents);
        Ok((WebContentsHandle::new(self.id, id), main_frame))
    }

    pub fn contains_web_contents(&self, handle: WebContentsHandle) -> bool {
        self.web_contents(handle).is_ok()
    }

    pub fn web_contents_count(&self) -> usize {
        self.web_contents.len()
    }

    pub(in crate::browser) fn web_contents_handles(
        &self,
    ) -> impl Iterator<Item = WebContentsHandle> + '_ {
        self.web_contents
            .keys()
            .map(|id| WebContentsHandle::new(self.id, *id))
    }

    pub fn selected_web_contents_handle(&self) -> Option<WebContentsHandle> {
        self.selected_web_contents
            .map(|selection| selection.web_contents)
    }

    pub fn web_contents_handle_for_window_id(&self, window_id: u64) -> Option<WebContentsHandle> {
        let id = self
            .web_contents
            .keys()
            .copied()
            .find(|id| id.get() == window_id)?;
        Some(WebContentsHandle::new(self.id, id))
    }

    pub fn web_contents_handle_for_window_name(&self, name: &str) -> Option<WebContentsHandle> {
        let id = self
            .web_contents
            .values()
            .find(|contents| contents.window.name.as_deref() == Some(name))?
            .id();
        Some(WebContentsHandle::new(self.id, id))
    }

    pub fn web_contents_opener(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<(WebContentsId, bool)>, String> {
        Ok(self
            .web_contents(handle)?
            .window
            .opener
            .map(|opener| (opener.web_contents_id, opener.can_access)))
    }

    pub fn web_contents_window_name(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<&str>, String> {
        Ok(self.web_contents(handle)?.window.name.as_deref())
    }

    pub fn clone_session_storage_namespace(
        &self,
        web_contents: WebContentsId,
    ) -> Option<crate::browser::web_contents::SessionStorageNamespace> {
        self.web_contents
            .get(&web_contents)
            .map(|contents| contents.session_storage.deep_clone())
    }

    pub fn web_contents_is_crashed(&self, handle: WebContentsHandle) -> Result<bool, String> {
        Ok(self.web_contents(handle)?.crashed)
    }

    pub fn performance_metric_snapshot(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<crate::page::RendererPerformanceMetricSnapshot>, String> {
        Ok(self.web_contents(handle)?.performance_metric_snapshot())
    }

    pub fn observe_renderer_page_state(
        &mut self,
        handle: WebContentsHandle,
        snapshot: &std::sync::Arc<moli_renderer_v8::RendererPageState>,
    ) -> Result<bool, String> {
        Ok(self
            .web_contents_mut(handle)?
            .observe_renderer_page_state(snapshot))
    }

    pub fn set_web_contents_crashed(
        &mut self,
        handle: WebContentsHandle,
        crashed: bool,
    ) -> Result<(), String> {
        self.web_contents_mut(handle)?.crashed = crashed;
        Ok(())
    }

    pub fn web_contents_initial_document_state(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<InitialDocumentSnapshot>, String> {
        Ok(self
            .web_contents(handle)?
            .navigation()
            .initial_empty_document_state()
            .map(Into::into))
    }

    pub fn web_contents_window_surface(
        &self,
        handle: WebContentsHandle,
    ) -> Result<WindowSurface, String> {
        Ok(self.web_contents(handle)?.window.surface)
    }

    pub fn update_web_contents_window_surface(
        &mut self,
        handle: WebContentsHandle,
        state: Option<WindowSurfaceState>,
        width: Option<u32>,
        height: Option<u32>,
        x: Option<i32>,
        y: Option<i32>,
    ) -> Result<(), String> {
        self.web_contents_mut(handle)?
            .window
            .surface
            .update(state, width, height, x, y);
        Ok(())
    }

    pub fn set_web_contents_window_name(
        &mut self,
        handle: WebContentsHandle,
        name: Option<String>,
    ) -> Result<(), String> {
        self.web_contents(handle)?;
        for contents in self.web_contents.values_mut() {
            if contents.id() == handle.id() {
                contents.window.name = name.clone();
            } else if name.is_some() && contents.window.name == name {
                contents.window.name = None;
            }
        }
        Ok(())
    }

    pub fn set_web_contents_opener(
        &mut self,
        handle: WebContentsHandle,
        opener: Option<WebContentsHandle>,
        can_access: bool,
    ) -> Result<(), String> {
        if let Some(opener) = opener {
            self.web_contents(opener)?;
            self.web_contents_mut(handle)?.window.opener = Some(WindowOpener {
                web_contents_id: opener.id(),
                can_access,
            });
        } else {
            self.web_contents_mut(handle)?.window.opener = None;
        }
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn set_web_contents_opener_for_test(
        &mut self,
        handle: WebContentsHandle,
        opener: WebContentsId,
        can_access: bool,
    ) -> Result<(), String> {
        self.web_contents_mut(handle)?.window.opener = Some(WindowOpener {
            web_contents_id: opener,
            can_access,
        });
        Ok(())
    }

    pub fn set_web_contents_network_offline(
        &mut self,
        handle: WebContentsHandle,
        offline: bool,
    ) -> Result<(), String> {
        self.web_contents_mut(handle)?.set_network_offline(offline);
        Ok(())
    }

    pub fn web_contents_network_offline(&self, handle: WebContentsHandle) -> Result<bool, String> {
        Ok(self.web_contents(handle)?.network_offline)
    }

    pub fn web_contents_tls_verify_host_override(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<bool>, String> {
        Ok(self.web_contents(handle)?.tls_verify_host_override)
    }

    pub fn set_web_contents_tls_verify_host_override(
        &mut self,
        handle: WebContentsHandle,
        enabled: Option<bool>,
    ) -> Result<(), String> {
        self.web_contents_mut(handle)?
            .set_tls_verify_host_override(enabled);
        Ok(())
    }

    pub fn web_contents_bypass_content_security_policy(
        &self,
        handle: WebContentsHandle,
    ) -> Result<bool, String> {
        Ok(self.web_contents(handle)?.bypass_content_security_policy)
    }

    pub fn set_web_contents_bypass_content_security_policy(
        &mut self,
        handle: WebContentsHandle,
        bypass: bool,
    ) -> Result<(), String> {
        self.web_contents_mut(handle)?
            .set_bypass_content_security_policy(bypass);
        Ok(())
    }

    pub fn web_contents_browser_identity_override(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<moli_browser_profile::BrowserIdentityProfile>, String> {
        Ok(self.web_contents(handle)?.browser_identity_override.clone())
    }

    pub fn set_web_contents_browser_identity_override(
        &mut self,
        handle: WebContentsHandle,
        identity: Option<moli_browser_profile::BrowserIdentityProfile>,
    ) -> Result<(), String> {
        self.web_contents_mut(handle)?
            .set_browser_identity_override(identity);
        Ok(())
    }

    pub fn web_contents_network_request_policy(
        &self,
        handle: WebContentsHandle,
    ) -> Result<NetworkRequestPolicy, String> {
        Ok(self.web_contents(handle)?.network_request_policy.clone())
    }

    pub fn set_web_contents_network_request_policy(
        &mut self,
        handle: WebContentsHandle,
        policy: NetworkRequestPolicy,
    ) -> Result<(), String> {
        self.web_contents_mut(handle)?
            .set_network_request_policy(policy);
        Ok(())
    }

    pub fn web_contents_locale_override(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<String>, String> {
        Ok(self.web_contents(handle)?.locale_override.clone())
    }

    pub fn set_web_contents_locale_override(
        &mut self,
        handle: WebContentsHandle,
        locale: Option<String>,
    ) -> Result<(), String> {
        self.web_contents_mut(handle)?.set_locale_override(locale);
        Ok(())
    }

    pub fn web_contents_timezone_override(
        &self,
        handle: WebContentsHandle,
    ) -> Result<Option<String>, String> {
        Ok(self.web_contents(handle)?.timezone_override.clone())
    }

    pub fn set_web_contents_timezone_override(
        &mut self,
        handle: WebContentsHandle,
        timezone: Option<String>,
    ) -> Result<(), String> {
        self.web_contents_mut(handle)?
            .set_timezone_override(timezone);
        Ok(())
    }

    pub fn web_contents_has_non_default_policy(
        &self,
        handle: WebContentsHandle,
    ) -> Result<bool, String> {
        let contents = self.web_contents(handle)?;
        Ok(contents.network_offline
            || contents.browser_identity_override.is_some()
            || contents.tls_verify_host_override.is_some()
            || contents.bypass_content_security_policy
            || contents.locale_override.is_some()
            || contents.timezone_override.is_some()
            || contents.emulation_policy != EmulationPolicy::default()
            || contents.network_request_policy != NetworkRequestPolicy::default())
    }

    pub fn web_contents_window_counts(&self) -> (usize, usize, usize) {
        let opener = self
            .web_contents
            .values()
            .filter(|contents| contents.window.opener.is_some())
            .count();
        let accessible_opener = self
            .web_contents
            .values()
            .filter(|contents| {
                contents
                    .window
                    .opener
                    .is_some_and(|opener| opener.can_access)
            })
            .count();
        let named = self
            .web_contents
            .values()
            .filter(|contents| contents.window.name.is_some())
            .count();
        (opener, accessible_opener, named)
    }

    pub fn web_contents_emulation_policy(
        &self,
        handle: WebContentsHandle,
    ) -> Result<EmulationPolicy, String> {
        Ok(self.web_contents(handle)?.emulation_policy.clone())
    }

    pub fn apply_web_contents_emulation_policy_change(
        &mut self,
        handle: WebContentsHandle,
        change: EmulationPolicyChange,
    ) -> Result<(), String> {
        self.web_contents_mut(handle)?
            .emulation_policy
            .apply(change);
        Ok(())
    }

    pub fn apply_web_contents_emulation_policy_changes(
        &mut self,
        handle: WebContentsHandle,
        changes: Vec<EmulationPolicyChange>,
    ) -> Result<(), String> {
        self.web_contents_mut(handle)?
            .emulation_policy
            .apply_changes(changes);
        Ok(())
    }

    pub fn loaded_document_count(&self) -> usize {
        self.web_contents
            .values()
            .filter(|contents| contents.main_frame.current_document.is_some())
            .count()
    }

    pub fn has_pending_javascript_dialog(&self) -> bool {
        self.web_contents
            .values()
            .any(|contents| !contents.javascript_dialogs.is_empty())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn navigation_engine_for_test(
        &self,
        handle: WebContentsHandle,
    ) -> Option<&NavigationEngine> {
        self.web_contents(handle).ok()?.navigation_engine_for_test()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn navigation_engine_for_test_mut(
        &mut self,
        handle: WebContentsHandle,
    ) -> Option<&mut NavigationEngine> {
        self.web_contents_mut(handle)
            .ok()?
            .navigation_engine_for_test_mut()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn take_document_host_for_test(
        &mut self,
        handle: WebContentsHandle,
    ) -> Option<crate::browser::web_contents::DocumentHost> {
        self.web_contents_mut(handle)
            .ok()?
            .main_frame
            .current_document
            .take()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn web_contents_id_for_test(&self, handle: WebContentsHandle) -> Option<WebContentsId> {
        Some(self.web_contents(handle).ok()?.id())
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn web_contents_for_test(&self, handle: WebContentsHandle) -> Option<&WebContents> {
        self.web_contents(handle).ok()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn web_contents_for_test_mut(
        &mut self,
        handle: WebContentsHandle,
    ) -> Option<&mut WebContents> {
        self.web_contents_mut(handle).ok()
    }
}
