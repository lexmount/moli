use super::*;
use crate::native_bridge::{
    ImageLoadEventId, JsContextHost, OwnerDispatchScope, PendingImageLoadEventOwner,
};
use crate::service_worker_runtime::{
    ServiceWorkerFetchDispatch, ServiceWorkerRequestDestination,
    service_worker_fetch_request_metadata,
};
use crate::types::{
    AsyncSubresourceFetchCompletion, AsyncSubresourceFetchResult, AsyncSubresourceNetworkContext,
    ImageRequestCorsMode, ImageRequestKey, PendingSubresourceFetchInfo, SubresourceResourceType,
};
use moli_fetch::{
    BrowserRequestMetadata, FetchCancelHandle, FetchPriorityHint, RequestCredentialsMode,
    RequestMode, RequestRedirectMode, RequestResourceType,
};

#[derive(Debug)]
pub(crate) enum ImageElementResourceFetchStart {
    Local {
        descriptor: Option<crate::native_bridge::ImageResponseDescriptor>,
        encoded: moli_parkable_image::ParkableImage,
    },
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
    let Some(environment) =
        host.subresource_request_environment(&resource_loader, OwnerDispatchScope::Top)
    else {
        return ScannedImagePreloadStart::Disabled;
    };
    let document_url = environment.document_url;
    let request_origin = environment.request_origin;
    let task_runner = resource_loader.task_runner();
    let parkable_images = resource_loader
        .request_client()
        .parkable_image_manager(&task_runner);
    let client_id = host.service_worker_client_id_for_subresource_owner(OwnerDispatchScope::Top);
    if host
        .service_worker_controller_for_fetch(client_id, &document_url, &request_url)
        .is_some()
    {
        return ScannedImagePreloadStart::ServiceWorker;
    }

