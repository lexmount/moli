use super::*;
use crate::util::context_host_ptr_from_global_bridge;

pub(super) fn previous_document_unload_times<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<(f64, f64)> {
    let context = object.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let host = unsafe { &*host_ptr };
    let dispatch = host.window_dispatch_scope_for_context(scope, context)?;
    host.current_runtime_window_execution_context_identity_for_dispatch_scope(scope, dispatch)?;
    let loader = host.document_resource_loader_for_dispatch_scope(dispatch)?;
    if let Some(timing) = loader.navigation_timing() {
        let snapshot = timing.snapshot()?;
        let origin = moli_url::WebOrigin::from_url(&snapshot.head.final_url);
        if snapshot.head.url_list().has_cross_origin_url(&origin) {
            return None;
        }
    }
    let origin = host.window_message_source_security(dispatch)?;
    loader
        .previous_document_unload_timing()?
        .for_origin(&origin)
}

pub(super) fn refresh_navigation_unload_timing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) {
    let Some((start, end)) = previous_document_unload_times(scope, entry) else {
        return;
    };
    let Some(context) = entry.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(time_origin) = super::window_state::current_window_performance_time_origin_seed(scope)
    else {
        return;
    };
    for (slot, absolute) in [
        (install::NAV_UNLOAD_EVENT_START_SLOT, start),
        (install::NAV_UNLOAD_EVENT_END_SLOT, end),
    ] {
        set_performance_entry_slot_number(
            scope,
            entry,
            slot,
            moli_time::relative_dom_time_millis(absolute, time_origin),
        );
    }
}

