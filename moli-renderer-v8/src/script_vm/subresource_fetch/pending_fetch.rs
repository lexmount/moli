use super::*;

#[cfg(any(test, feature = "devtools"))]
mod interception;

fn with_subresource_request_metadata(
    request: moli_fetch::Request,
    resource_type: SubresourceResourceType,
) -> moli_fetch::Request {
    use moli_fetch::BrowserRequestMetadata;

    match resource_type {
        SubresourceResourceType::Image => {
            request.with_browser_request_metadata(BrowserRequestMetadata::Image)
        }
        SubresourceResourceType::Font => {
            request.with_browser_request_metadata(BrowserRequestMetadata::Font)
        }
        SubresourceResourceType::Audio => {
            request.with_browser_request_metadata(BrowserRequestMetadata::Audio)
        }
        SubresourceResourceType::Video => {
            request.with_browser_request_metadata(BrowserRequestMetadata::Video)
        }
        SubresourceResourceType::TextTrack => {
            request.with_browser_request_metadata(BrowserRequestMetadata::TextTrack)
        }
        _ => request,
    }
}

pub(super) fn request_for_pending_subresource(
    pending: &PendingSubresourceFetchState,
    request_url: &Url,
    request_method: &str,
    request_headers: moli_fetch::RequestHeaders,
    request_body_bytes: Option<bytes::Bytes>,
) -> moli_fetch::Request {
    let captured_request = pending
        .dispatch
        .as_ref()
        .map(|dispatch| &dispatch.request)
        .or_else(|| {
            pending
                .continuation
                .window_fetch()?
                .intercepted_request
                .as_deref()
        });
    if let Some(request) = captured_request {
        let mut request = request.clone();
        request.url = request_url.clone();
        request.method = request_method.to_owned();
        request.cookie_context.set_request_method(request_method);
        request.cookie_context = request
            .cookie_context
            .recompute_site_context_for_request(request_url);
        request.set_body(
            request_body_bytes,
            pending
                .info
                .request_body
                .as_deref()
                .map(std::sync::Arc::from),
        );
        request.request_headers = request_headers;
        let request = request.with_redirect_headers(pending.redirect_headers.clone());
        return match &pending.continuation {
            PendingSubresourceContinuation::Image {
                request_metadata, ..
            } => request.with_subresource_request_metadata(request_metadata.clone()),
            PendingSubresourceContinuation::ResponsePreload(preload) => {
                crate::network_host::apply_response_preload_request_metadata(request, preload)
                    .with_network_partition_key(pending.network_partition_key.clone())
            }
            _ => request,
        };
    }

    let request = moli_fetch::Request::new_browser(
        request_method,
        request_url.clone(),
        request_body_bytes,
        request_headers,
        pending.request_origin.clone(),
    )
    .with_body_text(
        pending
            .info
            .request_body
            .as_deref()
            .map(std::sync::Arc::from),
    )
    .with_redirect_headers(pending.redirect_headers.clone())
    .with_initiator_url(&pending.info.document_url)
    .with_request_mode(pending.request_mode)
    .with_redirect_mode(
        pending
            .continuation
            .window_fetch()
            .map_or(moli_fetch::RequestRedirectMode::Follow, |fetch| {
                fetch.redirect_mode()
            }),
    )
    .with_use_cors_preflight(pending.continuation.use_cors_preflight())
    .with_credentials_mode(pending.credentials_mode)
    .with_network_partition_key(pending.network_partition_key.clone())
    .with_subframe_context(pending.info.frame_id.is_some());
    let request = match pending.info.resource_type {
        SubresourceResourceType::Script
        | SubresourceResourceType::Stylesheet
        | SubresourceResourceType::Image
        | SubresourceResourceType::Font
        | SubresourceResourceType::Audio
        | SubresourceResourceType::Video
        | SubresourceResourceType::Media
        | SubresourceResourceType::TextTrack
        | SubresourceResourceType::Ping
        | SubresourceResourceType::CspReport => {
            match crate::network::request_resource_type_for_subresource(pending.info.resource_type)
            {
                Some(resource_type) => request.with_resource_type(resource_type),
                None => request,
            }
        }
        SubresourceResourceType::Dictionary => request
            .with_resource_type(moli_fetch::RequestResourceType::Dictionary)
            .with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Dictionary)
            .without_inferred_referrer()
            .with_network_partition_key(pending.network_partition_key.clone()),
        SubresourceResourceType::Fetch => request
            .with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Fetch)
            .with_cache_mode(
                pending
                    .continuation
                    .window_fetch()
                    .map_or(moli_fetch::RequestCacheMode::Default, |fetch| {
                        fetch.cache_mode()
                    }),
            ),
        SubresourceResourceType::Manifest => request
            .with_resource_type(moli_fetch::RequestResourceType::Manifest)
            .with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Manifest),
        SubresourceResourceType::EventSource => request
            .with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::EventSource)
            .with_cache_mode(moli_fetch::RequestCacheMode::NoStore)
            .without_request_timeout(),
        SubresourceResourceType::Xhr => {
            request.with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Xhr)
        }
        SubresourceResourceType::WebSocket | SubresourceResourceType::Document => request,
    };
    let request = match &pending.continuation {
        PendingSubresourceContinuation::Image {
            request_metadata, ..
        } => request.with_subresource_request_metadata(request_metadata.clone()),
        PendingSubresourceContinuation::ResponsePreload(preload) => {
            crate::network_host::apply_response_preload_request_metadata(request, preload)
                .with_network_partition_key(pending.network_partition_key.clone())
        }
        _ => request,
    };
    with_subresource_request_metadata(request, pending.info.resource_type)
}

