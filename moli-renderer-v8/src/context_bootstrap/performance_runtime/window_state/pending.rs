use super::time_origin_seed;
use crate::{
    context_bootstrap::{
        dom_time_since_origin_millis,
        performance_runtime::{
            ResourceConnectionTiming, ResourcePerformanceEntry,
            install::{EVENT_COUNTS_TRACKED_TYPES, MIN_LIFECYCLE_TIMING_DELTA_MILLIS},
        },
    },
    util::{array_push_value, get_private_value, set_private_value, v8_string},
};

const WINDOW_PERFORMANCE_PENDING_LIFECYCLE_SLOT: &str = "__moliWindowPerformancePendingLifecycle";
const WINDOW_PERFORMANCE_PENDING_RESOURCES_SLOT: &str = "__moliWindowPerformancePendingResources";
const WINDOW_PERFORMANCE_PENDING_EVENT_COUNTS_SLOT: &str =
    "__moliWindowPerformancePendingEventCounts";

const DOM_CONTENT_LOADED_START_INDEX: usize = 0;
const DOM_CONTENT_LOADED_END_INDEX: usize = 1;
const LOAD_START_INDEX: usize = 2;
const LOAD_END_INDEX: usize = 3;
const LIFECYCLE_TIMESTAMP_COUNT: usize = 4;

pub(in crate::context_bootstrap::performance_runtime) fn record_pending_dom_content_loaded_start(
    scope: &mut v8::PinScope<'_, '_>,
) -> f64 {
    record_pending_lifecycle_timestamp(scope, DOM_CONTENT_LOADED_START_INDEX)
}

pub(in crate::context_bootstrap::performance_runtime) fn record_pending_dom_content_loaded_end(
    scope: &mut v8::PinScope<'_, '_>,
) -> f64 {
    record_pending_lifecycle_timestamp(scope, DOM_CONTENT_LOADED_END_INDEX)
}

pub(in crate::context_bootstrap::performance_runtime) fn record_pending_load_start(
    scope: &mut v8::PinScope<'_, '_>,
) -> f64 {
    record_pending_lifecycle_timestamp(scope, LOAD_START_INDEX)
}

pub(in crate::context_bootstrap::performance_runtime) fn record_pending_load_end(
    scope: &mut v8::PinScope<'_, '_>,
) -> f64 {
    record_pending_lifecycle_timestamp(scope, LOAD_END_INDEX)
}

pub(in crate::context_bootstrap::performance_runtime) fn take_pending_lifecycle_timestamps<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> [Option<f64>; LIFECYCLE_TIMESTAMP_COUNT] {
    let Some(values) = private_array(scope, window, WINDOW_PERFORMANCE_PENDING_LIFECYCLE_SLOT)
    else {
        return [None; LIFECYCLE_TIMESTAMP_COUNT];
    };
    clear_private_value(scope, window, WINDOW_PERFORMANCE_PENDING_LIFECYCLE_SLOT);
    std::array::from_fn(|index| {
        values
            .get_index(scope, index as u32)
            .filter(|value| !value.is_undefined())
            .and_then(|value| value.number_value(scope))
    })
}

