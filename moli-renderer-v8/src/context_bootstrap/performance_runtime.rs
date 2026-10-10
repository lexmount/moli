use super::*;
use crate::util::{get_private_value, set_private_value};

mod entries;
mod install;
mod lazy_subobjects;
mod marks_measures;
mod memory;
mod resource_buffer;
mod window_state;

const DEFAULT_RESOURCE_TIMING_BUFFER_SIZE: u32 = 250;
const PERFORMANCE_EVENT_LISTENERS_SLOT: &str = "__moliPerformanceEventListeners";
const PERFORMANCE_RESOURCE_TIMING_BUFFER_ID_SLOT: &str = "__moliPerformanceResourceTimingBufferId";
const PERFORMANCE_ON_RESOURCE_TIMING_BUFFER_FULL_SLOT: &str =
    "__moliPerformanceOnResourceTimingBufferFull";

pub(crate) const PERFORMANCE_TIME_ORIGIN_SLOT: &str = "__moliPerformanceTimeOrigin";
pub(in crate::context_bootstrap) const PERFORMANCE_TIMING_SLOT: &str = "__moliPerformanceTiming";
pub(in crate::context_bootstrap) const PERFORMANCE_NAVIGATION_SLOT: &str =
    "__moliPerformanceNavigation";
pub(in crate::context_bootstrap) const PERFORMANCE_NAVIGATION_TYPE_SLOT: &str =
    "__moliPerformanceNavigationType";
pub(in crate::context_bootstrap) const PERFORMANCE_NAVIGATION_REDIRECT_COUNT_SLOT: &str =
    "__moliPerformanceNavigationRedirectCount";
pub(in crate::context_bootstrap) const PERFORMANCE_ENTRY_NAME_SLOT: &str =
    "__moliPerformanceEntryName";
pub(in crate::context_bootstrap) const PERFORMANCE_ENTRY_TYPE_SLOT: &str =
    "__moliPerformanceEntryType";
pub(in crate::context_bootstrap) const PERFORMANCE_ENTRY_START_TIME_SLOT: &str =
    "__moliPerformanceEntryStartTime";
pub(in crate::context_bootstrap) const PERFORMANCE_ENTRY_DURATION_SLOT: &str =
    "__moliPerformanceEntryDuration";
pub(in crate::context_bootstrap) const PERFORMANCE_ENTRY_DETAIL_SLOT: &str =
    "__moliPerformanceEntryDetail";
pub(in crate::context_bootstrap) const PERFORMANCE_RESOURCE_INITIATOR_TYPE_SLOT: &str =
    "__moliPerformanceResourceInitiatorType";
pub(in crate::context_bootstrap) const PERFORMANCE_RESOURCE_NEXT_HOP_PROTOCOL_SLOT: &str =
    "__moliPerformanceResourceNextHopProtocol";
pub(in crate::context_bootstrap) const PERFORMANCE_RESOURCE_WORKER_START_SLOT: &str =
    "__moliPerformanceResourceWorkerStart";
pub(in crate::context_bootstrap) const PERFORMANCE_RESOURCE_REDIRECT_START_SLOT: &str =
    "__moliPerformanceResourceRedirectStart";
pub(in crate::context_bootstrap) const PERFORMANCE_RESOURCE_REDIRECT_END_SLOT: &str =
    "__moliPerformanceResourceRedirectEnd";
pub(in crate::context_bootstrap) const PERFORMANCE_RESOURCE_FETCH_START_SLOT: &str =
    "__moliPerformanceResourceFetchStart";
pub(in crate::context_bootstrap) const PERFORMANCE_RESOURCE_DOMAIN_LOOKUP_START_SLOT: &str =
    "__moliPerformanceResourceDomainLookupStart";
pub(in crate::context_bootstrap) const PERFORMANCE_RESOURCE_DOMAIN_LOOKUP_END_SLOT: &str =
    "__moliPerformanceResourceDomainLookupEnd";
pub(in crate::context_bootstrap) const PERFORMANCE_RESOURCE_CONNECT_START_SLOT: &str =
    "__moliPerformanceResourceConnectStart";
pub(in crate::context_bootstrap) const PERFORMANCE_RESOURCE_CONNECT_END_SLOT: &str =
    "__moliPerformanceResourceConnectEnd";
pub(in crate::context_bootstrap) const PERFORMANCE_RESOURCE_SECURE_CONNECTION_START_SLOT: &str =
    "__moliPerformanceResourceSecureConnectionStart";
pub(in crate::context_bootstrap) const PERFORMANCE_RESOURCE_REQUEST_START_SLOT: &str =
    "__moliPerformanceResourceRequestStart";
