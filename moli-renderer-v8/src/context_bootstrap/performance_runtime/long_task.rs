use moli_webapi::performance::long_task;

use super::super::performance_observer_runtime::queue_matching_performance_observers;
use super::entries::assign_queued_performance_entry_identity;
use super::{
    PERFORMANCE_TIME_ORIGIN_SLOT, ensure_current_window_performance, performance_slot_array,
    performance_slot_number,
};
use crate::util::{define_v8_array_data_property, set_private_value};

pub(in crate::context_bootstrap) const LONG_TASK_BUFFER_SLOT: &str =
    "__moliPerformanceLongTaskBuffer";
const LONG_TASK_BUFFER_SIZE: u32 = 200;
const LONG_TASK_DROPPED_ENTRIES_SLOT: &str = "__moliLongTaskDroppedEntries";

pub(in crate::context_bootstrap) fn long_task_dropped_entries_count<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
) -> u64 {
    performance_slot_number(scope, performance, LONG_TASK_DROPPED_ENTRIES_SLOT).unwrap_or_default()
        as u64
}

pub(crate) fn record_long_task_entry(
    scope: &mut v8::PinScope<'_, '_>,
    name: &'static str,
    start_unix_millis: f64,
    duration: f64,
    attribution: long_task::TaskAttribution,
) {
    let Ok(performance) = ensure_current_window_performance(scope) else {
        return;
    };
    let origin = performance_slot_number(scope, performance, PERFORMANCE_TIME_ORIGIN_SLOT)
        .unwrap_or(start_unix_millis);
    let start_time = moli_time::relative_dom_time_millis(start_unix_millis, origin);
    let entry = long_task::create(scope, performance, name, start_time, duration, attribution);
    assign_queued_performance_entry_identity(scope, performance, entry);
    let buffer =
        performance_slot_array(scope, performance, LONG_TASK_BUFFER_SLOT).unwrap_or_else(|| {
            let buffer = v8::Array::new(scope, 0);
            set_private_value(scope, performance, LONG_TASK_BUFFER_SLOT, buffer.into());
            buffer
        });
    if buffer.length() < LONG_TASK_BUFFER_SIZE {
        let _ = define_v8_array_data_property(scope, buffer, buffer.length(), entry.into());
    } else {
        let count = long_task_dropped_entries_count(scope, performance).saturating_add(1);
        let value = v8::Number::new(scope, count as f64);
        set_private_value(
            scope,
            performance,
            LONG_TASK_DROPPED_ENTRIES_SLOT,
            value.into(),
        );
    }
    queue_matching_performance_observers(scope, performance, entry, "longtask");
}
