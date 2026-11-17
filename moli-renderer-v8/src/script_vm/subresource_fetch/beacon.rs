use super::*;

impl ScriptVm {
    pub(super) fn complete_beacon_timing(
        &mut self,
        pending: &mut PendingSubresourceFetchState,
        head: Option<&moli_fetch::ResponseHead>,
        response_filter: Option<&crate::types::AsyncSubresourceFetchResponseFilter>,
        body_size: usize,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        let PendingSubresourceContinuation::Beacon(beacon) = &mut pending.continuation else {
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        };
        let Some(timing) = beacon.timing.take() else {
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        };
        if let Some(native) = pending.load.response_timing() {
            timing.observe_native_timing(native);
        }
        let entry = if let Some(head) = head {
            let filter = response_filter.cloned().unwrap_or_else(|| {
                crate::network_host::FetchResponseRequest {
                    method: &pending.info.method,
                    mode: pending.request_mode,
                    redirect_mode: beacon.request.redirect_mode(),
                }
                .network_response_filter(
                    &pending.request_origin,
                    head,
                    pending.credentials_mode,
                    pending.load.request_client().web_security_disabled(),
                )
            });
            timing.completed_for_request(
                &pending.info.url,
                &pending.request_origin,
                pending.request_mode,
                head,
                body_size,
                Some(&filter),
                pending.load.request_client().web_security_disabled(),
            )
        } else {
            timing.failed(&pending.info.url)
        };
        // document.open keeps the same Window timeline. Navigation retires
        // that Window's timing separately while its transport may continue.
        let Some(identity) = pending.execution_context.window_network_only_identity() else {
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        };
        let entered = self.with_default_context_scope(|scope, host_ptr| {
            Ok(unsafe { &mut *host_ptr }.record_window_resource_timing(scope, identity, entry))
        })?;
        Ok(if entered {
            AsyncSubresourceFetchBodyActivity::WindowRealmEntered
        } else {
            AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered
        })
    }
}