pub(in crate::context_bootstrap) const PERFORMANCE_RESOURCE_RESPONSE_START_SLOT: &str =
    "__moliPerformanceResourceResponseStart";
pub(in crate::context_bootstrap) const PERFORMANCE_RESOURCE_RESPONSE_END_SLOT: &str =
    "__moliPerformanceResourceResponseEnd";
pub(in crate::context_bootstrap) const PERFORMANCE_RESOURCE_TRANSFER_SIZE_SLOT: &str =
    "__moliPerformanceResourceTransferSize";
pub(in crate::context_bootstrap) const PERFORMANCE_RESOURCE_ENCODED_BODY_SIZE_SLOT: &str =
    "__moliPerformanceResourceEncodedBodySize";
pub(in crate::context_bootstrap) const PERFORMANCE_RESOURCE_DECODED_BODY_SIZE_SLOT: &str =
    "__moliPerformanceResourceDecodedBodySize";
pub(in crate::context_bootstrap) const PERFORMANCE_RESOURCE_RENDER_BLOCKING_STATUS_SLOT: &str =
    "__moliPerformanceResourceRenderBlockingStatus";
pub(in crate::context_bootstrap) const PERFORMANCE_RESOURCE_RESPONSE_STATUS_SLOT: &str =
    "__moliPerformanceResourceResponseStatus";
pub(in crate::context_bootstrap) const PERFORMANCE_RESOURCE_CONTENT_TYPE_SLOT: &str =
    "__moliPerformanceResourceContentType";
pub(in crate::context_bootstrap) const PERFORMANCE_EVENT_COUNTS_SLOT: &str =
    "__moliPerformanceEventCounts";
pub(in crate::context_bootstrap) const PERFORMANCE_EVENT_COUNTS_VALUES_SLOT: &str =
    "__moliPerformanceEventCountsValues";
pub(super) const PERFORMANCE_NAVIGATION_ENTRY_SLOT: &str = "__moliPerformanceNavigationEntry";
pub(super) const PERFORMANCE_NAVIGATION_TYPE_SEED_SLOT: &str =
    "__moliPerformanceNavigationTypeSeed";
pub(super) const PERFORMANCE_NAVIGATION_NAME_SEED_SLOT: &str =
    "__moliPerformanceNavigationNameSeed";
pub(super) const PERFORMANCE_LIFECYCLE_TIMESTAMPS_SLOT: &str =
    "__moliPerformanceLifecycleTimestamps";
pub(super) const PERFORMANCE_PENDING_EVENT_COUNTS_SLOT: &str =
    "__moliPerformancePendingEventCounts";
pub(in crate::context_bootstrap) const PERFORMANCE_OBSERVER_SUPPORTED_ENTRY_TYPES: &[&str] =
    &["mark", "measure", "navigation", "resource"];

pub(super) use super::performance_observer_runtime::{
    performance_entry_list_get_entries_by_name_callback,
    performance_entry_list_get_entries_by_type_callback,
    performance_entry_list_get_entries_callback, performance_observer_constructor_callback,
    performance_observer_disconnect_callback, performance_observer_observe_callback,
    performance_observer_take_records_callback,
};
use entries::{
    append_performance_entry, create_performance_entry, initialize_resource_timing_slots,
    push_performance_entry,
};
pub(super) use entries::{
    install_performance_entry_template_bindings, performance_get_entries_by_name_callback,
    performance_get_entries_by_type_callback, performance_get_entries_callback,
};
pub(in crate::context_bootstrap) use entries::{
    performance_entry_slot_number, performance_entry_slot_string, performance_entry_slot_value,
    set_performance_entry_slot_number,
};
pub(crate) use install::finalize_performance_observer_realm_bindings;
pub(crate) use install::increment_performance_event_count;
pub(super) use install::install_performance_template_bindings;
pub(crate) use install::{
    record_performance_dom_content_loaded_event_end,
    record_performance_dom_content_loaded_event_start, record_performance_load_event_end,
    record_performance_load_event_start,
};
pub(super) use marks_measures::{
    performance_clear_marks_callback, performance_clear_measures_callback,
    performance_mark_callback, performance_mark_constructor_callback, performance_measure_callback,
};
pub(crate) use resource_buffer::run_resource_timing_buffer_full_task;
pub(crate) use window_state::bind_window_performance_seed;
pub(super) use window_state::{
    build_window_performance_for_receiver, ensure_current_window_performance,
    finish_window_performance_materialization, install_default_window_performance_seed,
};

