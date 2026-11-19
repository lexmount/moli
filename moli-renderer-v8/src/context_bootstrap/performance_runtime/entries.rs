use super::super::performance_observer_runtime::queue_matching_performance_observers;
use super::*;
use crate::util::{get_private_value, set_private_value};
use crate::webidl;
use moli_webapi::performance::entry_list::filtered_entry_list_entries;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Performance.getEntriesByType")]
struct PerformanceGetEntriesByTypeArgs {
    #[webidl(required)]
    entry_type: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Performance.getEntriesByName")]
struct PerformanceGetEntriesByNameArgs {
    #[webidl(required, converter = "raw")]
    name: PerformanceEntryName,
    entry_type: Option<String>,
}

pub(in crate::context_bootstrap) use moli_webapi::performance::entries::{
    create_performance_entry, initialize_performance_entry_slots,
    initialize_resource_timing_response_metadata, initialize_resource_timing_slots,
    install_performance_entry_template_bindings, performance_entry_slot_number,
    performance_entry_slot_string, set_performance_entry_slot_number,
};

pub(in crate::context_bootstrap) fn performance_get_entries_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    super::install::ensure_navigation_performance_entry(scope, args.this());
    rv.set(filtered_performance_entries(scope, args.this(), None, None).into());
}

pub(in crate::context_bootstrap) fn performance_get_entries_by_type_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<PerformanceGetEntriesByTypeArgs>(scope, &args) else {
        return;
    };
    if parsed.entry_type == "navigation" {
        super::install::ensure_navigation_performance_entry(scope, args.this());
    }
    rv.set(filtered_performance_entries(scope, args.this(), Some(&parsed.entry_type), None).into());
}

pub(in crate::context_bootstrap) fn performance_get_entries_by_name_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<PerformanceGetEntriesByNameArgs>(scope, &args) else {
        return;
    };
    if parsed
        .entry_type
        .as_deref()
        .is_none_or(|value| value == "navigation")
    {
        super::install::ensure_navigation_performance_entry(scope, args.this());
    }
    rv.set(
        filtered_performance_entries(
            scope,
            args.this(),
            parsed.entry_type.as_deref(),
            Some(&parsed.name),
        )
        .into(),
    );
}

pub(super) fn filtered_performance_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    expected_type: Option<&str>,
    expected_name: Option<&PerformanceEntryName>,
) -> v8::Local<'s, v8::Array> {
    let Some(entries) = performance_slot_array(scope, performance, PERFORMANCE_ENTRIES_SLOT) else {
        return v8::Array::new(scope, 0);
    };
    filtered_entry_list_entries(scope, entries, expected_type, expected_name)
}

pub(super) fn find_latest_performance_mark_start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    name: &PerformanceEntryName,
) -> Option<f64> {
    let entries = performance_slot_array(scope, performance, PERFORMANCE_ENTRIES_SLOT)?;
    let mut found = None;
    for index in 0..entries.length() {
        let entry = entries.get_index(scope, index)?;
        let entry = v8::Local::<v8::Object>::try_from(entry).ok()?;
        if performance_entry_slot_string(scope, entry, PERFORMANCE_ENTRY_TYPE_SLOT).as_deref()
            == Some("mark")
            && PerformanceEntryName::from_entry(scope, entry).as_ref() == Some(name)
        {
            found = performance_entry_slot_number(scope, entry, PERFORMANCE_ENTRY_START_TIME_SLOT);
        }
    }
    found
}

pub(super) fn push_performance_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
) {
    let Some(context) = performance.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    if super::install::is_window_performance(scope, performance) {
        super::install::ensure_navigation_performance_entry(scope, performance);
    }
    assign_queued_performance_entry_identity(scope, performance, entry);
    let entry_type = performance_entry_slot_string(scope, entry, PERFORMANCE_ENTRY_TYPE_SLOT)
        .unwrap_or_default();
    if entry_type == "resource" {
        queue_matching_performance_observers(scope, performance, entry, &entry_type);
        super::resource_buffer::add_resource_timing_entry(scope, performance, entry);
        return;
    }
    append_performance_entry(scope, performance, entry);
    queue_matching_performance_observers(scope, performance, entry, &entry_type);
}

pub(super) fn assign_navigation_performance_entry_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
) {
    let id = ensure_performance_entry_id(scope, performance, entry);
    set_performance_entry_slot_number(scope, entry, PERFORMANCE_ENTRY_NAVIGATION_ID_SLOT, id);
}

pub(super) fn assign_queued_performance_entry_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
) {
    ensure_performance_entry_id(scope, performance, entry);
    let navigation_id = get_private_value(scope, performance, PERFORMANCE_NAVIGATION_ENTRY_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|navigation| {
            performance_entry_slot_number(scope, navigation, PERFORMANCE_ENTRY_ID_SLOT)
        })
        .unwrap_or(0.0);
    set_performance_entry_slot_number(
        scope,
        entry,
        PERFORMANCE_ENTRY_NAVIGATION_ID_SLOT,
        navigation_id,
    );
}

fn ensure_performance_entry_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
) -> f64 {
    if let Some(id) = performance_entry_slot_number(scope, entry, PERFORMANCE_ENTRY_ID_SLOT)
        .filter(|id| *id > 0.0)
    {
        return id;
    }
    let last_id = get_private_value(scope, performance, PERFORMANCE_LAST_ENTRY_ID_SLOT)
        .and_then(|value| value.number_value(scope))
        .unwrap_or_else(|| fastrand::u64(100..=10_000) as f64);
    let id = last_id + fastrand::u64(1..=10) as f64;
    let id_value = v8::Number::new(scope, id);
    set_private_value(
        scope,
        performance,
        PERFORMANCE_LAST_ENTRY_ID_SLOT,
        id_value.into(),
    );
    set_performance_entry_slot_number(scope, entry, PERFORMANCE_ENTRY_ID_SLOT, id);
    id
}

pub(super) fn append_performance_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
) {
    let Some(entries) = performance_slot_array(scope, performance, PERFORMANCE_ENTRIES_SLOT) else {
        return;
    };
    let _ = entries.set_index(scope, entries.length(), entry.into());
}
