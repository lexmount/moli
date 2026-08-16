use super::*;
use crate::native_bridge::{
    ImageLoadEventId, JsContextHost, OwnerDispatchScope, PendingImageLoadEventOwner,
};
use crate::service_worker_runtime::{
    ServiceWorkerFetchDispatch, ServiceWorkerRequestDestination,
    service_worker_fetch_request_metadata,
};
use crate::types::{
    AsyncSubresourceFetchCompletion, AsyncSubresourceNetworkContext, ImageRequestCorsMode,
    ImageRequestKey, PendingSubresourceFetchInfo, SubresourceResourceType,
};
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, FetchPriorityHint, RequestCredentialsMode,
    RequestMode, RequestRedirectMode, RequestResourceType,
};

#[derive(Debug)]
pub(crate) enum ImageElementResourceFetchStart {
    Local {
        response: Box<crate::protocol_types::NavigationResponse>,
    },
    Failed,
    PolicySkipped,
    Pending,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScannedImagePreloadStart {
    Admitted,
    Disabled,
    ServiceWorker,
}

pub(crate) fn start_scanned_image_preload(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    request_url: url::Url,
    fetch_priority: Option<FetchPriorityHint>,
) -> ScannedImagePreloadStart {
    if !host.layout_policy().uses_real_layout() {
        return ScannedImagePreloadStart::Disabled;
    }
    let Some(resource_loader) = host.current_main_document_resource_loader() else {
        return ScannedImagePreloadStart::Disabled;
    };
    if !resource_loader
        .request_client()
        .optional_resource_fetch_enabled(SubresourceResourceType::Image)
    {
        return ScannedImagePreloadStart::Disabled;
    }
    let document_url = host.document_url().clone();
    let client_id = host.service_worker_client_id_for_subresource_owner(OwnerDispatchScope::Top);
    if host
        .service_worker_controller_for_fetch(client_id, &document_url, &request_url)
        .is_some()
    {
        return ScannedImagePreloadStart::ServiceWorker;
    }

    let request_key = ImageRequestKey::with_density(
        request_url.as_str().to_owned(),
        ImageRequestCorsMode::NoCors,
        1.0,
    );
    let load = match host.admit_scanned_image_preload(request_key) {
        crate::native_bridge::ScannedImagePreloadAdmission::Fetch(load) => load,
        crate::native_bridge::ScannedImagePreloadAdmission::Reused => {
            return ScannedImagePreloadStart::Admitted;
        }
        crate::native_bridge::ScannedImagePreloadAdmission::Untracked => {
            return ScannedImagePreloadStart::Disabled;
        }
    };
    let request_headers = merge_subresource_request_headers(host.extra_http_headers(), &[]);
    let policy_context = effective_subresource_policy_context(scope, host, OwnerDispatchScope::Top);
    let request = match Request::new("GET", request_url.as_str(), None, request_headers) {
        Ok(request) => request
            .with_initiator_url(&document_url)
            .with_resource_type(RequestResourceType::Image)
            .with_page_network_policy()
            .with_request_mode(RequestMode::NoCors)
            .with_credentials_mode(RequestCredentialsMode::Include)
            .with_network_partition_key(active_subresource_network_partition_key(
                host,
                OwnerDispatchScope::Top,
            ))
            .with_redirect_mode(RequestRedirectMode::Follow)
            .with_browser_request_metadata(BrowserRequestMetadata::Image)
            .with_fetch_priority_hint(fetch_priority),
        Err(error) => {
            load.finish_network_result(
                resource_loader.task_runner(),
                std::sync::Arc::new(Err(error.to_string())),
                false,
            );
            return ScannedImagePreloadStart::Admitted;
        }
    };
    let loader = resource_loader.request_client().clone();
    let task_runner = resource_loader.task_runner();
    let decode_runner = task_runner.clone();
    let cancel_handle = load.cancel_handle();
    task_runner.spawn(async move {
        let result = match loader
            .fetch_raw_stream_with_cancel_and_network_metadata(request, cancel_handle)
            .await
        {
            Ok(observed) => {
                let (response, request_observation) = observed.into_parts();
                response
                    .into_materialized_raw_response()
                    .await
                    .map(|response| {
                        let (head, body) = response.into_parts();
                        let body_bytes = body
                            .try_into_materialized_bytes()
                            .expect("materialized raw image response must retain exact bytes");
                        crate::protocol_types::NavigationResponse::from_head_and_body(
                            head,
                            String::new(),
                            body_bytes,
                        )
                        .with_network_request_headers(
                            request_observation.map(|observation| observation.into_headers()),
                        )
                    })
                    .map_err(|error| format!("scanned image preload body failed: {error:#}"))
            }
            Err(error) => Err(format!("scanned image preload failed: {error:#}")),
        };
        let response_is_decode_eligible = result.as_ref().is_ok_and(|response| {
            response.redirect_chain.is_empty()
                && validate_fetch_response_security_policy_with_body(
                    &document_url,
                    &response.final_url,
                    &response.headers,
                    response.body_bytes(),
                    RequestMode::NoCors,
                    RequestCredentialsMode::Include,
                    policy_context,
                )
                .is_ok()
        });
        load.finish_network_result(
            decode_runner,
            std::sync::Arc::new(result),
            response_is_decode_eligible,
        );
    });
    ScannedImagePreloadStart::Admitted
}

pub(crate) fn start_image_element_resource_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    image_handle: crate::document_runtime::DomHandle,
    sequence: ImageLoadEventId,
    request_url: url::Url,
) -> Result<ImageElementResourceFetchStart, String> {
    let pending = host
        .pending_image_load_event(image_handle)
        .filter(|pending| pending.id() == sequence)
        .ok_or_else(|| "image lifecycle sequence is no longer pending".to_owned())?;
    if !host.pending_image_load_event_is_current(image_handle, pending) {
        return Err("image lifecycle sequence owner is stale".to_owned());
    }
    let (owner, frame_id, document_url) = match pending.owner() {
        PendingImageLoadEventOwner::Main(binding)
            if host.main_document_task_owner_is_current(binding.owner()) =>
        {
            (OwnerDispatchScope::Top, None, host.document_url().clone())
        }
        PendingImageLoadEventOwner::Child(binding) => {
            let snapshot = host
                .frame_owner_current_child_snapshot(binding.child_handle())
                .ok_or_else(|| "image child request scope is unavailable".to_owned())?;
            (
                OwnerDispatchScope::Child(binding.child_handle()),
                Some(snapshot.frame_id.0),
                snapshot.document_url,
            )
        }
        PendingImageLoadEventOwner::Main(_) => {
            return Err("image lifecycle sequence has no current request owner".to_owned());
        }
    };
    let resource_loader = host.document_resource_loader_for_dispatch_scope(owner);
    let network_enabled = resource_loader.as_ref().is_some_and(|loader| {
        loader
            .request_client()
            .optional_resource_fetch_enabled(SubresourceResourceType::Image)
    });
    if !matches!(request_url.scheme(), "blob" | "data") && !network_enabled {
        return Ok(ImageElementResourceFetchStart::PolicySkipped);
    }
    let element = host
        .dom_host()
        .node(image_handle)
        .and_then(crate::dom::native::Node::as_element)
        .ok_or_else(|| "image resource element is unavailable".to_owned())?;
    let element_kind =
        crate::native_bridge::element::ImageResourceElementKind::for_element(element)
            .ok_or_else(|| "element cannot own an image resource".to_owned())?;
    let request_initiator_type = pending.request_initiator_type();
    let (request_mode, credentials_mode, fetch_priority) = match element_kind {
        crate::native_bridge::element::ImageResourceElementKind::Image => {
            let (request_mode, credentials_mode) = image_cross_origin_request_modes(element);
            (
                request_mode,
                credentials_mode,
                FetchPriorityHint::from_attribute(element.attribute("fetchpriority")),
            )
        }
        crate::native_bridge::element::ImageResourceElementKind::Object => {
            (RequestMode::NoCors, RequestCredentialsMode::Include, None)
        }
    };

    if let Some(response) = local_url_response(&request_url) {
        let response: crate::protocol_types::NavigationResponse = response.into();
        let result = Ok(response);
        host.record_get_subresource_network_result_with_initiator(
            frame_id,
            document_url,
            request_url,
            SubresourceResourceType::Image,
            request_initiator_type,
            &result,
        );
        return result.map(|response| ImageElementResourceFetchStart::Local {
            response: Box::new(response),
        });
    }

    let resource_loader =
        resource_loader.ok_or_else(|| "image resource loader is unavailable".to_owned())?;
    let loader = resource_loader.request_client().clone();

    let in_document_image_priority_boost =
        owner == OwnerDispatchScope::Top && host.claim_main_image_priority_boost(image_handle);
    let network_partition_key = active_subresource_network_partition_key(host, owner);
    let policy_context = effective_subresource_policy_context(scope, host, owner);
    let request_cookie_report = observe_subresource_request_cookie_report(
        &loader,
        &document_url,
        &request_url,
        "GET",
        credentials_mode,
    );
    let request_headers = merge_subresource_request_headers(host.extra_http_headers(), &[]);
    let info = PendingSubresourceFetchInfo {
        internal_id: 0,
        network_request_handle: None,
        frame_id: frame_id.clone(),
        document_url: document_url.clone(),
        url: request_url.clone(),
        websocket_socket_id: None,
        method: "GET".to_owned(),
        request_headers: request_headers.clone(),
        request_body: None,
        request_body_bytes: None,
        resource_type: SubresourceResourceType::Image,
        request_cookie_report: request_cookie_report.clone(),
    };

    if host.should_intercept_subresource(SubresourceResourceType::Image) {
        let Some(_) = host.record_intercepted_image_subresource_fetch(
            v8::Global::new(scope, scope.get_current_context()),
            image_handle,
            sequence,
            owner,
            request_initiator_type,
            credentials_mode,
            request_mode,
            network_partition_key,
            policy_context,
            info,
        ) else {
            return Err("image lifecycle sequence changed before interception binding".to_owned());
        };
        return Ok(ImageElementResourceFetchStart::Pending);
    }

    let request = Request::new("GET", request_url.as_str(), None, request_headers.clone())
        .map_err(|error| error.to_string())?
        .with_initiator_url(&document_url)
        .with_resource_type(RequestResourceType::Image)
        .with_page_network_policy()
        .with_request_mode(request_mode)
        .with_credentials_mode(credentials_mode)
        .with_network_partition_key(network_partition_key.clone())
        .with_redirect_mode(RequestRedirectMode::Follow)
        .with_browser_request_metadata(BrowserRequestMetadata::Image)
        .with_fetch_priority_hint(fetch_priority)
        .with_in_document_image_priority_boost(in_document_image_priority_boost)
        .with_subframe_context(frame_id.is_some());
    let client_id = host.service_worker_client_id_for_subresource_owner(owner);
    let service_worker_controller = matches!(request_url.scheme(), "http" | "https")
        .then(|| host.service_worker_controller_for_fetch(client_id, &document_url, &request_url))
        .flatten();
    let scanned_preload = service_worker_controller
        .is_none()
        .then(|| host.claim_scanned_image_preload_for_element(image_handle))
        .flatten();
    let cancel_handle = scanned_preload
        .as_ref()
        .map_or_else(FetchCancelHandle::new, |load| load.cancel_handle());
    let Some(internal_id) = host.record_async_image_subresource_fetch(
        v8::Global::new(scope, scope.get_current_context()),
        image_handle,
        sequence,
        owner,
        request_initiator_type,
        cancel_handle.clone(),
        credentials_mode,
        request_mode,
        network_partition_key,
        policy_context,
        info,
    ) else {
        return Err("image lifecycle sequence changed before request binding".to_owned());
    };

    if service_worker_controller.is_some() {
        let dispatch = ServiceWorkerFetchDispatch {
            internal_id,
            request: host.service_worker_fetch_request(
                client_id,
                request_url.clone(),
                "GET".to_owned(),
                request_headers.clone(),
                None,
                ServiceWorkerRequestDestination::Image,
                request_mode,
                credentials_mode,
                RequestRedirectMode::Follow,
                request.priority_hints.fetch_priority,
                service_worker_fetch_request_metadata(&request),
            ),
            request_body_text: None,
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report,
            network_context: AsyncSubresourceNetworkContext {
                frame_id,
                document_url,
                resource_type: SubresourceResourceType::Image,
                policy_context,
            },
            completion_tx: host.resource_completion_sender(),
            request_client: loader,
            resource_task_runner: resource_loader.task_runner(),
            cancel_handle,
            direct_completion_tx: None,
        };
        if !host.dispatch_service_worker_fetch(dispatch) {
            let _ = host.resource_completion_sender().send_async_subresource(
                AsyncSubresourceFetchCompletion {
                    internal_id,
                    request_url,
                    request_method: "GET".to_owned(),
                    request_headers: request_headers.clone(),
                    request_body: None,
                    response_status_text: None,
                    skip_fetch_security_validation: false,
                    response_filter: None,
                    network_error_text: None,
                    result: Err("service worker image fetch dispatch failed".to_owned()),
                },
            );
        }
        return Ok(ImageElementResourceFetchStart::Pending);
    }

    if let Some(scanned_preload) = scanned_preload {
        debug_assert_eq!(scanned_preload.request_key().url(), request_url.as_str());
        let completion_tx = host.resource_completion_sender();
        resource_loader.task_runner().spawn(async move {
            let outcome = scanned_preload.wait_outcome().await;
            let _ = completion_tx.send_async_subresource(AsyncSubresourceFetchCompletion {
                internal_id,
                request_url,
                request_method: "GET".to_owned(),
                request_headers,
                request_body: None,
                response_status_text: None,
                skip_fetch_security_validation: false,
                response_filter: None,
                network_error_text: None,
                result: outcome.network_result().as_ref().clone(),
            });
        });
        return Ok(ImageElementResourceFetchStart::Pending);
    }

    spawn_async_subresource_fetch(
        resource_loader.task_runner(),
        host.resource_completion_sender(),
        loader,
        request,
        Some(cancel_handle),
        request_headers,
        internal_id,
        AsyncSubresourceNetworkContext {
            frame_id,
            document_url,
            resource_type: SubresourceResourceType::Image,
            policy_context,
        },
        request_url,
        "GET".to_owned(),
        Vec::new(),
        None,
    );
    Ok(ImageElementResourceFetchStart::Pending)
}

