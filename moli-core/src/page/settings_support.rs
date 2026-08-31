use anyhow::Result;

use super::document_cookie_diagnostics::{
    DocumentCookieFacadeTelemetrySnapshot, DocumentCookieOwnerSnapshot,
};
use super::protocol_support::{
    EmulatedIdleOverride, EmulatedMediaOverrides, PermissionOverrideRegistration,
    SubresourceResourceType, ViewportSurface,
};
use super::{CompletedPageCommand, Page, PendingDevToolsIoCommandDispatch, PendingPageCommand};
use crate::renderer::{
    RendererPageCommand, RendererPageCookieFacadeSnapshotReply, RendererPageReply,
    RendererRuntimeInspectorResponseSender,
};
use moli_renderer_v8::network::BrowserResourceRuntime;

impl Page {
    pub async fn set_fetch_subresource_interception_async(
        &mut self,
        enabled: bool,
        resource_type: Option<SubresourceResourceType>,
    ) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::SetFetchSubresourceInterception {
                enabled,
                resource_type,
            },
            "set fetch subresource interception",
        )
        .await
    }

    pub fn start_set_fetch_subresource_interception(
        &self,
        enabled: bool,
        resource_type: Option<SubresourceResourceType>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SetFetchSubresourceInterception {
            enabled,
            resource_type,
        })
    }

    pub fn finish_set_fetch_subresource_interception(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "set fetch subresource interception",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub fn start_set_javascript_dialog_handler_enabled(
        &self,
        enabled: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SetJavaScriptDialogHandlerEnabled(
            enabled,
        ))
    }

    pub(crate) async fn replace_browser_resource_runtime_async(
        &mut self,
        resource_runtime: &BrowserResourceRuntime,
    ) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::ReplaceBrowserResourceRuntime {
                resource_runtime: resource_runtime.clone(),
                navigator_identity: resource_runtime.browser_identity().clone(),
            },
            "replace browser resource runtime",
        )
        .await
    }

    pub fn start_replace_browser_resource_runtime(
        &self,
        resource_runtime: &BrowserResourceRuntime,
    ) -> Result<PendingPageCommand> {
        self.start_replace_browser_resource_runtime_with_navigator_identity(
            resource_runtime,
            resource_runtime.browser_identity().clone(),
        )
    }

    pub fn start_replace_browser_resource_runtime_with_navigator_identity(
        &self,
        resource_runtime: &BrowserResourceRuntime,
        navigator_identity: moli_browser_profile::BrowserIdentityProfile,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::ReplaceBrowserResourceRuntime {
            resource_runtime: resource_runtime.clone(),
            navigator_identity,
        })
    }

    pub fn finish_replace_browser_resource_runtime(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "replace browser resource runtime",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub async fn set_extra_http_headers_async(
        &mut self,
        headers: &[(String, String)],
    ) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::SetExtraHttpHeaders(headers.to_vec()),
            "set extra HTTP headers",
        )
        .await
    }

    pub fn start_set_extra_http_headers(
        &self,
        headers: &[(String, String)],
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SetExtraHttpHeaders(headers.to_vec()))
    }

    pub fn finish_set_extra_http_headers(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "set extra HTTP headers",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub async fn set_network_request_policy_async(
        &mut self,
        extra_http_headers: &[(String, String)],
        bypass_service_worker: bool,
        cache_disabled: bool,
        blocked_url_patterns: &[String],
    ) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::SetNetworkRequestPolicy {
                extra_http_headers: extra_http_headers.to_vec(),
                bypass_service_worker,
                cache_disabled,
                blocked_url_patterns: blocked_url_patterns.to_vec(),
            },
            "set network request policy",
        )
        .await
    }

    pub fn start_set_network_request_policy(
        &self,
        extra_http_headers: &[(String, String)],
        bypass_service_worker: bool,
        cache_disabled: bool,
        blocked_url_patterns: &[String],
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SetNetworkRequestPolicy {
            extra_http_headers: extra_http_headers.to_vec(),
            bypass_service_worker,
            cache_disabled,
            blocked_url_patterns: blocked_url_patterns.to_vec(),
        })
    }

    pub fn finish_set_network_request_policy(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "set network request policy",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub async fn set_permission_overrides_async(
        &mut self,
        overrides: &[PermissionOverrideRegistration],
    ) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::SetPermissionOverrides(overrides.to_vec()),
            "set permission overrides",
        )
        .await
    }

    pub fn start_set_permission_overrides(
        &self,
        overrides: &[PermissionOverrideRegistration],
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SetPermissionOverrides(
            overrides.to_vec(),
        ))
    }

    pub fn finish_set_permission_overrides(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "set permission overrides",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub fn start_set_idle_override(
        &mut self,
        idle_override: Option<EmulatedIdleOverride>,
    ) -> Result<PendingPageCommand> {
        let pending =
            self.start_page_command(RendererPageCommand::SetIdleOverride(idle_override))?;
        // SetIdleOverride is synchronous browser-side state in Chromium. Make
        // it visible at command admission so a navigation from another CDP
        // session cannot observe an older protocol snapshot after the renderer
        // has already accepted the command.
        self.idle_override = idle_override;
        Ok(pending)
    }

    pub fn finish_set_idle_override(&mut self, completion: CompletedPageCommand) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "set idle override",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub async fn set_locale_override_async(&mut self, locale: Option<&str>) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::SetLocaleOverride(locale.map(str::to_owned)),
            "set locale override",
        )
        .await
    }

    pub fn start_set_locale_override(&self, locale: Option<&str>) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SetLocaleOverride(
            locale.map(str::to_owned),
        ))
    }

    pub fn finish_set_locale_override(&mut self, completion: CompletedPageCommand) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "set locale override",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub async fn set_timezone_override_async(&mut self, timezone: Option<&str>) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::SetTimezoneOverride(timezone.map(str::to_owned)),
            "set timezone override",
        )
        .await
    }

    pub fn start_set_timezone_override(
        &self,
        timezone: Option<&str>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SetTimezoneOverride(
            timezone.map(str::to_owned),
        ))
    }

    pub fn finish_set_timezone_override(&mut self, completion: CompletedPageCommand) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "set timezone override",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub async fn set_script_execution_disabled_async(&mut self, disabled: bool) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::SetScriptExecutionDisabled(disabled),
            "set script execution disabled",
        )
        .await
    }

    /// Admits the DevTools IO-agent setting through the target's shared IO
    /// task FIFO without entering the renderer owner's Main command queue.
    pub fn start_set_script_execution_disabled_from_io(
        &self,
        disabled: bool,
    ) -> PendingDevToolsIoCommandDispatch {
        let route = self
            .handle
            .enqueue_set_script_execution_disabled_io_command(
                self.renderer_agent_attachment_id,
                self.renderer_devtools_command_session_id.clone(),
                disabled,
            );
        Self::pending_devtools_io_command_dispatch(route)
    }

    /// Publishes the terminal Emulation response through the concrete
    /// renderer DevTools session that owns this Page attachment.
    pub fn start_set_script_execution_disabled_from_io_with_response(
        &self,
        inspector_session_id: Option<String>,
        disabled: bool,
        response: RendererRuntimeInspectorResponseSender,
    ) -> Result<PendingDevToolsIoCommandDispatch> {
        let attachment = self.renderer_agent_attachment_id.ok_or_else(|| {
            anyhow::anyhow!("Emulation IO response requires a renderer attachment")
        })?;
        let route = self
            .handle
            .enqueue_set_script_execution_disabled_io_command_with_response(
                attachment,
                inspector_session_id,
                disabled,
                response,
            );
        Ok(Self::pending_devtools_io_command_dispatch(route))
    }

    pub async fn set_bypass_content_security_policy_async(&mut self, bypass: bool) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::SetBypassContentSecurityPolicy(bypass),
            "set bypass content security policy",
        )
        .await
    }

    pub fn start_set_bypass_content_security_policy(
        &self,
        bypass: bool,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SetBypassContentSecurityPolicy(bypass))
    }

    pub fn finish_set_bypass_content_security_policy(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "set bypass content security policy",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub async fn set_cpu_throttling_rate_async(&mut self, rate: f64) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::SetCpuThrottlingRate(rate),
            "set CPU throttling rate",
        )
        .await
    }

    pub fn start_set_cpu_throttling_rate(&self, rate: f64) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SetCpuThrottlingRate(rate))
    }

    pub fn finish_set_cpu_throttling_rate(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "set CPU throttling rate",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub async fn set_emulated_media_async(
        &mut self,
        overrides: &EmulatedMediaOverrides,
    ) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::SetEmulatedMedia(overrides.clone()),
            "set emulated media",
        )
        .await
    }

    pub fn start_set_emulated_media(
        &self,
        overrides: &EmulatedMediaOverrides,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SetEmulatedMedia(overrides.clone()))
    }

    pub fn finish_set_emulated_media(&mut self, completion: CompletedPageCommand) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "set emulated media",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub async fn set_viewport_surface_async(
        &mut self,
        viewport_surface: Option<ViewportSurface>,
    ) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::SetViewportSurface(viewport_surface),
            "set viewport surface",
        )
        .await
    }

    pub fn start_set_viewport_surface(
        &self,
        viewport_surface: Option<ViewportSurface>,
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SetViewportSurface(viewport_surface))
    }

    pub fn finish_set_viewport_surface(&mut self, completion: CompletedPageCommand) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "set viewport surface",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub(crate) async fn retire_document_resource_authorities_async(&mut self) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::RetireDocumentResourceAuthorities,
            "retire document resource authorities",
        )
        .await
    }

    pub async fn apply_document_cookie_facade_overrides_async(
        &mut self,
        overrides: &moli_cookie_jar::BrowserCookieFacadeOverrides,
    ) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::ApplyDocumentCookieFacadeOverrides(overrides.clone()),
            "apply document cookie facade overrides",
        )
        .await
    }

    pub async fn clear_document_cookie_facade_overrides_async(&mut self) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::ClearDocumentCookieFacadeOverrides,
            "clear document cookie facade overrides",
        )
        .await
    }

    pub async fn document_cookie_telemetry_snapshot_async(
        &mut self,
    ) -> Result<DocumentCookieFacadeTelemetrySnapshot> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::DocumentCookieTelemetrySnapshot)
            .await?;
        match reply {
            RendererPageReply::CookieFacadeSnapshot(snapshot) => match *snapshot {
                RendererPageCookieFacadeSnapshotReply::Telemetry(snapshot) => Ok(snapshot.into()),
                other => Page::unexpected_page_reply(
                    "document cookie telemetry page command",
                    "a cookie facade telemetry snapshot reply",
                    RendererPageReply::CookieFacadeSnapshot(Box::new(other)),
                ),
            },
            other => Page::unexpected_page_reply(
                "document cookie telemetry page command",
                "a cookie facade telemetry snapshot reply",
                other,
            ),
        }
    }

    pub async fn document_cookie_owner_snapshot_async(
        &mut self,
    ) -> Result<DocumentCookieOwnerSnapshot> {
        let pending = self.start_document_cookie_owner_snapshot()?;
        self.finish_document_cookie_owner_snapshot(pending.wait().await?)
    }

    pub fn start_document_cookie_owner_snapshot(&self) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::DocumentCookieOwnerSnapshot)
    }

    pub fn finish_document_cookie_owner_snapshot(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<DocumentCookieOwnerSnapshot> {
        let reply = self.finish_page_command(completion);
        match reply {
            RendererPageReply::CookieFacadeSnapshot(snapshot) => match *snapshot {
                RendererPageCookieFacadeSnapshotReply::Owner(snapshot) => Ok((*snapshot).into()),
                other => Page::unexpected_page_reply(
                    "document cookie owner page command",
                    "a cookie facade owner snapshot reply",
                    RendererPageReply::CookieFacadeSnapshot(Box::new(other)),
                ),
            },
            other => Page::unexpected_page_reply(
                "document cookie owner page command",
                "a cookie facade owner snapshot reply",
                other,
            ),
        }
    }

    pub async fn set_network_offline_async(&mut self, offline: bool) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::SetNetworkOffline(offline),
            "set network offline",
        )
        .await
    }

    pub fn start_set_network_offline(&self, offline: bool) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SetNetworkOffline(offline))
    }

    pub fn finish_set_network_offline(&mut self, completion: CompletedPageCommand) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "set network offline",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub async fn set_bypass_service_worker_async(&mut self, bypass: bool) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::SetBypassServiceWorker(bypass),
            "set bypass service worker",
        )
        .await
    }

    pub fn start_set_bypass_service_worker(&self, bypass: bool) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SetBypassServiceWorker(bypass))
    }

    pub fn finish_set_bypass_service_worker(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "set bypass service worker",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub async fn set_blocked_url_patterns_async(&mut self, patterns: &[String]) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::SetBlockedUrlPatterns(patterns.to_vec()),
            "set blocked URL patterns",
        )
        .await
    }

    pub fn start_set_blocked_url_patterns(
        &self,
        patterns: &[String],
    ) -> Result<PendingPageCommand> {
        self.start_page_command(RendererPageCommand::SetBlockedUrlPatterns(
            patterns.to_vec(),
        ))
    }

    pub fn finish_set_blocked_url_patterns(
        &mut self,
        completion: CompletedPageCommand,
    ) -> Result<()> {
        let reply = self.finish_page_command(completion);
        expect_page_reply!(
            reply,
            "set blocked URL patterns",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }
}