pub(super) fn ensure_current_performance_for_api<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(performance) = window_performance_value(scope) {
        return Some(performance);
    }
    if window_state::current_window_performance_time_origin_seed(scope).is_some() {
        return ensure_current_window_performance(scope).ok();
    }
    let global = scope.get_current_context().global(scope);
    global
        .get(scope, v8str(scope, "performance").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(in crate::context_bootstrap) struct WindowNavigationTimingSnapshot {
    pub(in crate::context_bootstrap) time_origin_millis: f64,
    pub(in crate::context_bootstrap) dom_content_loaded_end_millis: Option<f64>,
    pub(in crate::context_bootstrap) load_event_end_millis: Option<f64>,
    pub(in crate::context_bootstrap) navigation_type: String,
}

pub(in crate::context_bootstrap) fn current_window_navigation_timing_snapshot(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<WindowNavigationTimingSnapshot> {
    let performance = ensure_current_performance_for_api(scope)?;
    Some(WindowNavigationTimingSnapshot {
        time_origin_millis: performance_slot_number(
            scope,
            performance,
            PERFORMANCE_TIME_ORIGIN_SLOT,
        )?,
        dom_content_loaded_end_millis: install::performance_lifecycle_timestamp(
            scope,
            performance,
            install::DOM_CONTENT_LOADED_END_INDEX,
        ),
        load_event_end_millis: install::performance_lifecycle_timestamp(
            scope,
            performance,
            install::LOAD_END_INDEX,
        ),
        navigation_type: install::performance_navigation_type_seed(scope, performance),
    })
}

pub(in crate::context_bootstrap) fn ensure_navigation_performance_entry_for_api<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    install::ensure_navigation_performance_entry(scope, performance)
}

pub(crate) fn current_performance_time_origin(scope: &mut v8::PinScope<'_, '_>) -> f64 {
    if let Some(time_origin) = window_state::current_window_performance_time_origin_seed(scope) {
        return time_origin;
    }
    let global = scope.get_current_context().global(scope);
    global
        .get(scope, v8str(scope, "performance").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|performance| {
            performance_slot_number(scope, performance, PERFORMANCE_TIME_ORIGIN_SLOT)
        })
        .unwrap_or(0.0)
}

fn resource_timing_content_type(headers: &[(String, Vec<u8>)]) -> String {
    let Some(essence) = moli_web_mime::extract_response_mime_essence(headers) else {
        return String::new();
    };
    // Fetch stores a minimized MIME type in response body info. Classify the
    // extracted MIME record, not the first header or a sniffed body type.
    // https://mimesniff.spec.whatwg.org/#minimize-a-supported-mime-type
    if essence == "image/svg+xml" {
        // SVG retains its own type instead of joining the generic XML group.
        return essence;
    }
    if moli_web_mime::is_javascript_mime_essence(&essence) {
        "text/javascript".to_owned()
    } else if matches!(essence.as_str(), "application/json" | "text/json")
        || essence.ends_with("+json")
    {
        "application/json".to_owned()
    } else if matches!(essence.as_str(), "application/xml" | "text/xml")
        || essence.ends_with("+xml")
    {
        "application/xml".to_owned()
    } else if moli_web_mime::is_text_mime_essence(&essence)
        || moli_image::supports_image_mime_essence(&essence)
        || moli_web_mime::media_mime_support(&essence)
            != moli_web_mime::MediaMimeSupport::Unsupported
        || moli_web_mime::is_supported_font_mime_essence(&essence)
        || essence == "application/wasm"
    {
        // Text documents, decodable images, media, fonts and WebAssembly use
        // the same support decisions as their consuming APIs. An arbitrary
        // image/*, audio/*, video/* or font/* prefix does not imply support.
        essence
    } else {
        String::new()
    }
}

pub(crate) struct ResourcePerformanceEntry {
    name: String,
    initiator_type: String,
    start_unix_millis: Option<f64>,
    transfer_size: f64,
    encoded_body_size: f64,
    decoded_body_size: f64,
    render_blocking_status: String,
    response_status: f64,
    content_type: String,
}

impl ResourcePerformanceEntry {
    pub(crate) fn from_network_result(
        name: impl Into<String>,
        initiator_type: &'static str,
        start_unix_millis: Option<f64>,
        result: &std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) -> Self {
        match result {
            Ok(response) => {
                Self::from_network_response(name, initiator_type, start_unix_millis, response)
            }
            Err(_) => Self::from_network_failure(name, initiator_type, start_unix_millis),
        }
    }

    pub(crate) fn from_network_response(
        name: impl Into<String>,
        initiator_type: &'static str,
        start_unix_millis: Option<f64>,
        response: &crate::protocol_types::NavigationResponse,
    ) -> Self {
        Self::from_network_response_with_body_size(
            name,
            initiator_type,
            start_unix_millis,
            response,
            response.body_bytes().len(),
        )
    }

    pub(crate) fn from_network_response_with_body_size(
        name: impl Into<String>,
        initiator_type: &'static str,
        start_unix_millis: Option<f64>,
        response: &crate::protocol_types::NavigationResponse,
        body_size: usize,
    ) -> Self {
        Self::from_response_parts(
            name,
            initiator_type,
            start_unix_millis,
            response.status,
            &response.headers,
            body_size as f64,
        )
    }

    pub(crate) fn from_streaming_network_response(
        name: impl Into<String>,
        initiator_type: &'static str,
        start_unix_millis: Option<f64>,
        response: &moli_fetch::ResponseHead,
        body_size: usize,
    ) -> Self {
        Self::from_response_parts(
            name,
            initiator_type,
            start_unix_millis,
            response.status,
            &response.headers,
            body_size as f64,
        )
    }

    pub(crate) fn from_fetch_response(
        name: impl Into<String>,
        initiator_type: &'static str,
        start_unix_millis: Option<f64>,
        response: &moli_fetch::Response,
    ) -> Self {
        Self::from_response_parts(
            name,
            initiator_type,
            start_unix_millis,
            response.status,
            &response.headers,
            response.body_bytes().len() as f64,
        )
    }

    fn from_response_parts(
        name: impl Into<String>,
        initiator_type: &'static str,
        start_unix_millis: Option<f64>,
        response_status: u16,
        response_headers: &[(String, Vec<u8>)],
        body_size: f64,
    ) -> Self {
        let header_size = response_headers
            .iter()
            .map(|(name, value)| name.len() + value.len() + 4)
            .sum::<usize>() as f64;
        let content_type = resource_timing_content_type(response_headers);
        Self {
            name: name.into(),
            initiator_type: initiator_type.to_owned(),
            start_unix_millis,
            transfer_size: (body_size + header_size).max(1.0),
            encoded_body_size: body_size,
            decoded_body_size: body_size,
            // The fetch lifecycle does not yet retain render-blocking metadata.
            // Avoid claiming that a resource blocked rendering until it does.
            render_blocking_status: "non-blocking".to_owned(),
            response_status: f64::from(response_status),
            content_type,
        }
    }

    pub(crate) fn from_child_frame_document_network(
        name: impl Into<String>,
        initiator_type: &'static str,
        start_unix_millis: Option<f64>,
        network: &crate::protocol_types::ChildFrameDocumentNetworkSnapshot,
    ) -> Self {
        let body_size = network.encoded_data_length as f64;
        let header_size = network
            .response_headers
            .iter()
            .map(|(name, value)| name.len() + value.len() + 4)
            .sum::<usize>() as f64;
        let content_type = resource_timing_content_type(&network.response_headers);
        Self {
            name: name.into(),
            initiator_type: initiator_type.to_owned(),
            start_unix_millis,
            transfer_size: (body_size + header_size).max(1.0),
            encoded_body_size: body_size,
            decoded_body_size: body_size,
            render_blocking_status: "non-blocking".to_owned(),
            response_status: f64::from(network.status),
            content_type,
        }
    }

    pub(crate) fn from_network_failure(
        name: impl Into<String>,
        initiator_type: &'static str,
        start_unix_millis: Option<f64>,
    ) -> Self {
        Self::without_network_result(name, initiator_type, start_unix_millis)
    }

    pub(crate) fn without_network_result(
        name: impl Into<String>,
        initiator_type: &'static str,
        start_unix_millis: Option<f64>,
    ) -> Self {
        Self {
            name: name.into(),
            initiator_type: initiator_type.to_owned(),
            start_unix_millis,
            transfer_size: 0.0,
            encoded_body_size: 0.0,
            decoded_body_size: 0.0,
            render_blocking_status: "non-blocking".to_owned(),
            response_status: 0.0,
            content_type: String::new(),
        }
    }
}

pub(crate) fn record_resource_performance_entry(
    scope: &mut v8::PinScope<'_, '_>,
    entry: ResourcePerformanceEntry,
) {
    if !url::Url::parse(&entry.name).is_ok_and(|url| matches!(url.scheme(), "http" | "https")) {
        return;
    }
    let Some(performance) = window_performance_value(scope) else {
        window_state::queue_pending_resource_entry(scope, entry);
        return;
    };
    append_resource_performance_entry(scope, performance, entry);
}

fn append_resource_performance_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    entry: ResourcePerformanceEntry,
) {
    let start_time = entry.start_unix_millis.unwrap_or_else(unix_epoch_millis)
        - performance_slot_number(scope, performance, PERFORMANCE_TIME_ORIGIN_SLOT).unwrap_or(0.0);
    let resource =
        entries::create_performance_entry(scope, "resource", &entry.name, start_time, 0.0, None);
    initialize_resource_timing_slots(
        scope,
        resource,
        &entry.initiator_type,
        entry.transfer_size.max(0.0),
        entry.encoded_body_size.max(0.0),
        entry.decoded_body_size.max(0.0),
        &entry.render_blocking_status,
        entry.response_status,
        &entry.content_type,
    );
    push_performance_entry(scope, performance, resource);
}

