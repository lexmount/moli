use crate::{
    context_bootstrap::performance_runtime::ResourcePerformanceEntry,
    util::{array_push_value, get_private_value, set_private_value, v8_string},
};
use moli_webapi::performance::event_counts::EVENT_COUNTS_TRACKED_TYPES;

const WINDOW_PERFORMANCE_PROJECTED_LIFECYCLE_SLOT: &str =
    "__moliWindowPerformanceProjectedLifecycle";
const WINDOW_PERFORMANCE_PENDING_RESOURCES_SLOT: &str = "__moliWindowPerformancePendingResources";
const WINDOW_PERFORMANCE_PENDING_EVENT_COUNTS_SLOT: &str =
    "__moliWindowPerformancePendingEventCounts";

const LIFECYCLE_TIMESTAMP_COUNT: usize = 4;

pub(in crate::context_bootstrap::performance_runtime) fn store_projected_lifecycle_timestamps<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    timestamps: [Option<f64>; 4],
) {
    if timestamps.iter().all(Option::is_none) {
        return;
    }
    let array = v8::Array::new(scope, timestamps.len() as i32);
    for (index, timestamp) in timestamps.into_iter().enumerate() {
        if let Some(timestamp) = timestamp {
            let number = v8::Number::new(scope, timestamp);
            let _ = array.set_index(scope, index as u32, number.into());
        }
    }
    set_private_value(
        scope,
        window,
        WINDOW_PERFORMANCE_PROJECTED_LIFECYCLE_SLOT,
        array.into(),
    );
}

pub(in crate::context_bootstrap::performance_runtime) fn take_projected_lifecycle_timestamps<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> [Option<f64>; LIFECYCLE_TIMESTAMP_COUNT] {
    let Some(values) = private_array(scope, window, WINDOW_PERFORMANCE_PROJECTED_LIFECYCLE_SLOT)
    else {
        return [None; LIFECYCLE_TIMESTAMP_COUNT];
    };
    clear_private_value(scope, window, WINDOW_PERFORMANCE_PROJECTED_LIFECYCLE_SLOT);
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
    let record = v8::Array::new(scope, 18);
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
    let Some(initiator_url) = v8_string(scope, &entry.initiator_url) else {
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
        initiator_url.into(),
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
                initiator_url: array_string(scope, record, 9)?,
                end_unix_millis: record
                    .get_index(scope, 10)
                    .filter(|value| !value.is_undefined())
                    .and_then(|value| value.number_value(scope)),
                response_start_unix_millis: record
                    .get_index(scope, 11)
                    .filter(|value| !value.is_undefined())
                    .and_then(|value| value.number_value(scope)),
                next_hop_protocol: array_string(scope, record, 12)?,
                delivery_type: array_string(scope, record, 13)?,
                content_encoding: array_string(scope, record, 14)?,
                final_response_headers_start_unix_millis: record
                    .get_index(scope, 15)
                    .filter(|value| !value.is_undefined())
                    .and_then(|value| value.number_value(scope)),
                first_interim_response_start_unix_millis: record
                    .get_index(scope, 16)
                    .filter(|value| !value.is_undefined())
                    .and_then(|value| value.number_value(scope)),
                request_start_unix_millis: record
                    .get_index(scope, 17)
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
        WINDOW_PERFORMANCE_PROJECTED_LIFECYCLE_SLOT,
        WINDOW_PERFORMANCE_PENDING_RESOURCES_SLOT,
        WINDOW_PERFORMANCE_PENDING_EVENT_COUNTS_SLOT,
    ] {
        clear_private_value(scope, window, slot);
    }
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
        entry.initiator_url = "https://pending-resource.test/initiator.js".to_owned();
        entry.next_hop_protocol = "h2".to_owned();
        entry.delivery_type = "cache".to_owned();
        entry.content_encoding = "gzip".to_owned();
        entry.request_start_unix_millis = Some(1000.5);
        entry.first_interim_response_start_unix_millis = Some(1001.0);
        entry.final_response_headers_start_unix_millis = Some(1002.0);
        queue_pending_resource_entry(scope, entry);
        let entries = take_pending_resource_entries(scope, window);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].initiator_url,
            "https://pending-resource.test/initiator.js"
        );
        assert_eq!(entries[0].next_hop_protocol, "h2");
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
