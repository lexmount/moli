use super::*;

const NETWORK_IDLE_QUIET_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);

fn network_idle_sleep_for(
    ms_to_next: Option<u64>,
    pending_requests: usize,
    quiet_elapsed: std::time::Duration,
    remaining: std::time::Duration,
) -> std::time::Duration {
    let quiet_remaining = if pending_requests == 0 {
        NETWORK_IDLE_QUIET_WINDOW.saturating_sub(quiet_elapsed)
    } else {
        remaining
    };

    ms_to_next
        .map(std::time::Duration::from_millis)
        .unwrap_or(remaining)
        .min(quiet_remaining)
        .min(remaining)
}

impl PageVm {
    pub(in crate::runtime) async fn advance_network_idle_wait_turn(
        &mut self,
        mut state: PageVmNetworkIdleWaitState,
        remaining: std::time::Duration,
    ) -> Result<PageVmNetworkIdleWaitAdvance> {
        if self.vm().has_pending_location_navigation() {
            return Ok(PageVmNetworkIdleWaitAdvance::TriggeredNavigation);
        }

        let pending_requests = self.pending_subresource_request_count();
        let pending_child_frame_lifecycle = self.vm().has_pending_child_document_lifecycle();
        let activity_epoch = self.vm().subresource_activity_epoch();
        let ms_to_next = self.vm().ms_to_next_timeout();

        // Child frame document/script loads are not counted as ordinary
        // subresource requests, but NetworkIdle must not snapshot while an
        // iframe load event or cross-document traversal is still pending. Do
        // not include completed child-frame CDP backlog here; plain fetches do
        // not consume that protocol queue.
        let pending_idle_work = pending_requests + usize::from(pending_child_frame_lifecycle);
        let now = std::time::Instant::now();
        let quiet_elapsed = if pending_idle_work == 0 {
            if state.observed_activity_epoch != Some(activity_epoch) {
                state.observed_activity_epoch = Some(activity_epoch);
                state.quiet_since = Some(now);
            }
            let quiet_since = state.quiet_since.get_or_insert(now);
            let quiet_elapsed = now.saturating_duration_since(*quiet_since);
            if quiet_elapsed >= NETWORK_IDLE_QUIET_WINDOW {
                return Ok(PageVmNetworkIdleWaitAdvance::Completed);
            }
            quiet_elapsed
        } else {
            state.observed_activity_epoch = Some(activity_epoch);
            state.quiet_since = None;
            std::time::Duration::ZERO
        };
        let sleep_for =
            network_idle_sleep_for(ms_to_next, pending_idle_work, quiet_elapsed, remaining);
        if sleep_for.is_zero() {
            Ok(PageVmNetworkIdleWaitAdvance::Progressed { state })
        } else {
            Ok(PageVmNetworkIdleWaitAdvance::Waiting { sleep_for, state })
        }
    }

    pub(in crate::runtime) async fn advance_subresource_response_wait_turn(
        &mut self,
        criteria: &SubresourceResponseWaitCriteria,
        remaining: std::time::Duration,
    ) -> Result<PageVmSubresourceResponseWaitAdvance> {
        if criteria.is_empty() {
            return Err(anyhow!(
                "subresource response wait criteria must not be empty"
            ));
        }

        self.drain_network_output_into_report();
        if self.has_matching_subresource_response(criteria)? {
            return Ok(PageVmSubresourceResponseWaitAdvance::Completed);
        }

        if self.vm().has_pending_location_navigation() {
            return Ok(PageVmSubresourceResponseWaitAdvance::TriggeredNavigation);
        }

        self.drain_network_output_into_report();
        if self.has_matching_subresource_response(criteria)? {
            return Ok(PageVmSubresourceResponseWaitAdvance::Completed);
        }

        let ms_to_next = self.vm().ms_to_next_timeout();

        let sleep_for = ms_to_next
            .map(std::time::Duration::from_millis)
            .unwrap_or(remaining)
            .min(remaining);
        if sleep_for.is_zero() {
            Ok(PageVmSubresourceResponseWaitAdvance::Progressed)
        } else {
            Ok(PageVmSubresourceResponseWaitAdvance::Waiting { sleep_for })
        }
    }