pub(in crate::context_bootstrap) fn performance_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, performance, slot)
}

pub(crate) fn performance_slot_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<f64> {
    performance_slot_value(scope, performance, slot).and_then(|value| value.number_value(scope))
}

pub(in crate::context_bootstrap) fn performance_slot_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    performance_slot_value(scope, performance, slot)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(in crate::context_bootstrap) fn performance_slot_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Array>> {
    performance_slot_value(scope, performance, slot)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

pub(in crate::context_bootstrap) fn set_performance_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    slot: &'static str,
    value: v8::Local<'s, v8::Value>,
) {
    set_private_value(scope, performance, slot, value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_content_type_extracts_the_last_valid_mime_before_minimizing() {
        let cases: &[(&[&str], &str)] = &[
            (&[], ""),
            (&["", "invalid", "*/*"], ""),
            (
                &["APPLICATION/X-JAVASCRIPT; charset=UTF-8"],
                "text/javascript",
            ),
            (
                &["text/plain", "application/problem+json"],
                "application/json",
            ),
            (
                &["application/rss+xml, image/svg+xml; charset=UTF-8"],
                "image/svg+xml",
            ),
            (&["image/svg+xml", "text/xml"], "application/xml"),
            (&["application/json", "*/*", "invalid"], "application/json"),
            (&[r#"text/plain; x="a,application/json""#], "text/plain"),
            (&[r#"text/plain; x="a"#, "application/json"], "text/plain"),
            (&["application/json", "application/unknown"], ""),
        ];
        for &(values, expected) in cases {
            let headers = values
                .iter()
                .map(|value| ("cOnTeNt-TyPe".to_owned(), value.as_bytes().to_vec()))
                .collect::<Vec<_>>();
            let entry = ResourcePerformanceEntry::from_response_parts(
                "https://example.test/resource",
                "fetch",
                None,
                200,
                &headers,
                68.0,
            );
            assert_eq!(entry.content_type, expected, "{values:?}");
        }
    }

    #[test]
    fn resource_content_type_retains_only_supported_essences() {
        for essence in [
            "text/html",
            "text/plain",
            "text/css",
            "text/vtt",
            "image/png",
            "image/jpeg",
            "image/gif",
            "image/webp",
            "audio/mpeg",
            "video/mp4",
            "font/woff2",
            "font/collection",
            "application/font-woff",
            "application/wasm",
        ] {
            let headers = vec![(
                "Content-Type".to_owned(),
                format!("{essence}; x=private").into_bytes(),
            )];
            assert_eq!(resource_timing_content_type(&headers), essence);
        }
        for essence in [
            "image/jpe",
            "image/unknown",
            "application/png",
            "audio/unknown",
            "video/unknown",
            "font/unknown",
            "application/vnd.ms-fontobject",
            "application/octet-stream",
            "application/zip",
            "application/pdf",
        ] {
            let headers = vec![("Content-Type".to_owned(), essence.as_bytes().to_vec())];
            assert_eq!(resource_timing_content_type(&headers), "", "{essence}");
        }
    }

    #[test]
    fn failed_network_result_keeps_a_zero_sized_resource_timing_record() {
        let result = Err("failed to fetch stylesheet".to_owned());

        let entry = ResourcePerformanceEntry::from_network_result(
            "https://example.test/non_exist.css",
            "link",
            Some(42.0),
            &result,
        );

        assert_eq!(entry.name, "https://example.test/non_exist.css");
        assert_eq!(entry.initiator_type, "link");
        assert_eq!(entry.start_unix_millis, Some(42.0));
        assert_eq!(entry.transfer_size, 0.0);
        assert_eq!(entry.encoded_body_size, 0.0);
        assert_eq!(entry.decoded_body_size, 0.0);
        assert_eq!(entry.response_status, 0.0);
        assert!(entry.content_type.is_empty());
    }
}