impl PendingSubresourceFetchState {
    /// Keep protocol display fields and the captured Window request together.
    /// Cookie observations belong to each dispatch/redirect decision.
    pub(super) fn update_request_snapshot(&mut self, request: moli_fetch::Request) {
        self.info.url = request.url.clone();
        self.info.method = request.method.clone();
        self.info.request_headers = request.request_headers.clone();
        self.info.request_body_bytes = request.body.clone();
        self.info.request_body = request.body_text().map(|text| text.into_owned());
        self.redirect_headers = request.redirect_headers().cloned();
        if let PendingSubresourceContinuation::Fetch(fetch) = &mut self.continuation {
            fetch.intercepted_request = Some(Box::new(request));
        }
    }
}

impl ScriptVm {
    #[cfg(test)]
    pub(in crate::script_vm) fn eval_in_isolated_context(
        &mut self,
        execution_context_id: i64,
        source: &str,
    ) -> Result<String> {
        let (context_ptr, sync_child_records): (*const v8::Global<v8::Context>, bool) = {
            let world = self
                .page_isolated_world_contexts
                .context(execution_context_id)
                .ok_or_else(|| {
                    anyhow!("unknown isolated execution context `{execution_context_id}`")
                })?;
            (&world.context as *const _, world.child_handle.is_some())
        };
        self.eval_string_in_context_ptr_runtime_turn(context_ptr, source, sync_child_records)
    }
    pub(in crate::script_vm) fn exec_in_isolated_context(
        &mut self,
        execution_context_id: i64,
        source: &str,
    ) -> Result<()> {
        let (context_ptr, sync_child_records): (*const v8::Global<v8::Context>, bool) = {
            let world = self
                .page_isolated_world_contexts
                .context(execution_context_id)
                .ok_or_else(|| {
                    anyhow!("unknown isolated execution context `{execution_context_id}`")
                })?;
            (&world.context as *const _, world.child_handle.is_some())
        };
        self.exec_in_context_ptr_runtime_turn(
            context_ptr,
            source,
            None,
            0,
            true,
            sync_child_records,
        )
    }
}