pub(in crate::context_bootstrap::performance_runtime) fn queue_pending_resource_entry(
    scope: &mut v8::PinScope<'_, '_>,
    entry: ResourcePerformanceEntry,
) {
    let global = scope.get_current_context().global(scope);
    let entries = ensure_private_array(scope, global, WINDOW_PERFORMANCE_PENDING_RESOURCES_SLOT, 0);
    let record = v8::Array::new(scope, 25);
    let Some(name) = v8_string(scope, &entry.name) else {
        return;
    };
    let Some(initiator_type) = v8_string(scope, &entry.initiator_type) else {
        return;
    };
    let Some(render_blocking_status) = v8_string(scope, &entry.render_blocking_status) else {
        return;
    };
    let Some(content_type) = v8_string(scope, &entry.content_type) else {
        return;
    };
    let Some(next_hop_protocol) = v8_string(scope, &entry.next_hop_protocol) else {
        return;
    };
    let Some(delivery_type) = v8_string(scope, &entry.delivery_type) else {
        return;
    };
    let Some(content_encoding) = v8_string(scope, &entry.content_encoding) else {
        return;
    };
    let start_time: v8::Local<'_, v8::Value> = entry
        .start_unix_millis
        .map(|value| v8::Number::new(scope, value).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    let fields = [
        name.into(),
        initiator_type.into(),
        start_time,
        v8::Number::new(scope, entry.transfer_size).into(),
        v8::Number::new(scope, entry.encoded_body_size).into(),
        v8::Number::new(scope, entry.decoded_body_size).into(),
        render_blocking_status.into(),
        v8::Number::new(scope, entry.response_status).into(),
        content_type.into(),
        entry
            .end_unix_millis
            .map(|value| v8::Number::new(scope, value).into())
            .unwrap_or_else(|| v8::undefined(scope).into()),
        entry
            .response_start_unix_millis
            .map(|value| v8::Number::new(scope, value).into())
            .unwrap_or_else(|| v8::undefined(scope).into()),
        next_hop_protocol.into(),
        delivery_type.into(),
        content_encoding.into(),
        entry
            .final_response_headers_start_unix_millis
            .map(|value| v8::Number::new(scope, value).into())
            .unwrap_or_else(|| v8::undefined(scope).into()),
        entry
            .first_interim_response_start_unix_millis
            .map(|value| v8::Number::new(scope, value).into())
            .unwrap_or_else(|| v8::undefined(scope).into()),
        entry
            .request_start_unix_millis
            .map(|value| v8::Number::new(scope, value).into())
            .unwrap_or_else(|| v8::undefined(scope).into()),
        entry
            .fetch_start_unix_millis
            .map(|value| v8::Number::new(scope, value).into())
            .unwrap_or_else(|| v8::undefined(scope).into()),
        entry
            .connection_timing
            .map(|value| v8::Number::new(scope, value.domain_lookup_start).into())
            .unwrap_or_else(|| v8::undefined(scope).into()),
        entry
            .connection_timing
            .map(|value| v8::Number::new(scope, value.domain_lookup_end).into())
            .unwrap_or_else(|| v8::undefined(scope).into()),
        entry
            .connection_timing
            .map(|value| v8::Number::new(scope, value.connect_start).into())
            .unwrap_or_else(|| v8::undefined(scope).into()),
        entry
            .connection_timing
            .map(|value| v8::Number::new(scope, value.connect_end).into())
            .unwrap_or_else(|| v8::undefined(scope).into()),
        entry
            .connection_timing
            .and_then(|value| value.secure_connection_start)
            .map(|value| v8::Number::new(scope, value).into())
            .unwrap_or_else(|| v8::undefined(scope).into()),
        entry
            .redirect_start_unix_millis
            .map(|value| v8::Number::new(scope, value).into())
            .unwrap_or_else(|| v8::undefined(scope).into()),
        entry
            .redirect_end_unix_millis
            .map(|value| v8::Number::new(scope, value).into())
            .unwrap_or_else(|| v8::undefined(scope).into()),
    ];
    for (index, value) in fields.into_iter().enumerate() {
        let _ = record.set_index(scope, index as u32, value);
    }
    array_push_value(scope, entries, record.into());
}

pub(in crate::context_bootstrap::performance_runtime) fn take_pending_resource_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Vec<ResourcePerformanceEntry> {
    let Some(records) = private_array(scope, window, WINDOW_PERFORMANCE_PENDING_RESOURCES_SLOT)
    else {
        return Vec::new();
    };
    clear_private_value(scope, window, WINDOW_PERFORMANCE_PENDING_RESOURCES_SLOT);
    (0..records.length())
        .filter_map(|index| {
            let record = records
                .get_index(scope, index)
                .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())?;
            Some(ResourcePerformanceEntry {
                preload_state: Default::default(),
                name: array_string(scope, record, 0)?,
                initiator_type: array_string(scope, record, 1)?,
                redirect_start_unix_millis: record
                    .get_index(scope, 23)
                    .filter(|value| !value.is_undefined())
                    .and_then(|value| value.number_value(scope)),
                redirect_end_unix_millis: record
                    .get_index(scope, 24)
                    .filter(|value| !value.is_undefined())
                    .and_then(|value| value.number_value(scope)),
                fetch_start_unix_millis: record
                    .get_index(scope, 17)
                    .filter(|value| !value.is_undefined())
                    .and_then(|value| value.number_value(scope)),
                connection_timing: if record
                    .get_index(scope, 18)
                    .is_some_and(|value| !value.is_undefined())
                {
                    Some(ResourceConnectionTiming {
                        domain_lookup_start: array_number(scope, record, 18)?,
                        domain_lookup_end: array_number(scope, record, 19)?,
                        connect_start: array_number(scope, record, 20)?,
                        connect_end: array_number(scope, record, 21)?,
                        secure_connection_start: record
                            .get_index(scope, 22)
                            .filter(|value| !value.is_undefined())
                            .and_then(|value| value.number_value(scope)),
                    })
                } else {
                    None
                },
                start_unix_millis: record
                    .get_index(scope, 2)
                    .filter(|value| !value.is_undefined())
                    .and_then(|value| value.number_value(scope)),
                transfer_size: array_number(scope, record, 3)?,
                encoded_body_size: array_number(scope, record, 4)?,
                decoded_body_size: array_number(scope, record, 5)?,
                render_blocking_status: array_string(scope, record, 6)?,
                response_status: array_number(scope, record, 7)?,
                content_type: array_string(scope, record, 8)?,
                end_unix_millis: record
                    .get_index(scope, 9)
                    .filter(|value| !value.is_undefined())
                    .and_then(|value| value.number_value(scope)),
                response_start_unix_millis: record
                    .get_index(scope, 10)
                    .filter(|value| !value.is_undefined())
                    .and_then(|value| value.number_value(scope)),
                request_start_unix_millis: record
                    .get_index(scope, 16)
                    .filter(|value| !value.is_undefined())
                    .and_then(|value| value.number_value(scope)),
                next_hop_protocol: array_string(scope, record, 11)?,
                delivery_type: array_string(scope, record, 12)?,
                content_encoding: array_string(scope, record, 13)?,
                final_response_headers_start_unix_millis: record
                    .get_index(scope, 14)
                    .filter(|value| !value.is_undefined())
                    .and_then(|value| value.number_value(scope)),
                first_interim_response_start_unix_millis: record
                    .get_index(scope, 15)
                    .filter(|value| !value.is_undefined())
                    .and_then(|value| value.number_value(scope)),
            })
        })
        .collect()
}

