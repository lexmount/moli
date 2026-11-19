use super::*;

#[cfg(feature = "devtools")]
mod devtools;

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
    pub(in crate::runtime) async fn advance_idle_wait_turn(
        &mut self,
        mut state: PageVmIdleWaitState,
        remaining: std::time::Duration,
    ) -> Result<PageVmIdleWaitAdvance> {
        if self.vm().has_pending_location_navigation() {
            return Ok(PageVmIdleWaitAdvance::TriggeredNavigation);
        }

        let PageVmIdleWaitState::Network {
            quiet_since,
            observed_activity_epoch,
        } = &mut state
        else {
            // Resource exhaustion is observed after a real checkpoint, never
            // inferred from an empty JavaScript stack or a quiet network.
            self.vm_mut().finish_event_loop_idle_checkpoint()?;
            if self.vm().has_pending_location_navigation() {
                return Ok(PageVmIdleWaitAdvance::TriggeredNavigation);
            }
            if !self.vm_mut().has_event_loop_keep_alive_work()?
                && !self.vm().has_pending_child_document_lifecycle()
                && self.page_task_queue.is_empty()
            {
                return Ok(PageVmIdleWaitAdvance::Completed);
            }
            let sleep_for = self
                .vm()
                .ms_to_next_timeout()
                .map(std::time::Duration::from_millis)
                .unwrap_or(remaining)
                .min(remaining);
            return if sleep_for.is_zero() {
                Ok(PageVmIdleWaitAdvance::Progressed { state })
            } else {
                Ok(PageVmIdleWaitAdvance::Waiting { sleep_for, state })
            };
        };

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
            if *observed_activity_epoch != Some(activity_epoch) {
                *observed_activity_epoch = Some(activity_epoch);
                *quiet_since = Some(now);
            }
            let quiet_since = quiet_since.get_or_insert(now);
            let quiet_elapsed = now.saturating_duration_since(*quiet_since);
            if quiet_elapsed >= NETWORK_IDLE_QUIET_WINDOW {
                return Ok(PageVmIdleWaitAdvance::Completed);
            }
            quiet_elapsed
        } else {
            *observed_activity_epoch = Some(activity_epoch);
            *quiet_since = None;
            std::time::Duration::ZERO
        };
        let sleep_for =
            network_idle_sleep_for(ms_to_next, pending_idle_work, quiet_elapsed, remaining);
        if sleep_for.is_zero() {
            Ok(PageVmIdleWaitAdvance::Progressed { state })
        } else {
            Ok(PageVmIdleWaitAdvance::Waiting { sleep_for, state })
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

    #[cfg(feature = "devtools")]
    pub(crate) fn set_fetch_subresource_interception(
        &mut self,
        classes: crate::SubresourceInterceptionClasses,
    ) {
        self.request_client
            .page_network_policy()
            .set_subresource_interception(classes);
    }

    pub(crate) fn set_extra_http_headers(&mut self, headers: &moli_fetch::RequestHeaders) {
        self.extra_http_headers = headers.clone();
        self.vm_mut().set_extra_http_headers(headers);
    }

    pub(crate) fn set_permission_overrides(
        &mut self,
        overrides: &[crate::protocol_types::PermissionOverrideRegistration],
    ) {
        self.permission_overrides = overrides.to_vec();
        self.vm_mut().set_permission_overrides(overrides);
    }

    pub(crate) fn set_network_offline(&mut self, offline: bool) {
        self.network_offline = offline;
        self.vm_mut().set_network_offline(offline);
    }

    pub(crate) fn set_bypass_service_worker(&mut self, bypass: bool) {
        self.vm_mut().set_bypass_service_worker(bypass);
    }

    pub(crate) fn set_cache_disabled(&mut self, disabled: bool) {
        self.vm_mut().set_cache_disabled(disabled);
    }

    pub(crate) fn set_blocked_url_patterns(&mut self, patterns: &[String]) {
        self.blocked_url_patterns = patterns.to_vec();
        self.vm_mut().set_blocked_url_patterns(patterns);
    }

    pub(crate) fn replace_browser_resource_runtime(
        &mut self,
        resource_runtime: &crate::network::BrowserResourceRuntime,
        navigator_identity: &moli_browser_profile::BrowserIdentityProfile,
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
            .replace_document_resource_runtime_with_navigator_identity(
                &page_loader,
                navigator_identity,
            );
        self.request_client = document_loader.request_client().clone();
        self.navigator_identity = navigator_identity.clone();
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
