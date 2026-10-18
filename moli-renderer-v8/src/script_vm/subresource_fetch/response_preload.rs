use super::*;

impl ScriptVm {
    pub(super) fn complete_response_preload(
        &mut self,
        pending: &mut PendingSubresourceFetchState,
        response_filter: Option<&AsyncSubresourceFetchResponseFilter>,
        from_service_worker: bool,
        result: &std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        let PendingSubresourceContinuation::ResponsePreload(preload) = &mut pending.continuation
        else {
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        };
        if let Some(native) = pending.load.response_timing() {
            preload.timing.observe_native_timing(native);
        }
        let (entry, response) = match result {
            Ok(response) => {
                let filter = response_filter.cloned().unwrap_or_else(|| {
                    crate::network_host::FetchResponseRequest {
                        method: "GET",
                        mode: pending.request_mode,
                        redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                    }
                    .network_response_filter(
                        &pending.request_origin,
                        &response.head(),
                        pending.credentials_mode,
                    )
                });
                let entry = if pending.request_mode == moli_fetch::RequestMode::NoCors {
                    preload.timing.classic_script_response(
                        &pending.info.url,
                        &pending.request_origin,
                        &response.head(),
                        response.body_bytes().len(),
                    )
                } else {
                    preload.timing.response(
                        &pending.info.url,
                        &pending.request_origin,
                        &response.head(),
                        &filter,
                        response.body_bytes().len(),
                    )
                };
                let response =
                    if crate::subresource_integrity::response_matches_subresource_integrity_metadata(
                        response.body_bytes(),
                        preload.metadata.integrity.as_deref(),
                        filter.is_readable(),
                    ) {
                        Ok(crate::network::preloads::DocumentPreloadResponse {
                            response: response.clone(),
                            response_filter: Some(filter),
                            from_service_worker,
                            service_worker_response_url: from_service_worker
                                .then(|| response.final_url.clone()),
                        })
                    } else {
                        Err("script preload failed its integrity check".to_owned())
                    };
                (entry, response)
            }
            Err(error) => (
                preload.timing.failure(&pending.info.url),
                Err(error.clone()),
            ),
        };
        if let Some(producer) = preload.producer.take() {
            producer.complete(response);
        }
        let Some(target) = pending
            .execution_context
            .window_document_network_only_identity()
        else {
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        };
        let entered = self.with_default_context_scope(|scope, host_ptr| {
            Ok(unsafe { &mut *host_ptr }.record_document_resource_timing(scope, target, entry))
        })?;
        Ok(if entered {
            AsyncSubresourceFetchBodyActivity::WindowRealmEntered
        } else {
            AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered
        })
    }
}