pub(in crate::context_bootstrap::performance_runtime) fn increment_pending_event_count(
    scope: &mut v8::PinScope<'_, '_>,
    index: usize,
) {
    let global = scope.get_current_context().global(scope);
    let values = ensure_private_array(
        scope,
        global,
        WINDOW_PERFORMANCE_PENDING_EVENT_COUNTS_SLOT,
        EVENT_COUNTS_TRACKED_TYPES.len() as i32,
    );
    let current = values
        .get_index(scope, index as u32)
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    let next = v8::Integer::new_from_unsigned(scope, current.saturating_add(1));
    let _ = values.set_index(scope, index as u32, next.into());
}

pub(in crate::context_bootstrap::performance_runtime) fn take_pending_event_counts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Vec<u32> {
    let Some(values) = private_array(scope, window, WINDOW_PERFORMANCE_PENDING_EVENT_COUNTS_SLOT)
    else {
        return Vec::new();
    };
    clear_private_value(scope, window, WINDOW_PERFORMANCE_PENDING_EVENT_COUNTS_SLOT);
    (0..EVENT_COUNTS_TRACKED_TYPES.len())
        .map(|index| {
            values
                .get_index(scope, index as u32)
                .and_then(|value| value.uint32_value(scope))
                .unwrap_or(0)
        })
        .collect()
}