    let drag_origin = host.document_security_origin(host.document_handle());
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
            .with_request_origin(request_origin.clone())
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
                task_runner,
                std::sync::Arc::new(Err(error.to_string())),
                None,
                false,
                None,
            );
            return ScannedImagePreloadStart::Admitted;
        }
    };
    let loader = resource_loader.request_client().clone();
    let decode_runner = task_runner.clone();
    let cancel_handle = load.cancel_handle();
    task_runner.spawn(async move {
        let result = match loader
            .fetch_raw_stream_with_cancel_and_network_metadata(request, cancel_handle)
            .await
        {
            Ok(observed) => {
                crate::network_host::collect_image_response_into_parkable(observed, parkable_images)
                    .await
                    .map_err(|error| format!("scanned image preload body failed: {error}"))
            }
            Err(error) => Err(format!("scanned image preload failed: {error:#}")),
        };
        let (result, encoded) = match result {
            Ok((response, encoded)) => (Ok(response), Some(encoded)),
            Err(error) => (Err(error), None),
        };
        let response_is_decode_eligible = result.as_ref().is_ok_and(|response| {
            let Some(encoded) = encoded.as_ref() else {
                return false;
            };
            let Ok(snapshot) = encoded.snapshot() else {
                return false;
            };
            response.redirect_chain.is_empty()
                && validate_fetch_response_security_policy_with_body(
                    &request_origin,
                    &response.head(),
                    &snapshot,
                    RequestMode::NoCors,
                    RequestCredentialsMode::Include,
                    policy_context,
                )
                .is_ok()
        });
        load.finish_network_result(
            decode_runner,
            std::sync::Arc::new(result),
            encoded,
            response_is_decode_eligible,
            Some(drag_origin),
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
    let owner = match pending.owner() {
        PendingImageLoadEventOwner::Main(binding)
            if host.main_document_task_owner_is_current(binding.owner()) =>
        {
            OwnerDispatchScope::Top
        }
        PendingImageLoadEventOwner::Child(binding) => {
            OwnerDispatchScope::Child(binding.child_handle())
        }
        PendingImageLoadEventOwner::Main(_) => {
            return Err("image request owner is stale".to_owned());
        }
    };
    let resource_loader = host.document_resource_loader_for_dispatch_scope(owner);
    let environment = resource_loader
        .as_ref()
        .and_then(|loader| host.subresource_request_environment(loader, owner))
        .ok_or_else(|| "image Document request environment is unavailable".to_owned())?;
    let crate::network::context::SubresourceRequestEnvironment {
        document_url,
        request_origin,
        frame_id,
        ..
    } = environment;
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
        .filter(|element| element.is_image_resource_element())
        .ok_or_else(|| "image element is unavailable".to_owned())?;
    let request_initiator_type = pending.request_initiator_type();
    let (request_mode, credentials_mode) = image_cross_origin_request_modes(element);
    let fetch_priority = FetchPriorityHint::from_attribute(element.attribute("fetchpriority"));

    if let Some(response) = local_url_response(&request_url) {
        let mut response: crate::protocol_types::NavigationResponse = response.into();
        let manager = resource_loader.as_ref().map_or_else(
            moli_parkable_image::ParkableImageManager::default,
            |loader| {
                let runner = loader.task_runner();
                loader.request_client().parkable_image_manager(&runner)
            },
        );
        let encoded = manager.from_frozen_bytes(response.take_body_bytes());
        let descriptor = image_response_descriptor_from_parkable(&response, &encoded);
        let result = Ok(response);
        host.record_get_subresource_network_result_with_body_and_initiator(
            frame_id,
            document_url,
            request_url,
            SubresourceResourceType::Image,
            request_initiator_type,
            &result,
            crate::types::SubresourceResponseBody::from_parkable_image(encoded.clone()),
        );
        return result.map(|_| ImageElementResourceFetchStart::Local {
            descriptor,
            encoded,
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
        &request_origin,
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
        .with_request_origin(request_origin.clone())
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
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report,
            network_context: AsyncSubresourceNetworkContext {
                frame_id,
                request_origin: request_origin.clone(),
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
                    result: AsyncSubresourceFetchResult::Failure(
                        "service worker image fetch dispatch failed".to_owned(),
                    ),
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
                result: AsyncSubresourceFetchResult::from_image_parts(
                    outcome.network_result().as_ref().clone(),
                    outcome.encoded(),
                ),
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
        request_headers.to_byte_strings(),
        internal_id,
        AsyncSubresourceNetworkContext {
            frame_id,
            request_origin: request_origin.clone(),
            document_url,
            resource_type: SubresourceResourceType::Image,
            policy_context,
        },
        request_url,
        "GET".to_owned(),
        Default::default(),
        None,
    );
    Ok(ImageElementResourceFetchStart::Pending)
}

pub(crate) fn image_response_descriptor_from_parkable(
    response: &crate::protocol_types::NavigationResponse,
    encoded: &moli_parkable_image::ParkableImage,
) -> Option<crate::native_bridge::ImageResponseDescriptor> {
    let snapshot = encoded.snapshot().ok()?;
    image_response_descriptor_from_bytes(&response.headers, &snapshot)
}

fn image_response_descriptor_from_bytes(
    headers: &[(String, Vec<u8>)],
    body: &[u8],
) -> Option<crate::native_bridge::ImageResponseDescriptor> {
    // HTTP error responses can still contain usable image data.
    let computed_mime_type = moli_web_mime::computed_response_mime_type(
        headers,
        moli_web_mime::MimeSniffingContext::Image,
        body,
    );
    if moli_web_mime::is_svg_image_mime_essence(&computed_mime_type) {
        let metadata = moli_image::probe_svg_image(body).ok()?;
        return crate::native_bridge::ImageResponseDescriptor::svg(metadata);
    }
    let metadata = moli_image::probe_raster_image(body).ok()?;
    Some(crate::native_bridge::ImageResponseDescriptor::raster(
        metadata,
    ))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_response_decodes_raster_and_svg_at_non_success_http_statuses() {
        let pixels = moli_image::RgbaImage::try_new(2, 1, vec![255; 8]).unwrap();
        let png = moli_image::encode_png(&pixels).unwrap().bytes;
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1"/>"#;
        let manager = moli_parkable_image::ParkableImageManager::default();
        for (mime, bytes) in [
            ("image/png", png.as_slice()),
            ("image/svg+xml", svg.as_slice()),
            ("application/xml", png.as_slice()),
            ("text/xml", png.as_slice()),
            ("application/blah+xml", png.as_slice()),
        ] {
            let encoded = manager.from_frozen_bytes(bytes.to_vec());
            for nosniff in [false, true] {
                let mut headers = vec![("Content-Type".to_owned(), mime.as_bytes().to_vec())];
                if nosniff {
                    headers.push(("X-Content-Type-Options".to_owned(), b"nosniff".to_vec()));
                }
                for status in [200, 206, 301, 302, 400, 404, 410, 500, 503] {
                    let response = crate::protocol_types::NavigationResponse::from_text_body(
                        url::Url::parse("https://images.test/image").unwrap(),
                        status,
                        headers.clone(),
                        String::new(),
                    );
                    let descriptor = image_response_descriptor_from_parkable(&response, &encoded)
                        .unwrap_or_else(|| {
                            panic!("{mime} image rejected at HTTP {status}, nosniff={nosniff}")
                        });
                    assert_eq!((descriptor.width, descriptor.height), (2, 1));
                }
            }
        }
    }

    #[test]
    fn image_response_rejects_empty_and_invalid_data_at_any_http_status() {
        let manager = moli_parkable_image::ParkableImageManager::default();
        let png =
            moli_image::encode_png(&moli_image::RgbaImage::try_new(2, 1, vec![255; 8]).unwrap())
                .unwrap()
                .bytes;
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1"/>"#;
        for (mime, bytes) in [
            ("image/png", b"".as_slice()),
            ("image/png", b"\x89PNG\r\n\x1a\n".as_slice()),
            ("image/png", b"not an image".as_slice()),
            ("image/svg+xml", b"<html>Not Found</html>".as_slice()),
            ("text/html", b"<html>Not Found</html>".as_slice()),
            ("application/xml", b"\x89PNG\r\n\x1a\n".as_slice()),
            (
                "application/xml",
                b"<?xml version='1.0'?><not-an-image/>".as_slice(),
            ),
            ("application/blah+xml", b"<html>Not Found</html>".as_slice()),
            ("image/svg+xml", png.as_slice()),
            ("application/xml", svg.as_slice()),
            ("application/blah+xml", svg.as_slice()),
        ] {
            let encoded = manager.from_frozen_bytes(bytes.to_vec());
            for status in [200, 404, 500] {
                let response = crate::protocol_types::NavigationResponse::from_text_body(
                    url::Url::parse("https://images.test/image").unwrap(),
                    status,
                    vec![("Content-Type".to_owned(), mime.as_bytes().to_vec())],
                    String::new(),
                );
                assert!(
                    image_response_descriptor_from_parkable(&response, &encoded).is_none(),
                    "invalid {mime} image accepted at HTTP {status}"
                );
            }
        }
    }
}