    pub(super) fn drain_network_output_into_report(&mut self) {
        let network_output = self.vm_mut().take_network_output();
        self.report.extend_network_output(network_output);
    }

    fn has_matching_subresource_response(
        &self,
        criteria: &SubresourceResponseWaitCriteria,
    ) -> Result<bool> {
        for record in self.report.subresource_network_records() {
            if criteria.try_matches(record).map_err(|error| {
                anyhow!(
                    "failed to read subresource response body while matching wait criteria: {error}"
                )
            })? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn set_fetch_subresource_interception(
        &mut self,
        enabled: bool,
        resource_type: Option<crate::SubresourceResourceType>,
    ) {
        self.vm_mut()
            .set_fetch_subresource_interception(enabled, resource_type);
    }

    pub(crate) fn set_extra_http_headers(&mut self, headers: &[(String, String)]) {
        self.extra_http_headers = headers.to_vec();
        self.vm_mut().set_extra_http_headers(headers);
    }

    pub(crate) fn set_permission_overrides(
        &mut self,
        overrides: &[crate::protocol_types::PermissionOverrideRegistration],
    ) {
        self.permission_overrides = overrides.to_vec();
        self.vm_mut().set_permission_overrides(overrides);
    }

    pub(crate) fn set_locale_override(&mut self, locale: Option<&str>) -> Result<()> {
        self.vm_mut().set_locale_override_and_sync_surface(locale)
    }

    pub(crate) fn set_timezone_override(&mut self, timezone: Option<&str>) -> Result<()> {
        self.vm_mut()
            .set_timezone_override_and_sync_surface(timezone)
    }

    pub(crate) fn set_script_execution_disabled(&mut self, disabled: bool) {
        self.vm_mut().set_script_execution_disabled(disabled);
    }

    pub(crate) fn set_bypass_content_security_policy(&mut self, bypass: bool) {
        self.bypass_content_security_policy = bypass;
        self.vm_mut().set_bypass_content_security_policy(bypass);
    }

    pub(crate) fn set_cpu_throttling_rate(&mut self, rate: f64) {
        self.cpu_throttling_rate = rate;
    }

    pub(crate) fn set_emulated_media(
        &mut self,
        overrides: &crate::protocol_types::EmulatedMediaOverrides,
    ) {
        self.emulated_media = overrides.clone();
        self.vm_mut().set_emulated_media(overrides);
    }

    pub(crate) fn set_idle_override(
        &mut self,
        idle_override: Option<crate::protocol_types::EmulatedIdleOverride>,
    ) -> anyhow::Result<()> {
        self.vm_mut()
            .set_idle_override_and_sync_surface(idle_override)?;
        self.idle_override = idle_override;
        Ok(())
    }

    pub(crate) fn set_viewport_surface(
        &mut self,
        viewport_surface: Option<crate::protocol_types::ViewportSurface>,
    ) -> anyhow::Result<()> {
        self.vm_mut().set_viewport_surface(viewport_surface)?;
        self.viewport_surface = viewport_surface;
        Ok(())
    }

    pub(crate) fn set_network_offline(&mut self, offline: bool) {
        self.network_offline = offline;
        self.vm_mut().set_network_offline(offline);
    }

    pub(crate) fn set_bypass_service_worker(&mut self, bypass: bool) {
        self.vm_mut().set_bypass_service_worker(bypass);
    }

    pub(crate) fn set_blocked_url_patterns(&mut self, patterns: &[String]) {
        self.blocked_url_patterns = patterns.to_vec();
        self.vm_mut().set_blocked_url_patterns(patterns);
    }

    pub(crate) fn replace_browser_resource_runtime(
        &mut self,
        resource_runtime: &crate::network::BrowserResourceRuntime,
    ) {
        // Replacing a browser/network backend must not replace the live
        // target's Page policy. Pair the new backend with the exact policy
        // already owned by this Page before publishing it to ScriptVm.
        let page_loader =
            ResourceRequestClient::from_browser_resource_runtime_with_page_network_policy(
                resource_runtime.clone(),
                self.request_client.page_network_policy(),
            );
        let document_loader = self
            .vm_mut()
            .replace_document_resource_runtime(&page_loader);
        self.request_client = document_loader.request_client().clone();
    }

    pub(crate) fn retire_document_resource_authorities(&mut self) {
        self.vm_mut().retire_document_resource_authorities();
    }

    pub(crate) fn apply_document_cookie_facade_overrides(
        &mut self,
        overrides: &moli_cookie_jar::BrowserCookieFacadeOverrides,
    ) {
        self.vm_mut()
            .document_runtime
            .apply_document_cookie_facade_overrides(overrides);
    }

    pub(crate) fn clear_document_cookie_facade_overrides(&mut self) {
        self.vm_mut()
            .document_runtime
            .clear_document_cookie_facade_overrides();
    }

    pub(crate) fn document_cookie_telemetry_snapshot(
        &self,
    ) -> crate::DocumentCookieFacadeTelemetrySnapshot {
        self.vm()
            .document_runtime
            .document_cookie_telemetry_snapshot()
    }

    pub(crate) fn document_cookie_owner_snapshot(&self) -> crate::DocumentCookieOwnerSnapshot {
        self.vm().document_runtime.document_cookie_owner_snapshot()
    }

    pub(crate) fn pending_subresource_request_count(&self) -> usize {
        self.vm().pending_subresource_request_count()
    }

    pub(crate) fn continue_pending_subresource_fetch(
        &mut self,
        internal_id: u64,
        url: Option<Url>,
        method: Option<String>,
        body: Option<Option<String>>,
        headers: Option<Vec<(String, String)>>,
        intercept_response: bool,
        handle_auth_requests: bool,
    ) -> Result<crate::PendingSubresourceContinueOutcome> {
        let execution = self.vm_mut().continue_pending_subresource_fetch_body(
            internal_id,
            url,
            method,
            body,
            headers,
            intercept_response,
            handle_auth_requests,
        )?;
        self.finish_async_subresource_command(execution)
    }

    pub(crate) fn continue_pending_subresource_auth(
        &mut self,
        internal_id: u64,
        auth: crate::SubresourceAuthCredentials,
    ) -> Result<crate::PendingSubresourceContinueOutcome> {
        let execution = self
            .vm_mut()
            .continue_pending_subresource_auth_body(internal_id, auth)?;
        self.finish_async_subresource_command(execution)
    }

    pub(crate) fn fail_pending_subresource_auth(
        &mut self,
        internal_id: u64,
        error_text: String,
    ) -> Result<()> {
        let execution = self
            .vm_mut()
            .fail_pending_subresource_auth_body(internal_id, error_text)?;
        self.finish_async_subresource_command(execution)
    }

    pub(crate) fn cancel_pending_subresource_auth(&mut self, internal_id: u64) -> Result<()> {
        let execution = self
            .vm_mut()
            .cancel_pending_subresource_auth_body(internal_id)?;
        self.finish_async_subresource_command(execution)
    }

    pub(crate) fn fail_pending_subresource_fetch(
        &mut self,
        internal_id: u64,
        error_text: String,
    ) -> Result<()> {
        let execution = self
            .vm_mut()
            .fail_pending_subresource_fetch_body(internal_id, error_text)?;
        self.finish_async_subresource_command(execution)
    }

    pub(crate) fn fulfill_pending_subresource_fetch(
        &mut self,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> Result<()> {
        let execution = self.vm_mut().fulfill_pending_subresource_fetch_body(
            internal_id,
            response_code,
            response_headers,
            response_body,
        )?;
        self.finish_async_subresource_command(execution)
    }

    pub(crate) fn continue_pending_subresource_response(
        &mut self,
        internal_id: u64,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    ) -> Result<()> {
        let execution = self.vm_mut().continue_pending_subresource_response_body(
            internal_id,
            response_code,
            response_headers,
        )?;
        self.finish_async_subresource_command(execution)
    }

    pub(crate) fn fail_pending_subresource_response(
        &mut self,
        internal_id: u64,
        error_text: String,
    ) -> Result<()> {
        let execution = self
            .vm_mut()
            .fail_pending_subresource_response_body(internal_id, error_text)?;
        self.finish_async_subresource_command(execution)
    }

    pub(crate) fn fulfill_pending_subresource_response(
        &mut self,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> Result<()> {
        let execution = self.vm_mut().fulfill_pending_subresource_response_body(
            internal_id,
            response_code,
            response_headers,
            response_body,
        )?;
        self.finish_async_subresource_command(execution)
    }

    /// Finish one Fetch-interception command after its body has released all
    /// ScriptVm borrows and V8 scopes.
    ///
    /// A command that synchronously entered a Window realm owns one command-end
    /// checkpoint. Worker, WebSocket, and network-only branches do not borrow
    /// that authority merely because they share the same protocol command.
    fn finish_async_subresource_command<T>(
        &mut self,
        execution: crate::script_vm::AsyncSubresourceCommandExecution<T>,
    ) -> Result<T> {
        let (output, activity, post_checkpoint_event) = execution.into_parts();
        if matches!(
            activity,
            crate::script_vm::AsyncSubresourceFetchBodyActivity::WindowRealmEntered
        ) {
            self.vm_mut()
                .finish_async_subresource_command_checkpoint()?;
        }
        if let Some(event) = post_checkpoint_event {
            self.vm_mut().publish_async_subresource_command_event(event);
        }
        Ok(output)
    }

    pub(crate) fn receive_synthetic_websocket_text(&self, socket_id: u64, data: String) -> bool {
        self.vm().receive_synthetic_websocket_text(socket_id, data)
    }

    pub(crate) fn receive_synthetic_websocket_binary(&self, socket_id: u64, data: Vec<u8>) -> bool {
        self.vm()
            .receive_synthetic_websocket_binary(socket_id, data)
    }

    pub(crate) fn close_synthetic_websocket_from_server(
        &self,
        socket_id: u64,
        code: Option<u16>,
        reason: String,
    ) -> bool {
        self.vm()
            .close_synthetic_websocket_from_server(socket_id, code, reason)
    }
}

#[cfg(test)]
mod tests {
    use super::network_idle_sleep_for;

    #[test]
    fn network_idle_sleep_for_uses_full_quiet_window_when_no_timers_are_pending() {
        let sleep_for = network_idle_sleep_for(
            None,
            0,
            std::time::Duration::from_millis(125),
            std::time::Duration::from_secs(2),
        );

        assert_eq!(sleep_for, std::time::Duration::from_millis(375));
    }

    #[test]
    fn network_idle_sleep_for_respects_next_timeout_before_quiet_window_finishes() {
        let sleep_for = network_idle_sleep_for(
            Some(40),
            0,
            std::time::Duration::from_millis(125),
            std::time::Duration::from_secs(2),
        );

        assert_eq!(sleep_for, std::time::Duration::from_millis(40));
    }

    #[test]
    fn network_idle_sleep_for_uses_deadline_while_requests_are_still_pending() {
        let sleep_for = network_idle_sleep_for(
            None,
            1,
            std::time::Duration::from_millis(125),
            std::time::Duration::from_millis(180),
        );

        assert_eq!(sleep_for, std::time::Duration::from_millis(180));
    }
}
