use super::BrowserContext;
use moli_core::network::BrowserResourceRuntime;
use moli_core::page::{
    CompletedPageCommand, EmulatedIdleOverride, EmulatedMediaOverrides, PendingPageCommand,
    ViewportSurface,
};

pub(crate) enum PagePolicyUpdateKind {
    SetExtraHttpHeaders,
    SetLocaleOverride,
    SetNetworkConditions,
    SetCpuThrottlingRate,
    SetIdleOverride,
    SetTimezoneOverride,
    SetEmulatedMedia,
    SetViewportSurface,
    ReplaceBrowserResourceRuntime,
}

impl BrowserContext {
    pub(crate) fn start_target_permission_update(
        &self,
        target_id: &str,
        overrides: &[moli_core::page::PermissionOverrideRegistration],
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_set_permission_overrides(overrides)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_target_permission_update(
        &mut self,
        target_id: &str,
        completion: CompletedPageCommand,
    ) -> Result<(), String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .finish_set_permission_overrides(completion)
            .map_err(|error| error.to_string())
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn reconcile_target_runtime_policy_async(
        &mut self,
        target_id: &str,
        script_execution_disabled: bool,
        media: &EmulatedMediaOverrides,
        cpu_throttling_rate: f64,
        network_offline: bool,
        viewport: Option<ViewportSurface>,
        viewport_script: &str,
    ) -> anyhow::Result<()> {
        let Some(page) = self.loaded_page_for_target_mut(target_id) else {
            return Ok(());
        };
        let mut first_error = None;
        let mut record = |surface: &str, result: anyhow::Result<()>| {
            if let Err(error) = result {
                first_error.get_or_insert_with(|| anyhow::anyhow!("{surface}: {error}"));
            }
        };
        // Each effective policy receives an attempt, even after an earlier admission failure.
        record(
            "script execution",
            page.set_script_execution_disabled_async(script_execution_disabled)
                .await,
        );
        record("emulated media", page.set_emulated_media_async(media).await);
        record(
            "CPU throttling",
            page.set_cpu_throttling_rate_async(cpu_throttling_rate)
                .await,
        );
        record(
            "network conditions",
            page.set_network_offline_async(network_offline).await,
        );
        record(
            "device metrics viewport",
            page.set_viewport_surface_async(viewport).await,
        );
        record(
            "device metrics script",
            page.run_page_surface_override_script_async(viewport_script)
                .await,
        );
        first_error.map_or(Ok(()), Err)
    }

    pub(crate) fn start_set_cpu_throttling_rate_for_target(
        &self,
        target_id: &str,
        rate: f64,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_set_cpu_throttling_rate(rate)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_set_idle_override_for_target(
        &mut self,
        target_id: &str,
        idle_override: Option<EmulatedIdleOverride>,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target_mut(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_set_idle_override(idle_override)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_set_timezone_override_for_target(
        &self,
        target_id: &str,
        timezone: Option<&str>,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_set_timezone_override(timezone)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_set_locale_override_for_target(
        &self,
        target_id: &str,
        locale: Option<&str>,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_set_locale_override(locale)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_set_emulated_media_for_target(
        &self,
        target_id: &str,
        overrides: &EmulatedMediaOverrides,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_set_emulated_media(overrides)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_set_viewport_surface_for_target(
        &self,
        target_id: &str,
        viewport_surface: Option<ViewportSurface>,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_set_viewport_surface(viewport_surface)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_set_extra_http_headers_for_target(
        &self,
        target_id: &str,
        headers: &[(String, String)],
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_set_extra_http_headers(headers)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_replace_browser_resource_runtime_for_target(
        &self,
        target_id: &str,
        resource_runtime: &BrowserResourceRuntime,
    ) -> Result<PendingPageCommand, String> {
        self.loaded_page_for_target(target_id)
            .ok_or("NoDocumentLoaded")?
            .start_replace_browser_resource_runtime(resource_runtime)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_target_page_policy_update(
        &mut self,
        target_id: &str,
        kind: PagePolicyUpdateKind,
        completion: CompletedPageCommand,
    ) -> Result<(), String> {
        if let Some(page) = self.loaded_page_for_target_mut(target_id)
            && completion.is_from_page(page)
        {
            return match kind {
                PagePolicyUpdateKind::SetExtraHttpHeaders => {
                    page.finish_set_extra_http_headers(completion)
                }
                PagePolicyUpdateKind::SetLocaleOverride => {
                    page.finish_set_locale_override(completion)
                }
                PagePolicyUpdateKind::SetNetworkConditions => {
                    page.finish_set_network_offline(completion)
                }
                PagePolicyUpdateKind::SetCpuThrottlingRate => {
                    page.finish_set_cpu_throttling_rate(completion)
                }
                PagePolicyUpdateKind::SetIdleOverride => page.finish_set_idle_override(completion),
                PagePolicyUpdateKind::SetTimezoneOverride => {
                    page.finish_set_timezone_override(completion)
                }
                PagePolicyUpdateKind::SetEmulatedMedia => {
                    page.finish_set_emulated_media(completion)
                }
                PagePolicyUpdateKind::SetViewportSurface => {
                    page.finish_set_viewport_surface(completion)
                }
                PagePolicyUpdateKind::ReplaceBrowserResourceRuntime => {
                    page.finish_replace_browser_resource_runtime(completion)
                }
            }
            .map_err(|error| error.to_string());
        }
        Self::finish_unobserved_page_policy_update(completion)
    }

    pub(crate) fn finish_unobserved_page_policy_update(
        completion: CompletedPageCommand,
    ) -> Result<(), String> {
        completion
            .into_unit_page_command_turn()
            .map(drop)
            .map_err(|error| {
                format!("stale Emulation command returned an unexpected reply: {error}")
            })
    }
}