/// Refresh the entry in its own Window realm. Borrowed prototype getters must
/// not pick up the caller's navigation or a replacement Window's Document.
pub(super) fn refresh_navigation_timing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) {
    if performance_entry_slot_string(scope, entry, PERFORMANCE_ENTRY_TYPE_SLOT).as_deref()
        != Some("navigation")
    {
        return;
    }
    refresh_navigation_unload_timing(scope, entry);
    let Some(context) = entry.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &*host_ptr };
    let Some(dispatch) = host.window_dispatch_scope_for_context(scope, context) else {
        return;
    };
    if host
        .current_runtime_window_execution_context_identity_for_dispatch_scope(scope, dispatch)
        .is_none()
    {
        return;
    }
    let Some(snapshot) = host
        .document_resource_loader_for_dispatch_scope(dispatch)
        .and_then(|loader| loader.navigation_timing())
        .and_then(|timing| timing.snapshot())
    else {
        return;
    };
    let origin = moli_url::WebOrigin::from_url(&snapshot.head.final_url);
    let expose_redirects = !snapshot.head.url_list().has_cross_origin_url(&origin);
    // A navigation exposes the final Document's response. Cross-origin hops
    // hide redirect timing without hiding that final response's own timing.
    let mut final_head = snapshot.head.clone();
    final_head.redirect_chain.clear();
    let mut timing =
        FetchResourceTiming::from_start("navigation", snapshot.started, snapshot.time_origin);
    timing.response_started_at(Some(snapshot.headers_received));
    timing.observe_native_timing(snapshot.native);
    let mut data = timing.response(
        &snapshot.head.final_url,
        &origin,
        &final_head,
        &moli_fetch::FetchResponseFilter::Basic,
        snapshot.materialized_body_size.unwrap_or(0),
    );
    let epoch = |instant: std::time::Instant| {
        snapshot.time_origin
            + instant
                .saturating_duration_since(snapshot.started)
                .as_secs_f64()
                * 1000.0
    };
    let worker_start = snapshot.native.service_worker_started_at().map(epoch);
    let worker_ready = snapshot.native.service_worker_dispatched_at().map(epoch);
    let synthetic = worker_start.is_some() && snapshot.native.response_started_at().is_none();
    let fetch_start = if snapshot.head.redirected {
        snapshot
            .native
            .fetch_started_at()
            .map(epoch)
            .unwrap_or_else(|| epoch(snapshot.fetch_started))
    } else {
        worker_ready
            .or_else(|| snapshot.native.fetch_started_at().map(epoch))
            .unwrap_or_else(|| epoch(snapshot.fetch_started))
    };
    if expose_redirects && snapshot.head.redirected {
        data.redirect_start_unix_millis = Some(snapshot.time_origin);
        data.redirect_end_unix_millis = Some(fetch_start);
    }
    if let Some(connection) = &mut data.connection_timing {
        connection.domain_lookup_start = connection.domain_lookup_start.max(fetch_start);
        connection.domain_lookup_end = connection.domain_lookup_end.max(fetch_start);
        connection.connect_start = connection.connect_start.max(fetch_start);
        connection.connect_end = connection.connect_end.max(fetch_start);
        connection.secure_connection_start = connection
            .secure_connection_start
            .map(|start| start.max(fetch_start));
    }
    data.request_start_unix_millis = data.request_start_unix_millis.or(Some(fetch_start));
    let response_end = snapshot.native.response_end_at().map(epoch).or_else(|| {
        snapshot
            .materialized_body_size
            .map(|_| epoch(snapshot.headers_received))
    });
    if synthetic {
        data.connection_timing = Some(ResourceConnectionTiming::at(fetch_start, false));
        data.request_start_unix_millis = Some(fetch_start);
        data.response_start_unix_millis = Some(epoch(snapshot.headers_received));
        data.final_response_headers_start_unix_millis = data.response_start_unix_millis;
        data.transfer_size = 0.0;
    }
    // A streaming document's terminal fields stay zero until the selected
    // transfer completes. Cached entry references see the terminal observation.
    if response_end.is_none() {
        data.transfer_size = 0.0;
        data.encoded_body_size = 0.0;
        data.decoded_body_size = 0.0;
    }
    initialize_resource_timing_slots(
        scope,
        entry,
        "navigation",
        data.transfer_size,
        data.encoded_body_size,
        data.decoded_body_size,
        "non-blocking",
        data.response_status,
        &data.content_type,
    );
    entries::initialize_resource_timing_response_metadata(
        scope,
        entry,
        &data.delivery_type,
        &data.content_encoding,
        "",
    );
    for (slot, value) in [
        (PERFORMANCE_RESOURCE_WORKER_START_SLOT, worker_start),
        (PERFORMANCE_RESOURCE_FETCH_START_SLOT, Some(fetch_start)),
        (
            PERFORMANCE_RESOURCE_REDIRECT_START_SLOT,
            data.redirect_start_unix_millis,
        ),
        (
            PERFORMANCE_RESOURCE_REDIRECT_END_SLOT,
            data.redirect_end_unix_millis,
        ),
        (
            PERFORMANCE_RESOURCE_DOMAIN_LOOKUP_START_SLOT,
            data.connection_timing.map(|t| t.domain_lookup_start),
        ),
        (
            PERFORMANCE_RESOURCE_DOMAIN_LOOKUP_END_SLOT,
            data.connection_timing.map(|t| t.domain_lookup_end),
        ),
        (
            PERFORMANCE_RESOURCE_CONNECT_START_SLOT,
            data.connection_timing.map(|t| t.connect_start),
        ),
        (
            PERFORMANCE_RESOURCE_CONNECT_END_SLOT,
            data.connection_timing.map(|t| t.connect_end),
        ),
        (
            PERFORMANCE_RESOURCE_SECURE_CONNECTION_START_SLOT,
            data.connection_timing
                .and_then(|t| t.secure_connection_start),
        ),
        (
            PERFORMANCE_RESOURCE_REQUEST_START_SLOT,
            data.request_start_unix_millis,
        ),
        (
            PERFORMANCE_RESOURCE_RESPONSE_START_SLOT,
            data.response_start_unix_millis,
        ),
        (
            PERFORMANCE_RESOURCE_FINAL_RESPONSE_HEADERS_START_SLOT,
            data.final_response_headers_start_unix_millis,
        ),
        (
            PERFORMANCE_RESOURCE_FIRST_INTERIM_RESPONSE_START_SLOT,
            data.first_interim_response_start_unix_millis,
        ),
        (PERFORMANCE_RESOURCE_RESPONSE_END_SLOT, response_end),
    ] {
        let value = value
            .map(|value| moli_time::relative_dom_time_millis(value, snapshot.time_origin))
            .unwrap_or(0.0);
        set_performance_entry_slot_number(scope, entry, slot, value);
    }
    if let Some(protocol) = v8_string(scope, &data.next_hop_protocol) {
        set_private_value(
            scope,
            entry,
            PERFORMANCE_RESOURCE_NEXT_HOP_PROTOCOL_SLOT,
            protocol.into(),
        );
    }
}
