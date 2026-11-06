use super::subresource::{PreparedSubresourceFetch, SubresourceFetchOptions};
use super::*;
use crate::{
    document_runtime::DomHandle,
    native_bridge::JsContextHost,
    types::{
        AsyncSubresourceFetchCompletion, PendingInputImageFetch, SubresourceRequestInitiatorType,
    },
};
use anyhow::{Context, Result};

/// Image buttons have their own fetch algorithm: no responsive source set,
/// no crossorigin attribute, and no HTML img available-image shortcut.
pub(crate) fn update_input_image(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    element: DomHandle,
    source_was_set: bool,
    request_initiator_type: SubresourceRequestInitiatorType,
) {
    // The bridge and exact Document binding remain authoritative across any
    // author code reached by the existing CSP notification boundary.
    let host = unsafe { &mut *host_ptr };
    let Some(update) = host.begin_input_image_update(scope, element, source_was_set) else {
        return;
    };
    if let Some(previous) = update.previous_network_id {
        host.abort_input_image_fetch(scope, previous);
    }
    let Some(source) = update.source else {
        return;
    };
    if source.as_str_lossy().is_empty() {
        return;
    }
    let owner = update.target.dispatch_scope();
    let Some(resource_loader) = host.document_resource_loader_for_dispatch_scope(owner) else {
        return;
    };
    if !resource_loader
        .request_client()
        .optional_resource_fetch_enabled(crate::types::SubresourceResourceType::Image)
    {
        return;
    }
    let Some(environment) = host.subresource_request_environment(&resource_loader, owner) else {
        return;
    };
    let Ok(request_url) = url::Url::options()
        .base_url(Some(&environment.base_url))
        .parse(source.as_str_lossy())
    else {
        return;
    };
    let result = start_input_image_fetch(
        scope,
        host,
        element,
        update.id,
        update.target,
        resource_loader,
        environment,
        request_url,
        request_initiator_type,
    );
    if let Err(error) = result {
        tracing::debug!(%error, "input image request did not start");
        host.finish_input_image_request(element, update.id, update.target, false);
    }
}

#[allow(clippy::too_many_arguments)]
fn start_input_image_fetch(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    element: DomHandle,
    id: crate::native_bridge::InputImageRequestId,
    target: crate::native_bridge::WindowDocumentTaskTarget,
    resource_loader: crate::network::context::DocumentResourceLoader,
    environment: crate::network::context::SubresourceRequestEnvironment,
    request_url: url::Url,
    request_initiator_type: SubresourceRequestInitiatorType,
) -> Result<()> {
    let owner = target.dispatch_scope();
    let execution_owner = host
        .current_window_execution_context_owner(owner)
        .context("input image Window is no longer available")?;
    let binding = host
        .clone_window_execution_context_binding(scope, execution_owner, owner)
        .filter(|binding| binding.is_current(host))
        .context("input image Document is no longer available")?;
    let policy = host
        .document_connect_policy_snapshot_for_owner(owner)
        .context("input image request policy is unavailable")?
        .for_resource_kind(
            crate::content_security_policy::ContentSecurityPolicyResourceKind::DocumentImage,
        );
    let report_context = capture_window_csp_report_request_context(scope, host, owner)
        .context("input image reporting Document is unavailable")?;
    if host
        .check_element_subresource_csp(
            scope,
            element,
            &request_url,
            crate::document_runtime::DocumentSubresourceCspKind::Image,
        )
        .blocks_request()
    {
        anyhow::bail!("input image blocked by Content Security Policy");
    }
    if !host.input_image_request_is_current(element, id, target) {
        return Ok(());
    }
    let mut timing = crate::context_bootstrap::FetchResourceTiming::for_initiator_type("input");
    timing.capture_window_initiator(scope, host);
    let pending = PendingInputImageFetch {
        element,
        id,
        target,
        timing,
        redirect_state: FetchCspRedirectState::new(&policy),
        policy,
        report_context,
        request_initiator_type,
    };
    let prepared = PreparedSubresourceFetch::prepare(
        scope,
        host,
        owner,
        &resource_loader,
        environment,
        request_url.clone(),
        SubresourceFetchOptions::CSS_IMAGE,
    )?;
    let intercepted =
        host.should_intercept_subresource(crate::types::SubresourceResourceType::Image);
    let internal_id = host
        .record_async_input_image_fetch(
            binding,
            &resource_loader,
            pending,
            prepared.cancel_handle.clone(),
            prepared.network_partition_key.clone(),
            prepared.network_context.policy_context,
            prepared.pending_info.clone(),
            intercepted,
        )
        .context("input image owner changed before request registration")?;
    if intercepted {
        return Ok(());
    }
    let blob = CapturedBlobUrl::capture(&request_url);
    if let Some(response) =
        local_url_response_with_blob_entry(&request_url, "GET", &[], blob.as_ref())
    {
        let result = response
            .map(Into::into)
            .map_err(|error| error.into_message());
        let _ = host.resource_completion_sender().send_async_subresource(
            AsyncSubresourceFetchCompletion {
                internal_id,
                request_url,
                request_method: "GET".into(),
                request_headers: Default::default(),
                request_body: None,
                response_status_text: None,
                skip_fetch_security_validation: false,
                response_filter: None,
                network_error_text: None,
                result: result.into(),
            },
        );
        return Ok(());
    }
    prepared.dispatch(
        host,
        internal_id,
        "service worker input image dispatch failed",
    );
    Ok(())
}