pub(super) fn clear_pending_window_performance_state(
    scope: &mut v8::PinScope<'_, '_>,
    window: v8::Local<'_, v8::Object>,
) {
    for slot in [
        WINDOW_PERFORMANCE_PENDING_LIFECYCLE_SLOT,
        WINDOW_PERFORMANCE_PENDING_RESOURCES_SLOT,
        WINDOW_PERFORMANCE_PENDING_EVENT_COUNTS_SLOT,
    ] {
        clear_private_value(scope, window, slot);
    }
}

fn record_pending_lifecycle_timestamp(scope: &mut v8::PinScope<'_, '_>, index: usize) -> f64 {
    let global = scope.get_current_context().global(scope);
    let values = ensure_private_array(
        scope,
        global,
        WINDOW_PERFORMANCE_PENDING_LIFECYCLE_SLOT,
        LIFECYCLE_TIMESTAMP_COUNT as i32,
    );
    if let Some(existing) = values
        .get_index(scope, index as u32)
        .filter(|value| !value.is_undefined())
        .and_then(|value| value.number_value(scope))
    {
        return existing;
    }
    let previous = index
        .checked_sub(1)
        .and_then(|previous| values.get_index(scope, previous as u32))
        .and_then(|value| value.number_value(scope))
        .unwrap_or(0.0);
    let now = dom_time_since_origin_millis(time_origin_seed(scope, global)).max(0.0);
    let timestamp = if now > previous {
        now
    } else {
        previous + MIN_LIFECYCLE_TIMING_DELTA_MILLIS
    };
    let timestamp_value = v8::Number::new(scope, timestamp);
    let _ = values.set_index(scope, index as u32, timestamp_value.into());
    timestamp
}

fn ensure_private_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    slot: &'static str,
    initial_length: i32,
) -> v8::Local<'s, v8::Array> {
    if let Some(values) = private_array(scope, window, slot) {
        return values;
    }
    let values = v8::Array::new(scope, initial_length);
    set_private_value(scope, window, slot, values.into());
    values
}

fn private_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_value(scope, window, slot)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

fn clear_private_value(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    slot: &'static str,
) {
    let undefined = v8::undefined(scope);
    set_private_value(scope, object, slot, undefined.into());
}

fn array_string(
    scope: &mut v8::PinScope<'_, '_>,
    values: v8::Local<'_, v8::Array>,
    index: u32,
) -> Option<String> {
    values
        .get_index(scope, index)?
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

fn array_number(
    scope: &mut v8::PinScope<'_, '_>,
    values: v8::Local<'_, v8::Array>,
    index: u32,
) -> Option<f64> {
    values.get_index(scope, index)?.number_value(scope)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_resource_entries_preserve_response_metadata_and_header_boundaries() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let window = context.global(scope);
        let mut entry = ResourcePerformanceEntry::from_network_failure(
            "https://pending-resource.test/a",
            "fetch",
            Some(1000.0),
        );
        entry.delivery_type = "cache".to_owned();
        entry.content_encoding = "gzip".to_owned();
        entry.request_start_unix_millis = Some(1000.5);
        entry.first_interim_response_start_unix_millis = Some(1001.0);
        entry.final_response_headers_start_unix_millis = Some(1002.0);
        queue_pending_resource_entry(scope, entry);
        let entries = take_pending_resource_entries(scope, window);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].delivery_type, "cache");
        assert_eq!(entries[0].content_encoding, "gzip");
        assert_eq!(entries[0].request_start_unix_millis, Some(1000.5));
        assert_eq!(
            entries[0].first_interim_response_start_unix_millis,
            Some(1001.0)
        );
        assert_eq!(
            entries[0].final_response_headers_start_unix_millis,
            Some(1002.0)
        );
        assert!(take_pending_resource_entries(scope, window).is_empty());
    }
}