pub(crate) fn image_response_descriptor(
    response: &crate::protocol_types::NavigationResponse,
) -> Option<crate::native_bridge::ImageResponseDescriptor> {
    if !image_response_status_is_successful(response.status) {
        return None;
    }
    let body = response.body_bytes();
    let computed_mime_type = moli_web_mime::computed_response_mime_type(
        &response.headers,
        moli_web_mime::MimeSniffingContext::Image,
        body,
    );
    if !moli_web_mime::is_image_mime_essence(&computed_mime_type) {
        return None;
    }
    if moli_web_mime::is_svg_image_mime_essence(&computed_mime_type) {
        let metadata = moli_image::probe_svg_image(body).ok()?;
        return crate::native_bridge::ImageResponseDescriptor::svg(metadata);
    }
    let metadata = moli_image::probe_raster_image(body).ok()?;
    Some(crate::native_bridge::ImageResponseDescriptor::raster(
        metadata,
    ))
}

pub(crate) fn image_response_status_is_successful(status: u16) -> bool {
    (200..300).contains(&status)
}

fn image_cross_origin_request_modes(
    element: &crate::dom::native::Element,
) -> (RequestMode, RequestCredentialsMode) {
    match element
        .attribute("crossorigin")
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None => (RequestMode::NoCors, RequestCredentialsMode::Include),
        Some("use-credentials") => (RequestMode::Cors, RequestCredentialsMode::Include),
        Some(_) => (RequestMode::Cors, RequestCredentialsMode::SameOrigin),
    }
}
