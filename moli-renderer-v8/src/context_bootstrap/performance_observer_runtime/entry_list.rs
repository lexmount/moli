use super::*;
use crate::util::{get_private_value, serialize_v8_iter_array};
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PerformanceObserverEntryList.getEntriesByType")]
struct PerformanceEntryListGetEntriesByTypeArgs {
    #[webidl(required)]
    entry_type: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PerformanceObserverEntryList.getEntriesByName")]
struct PerformanceEntryListGetEntriesByNameArgs {
    #[webidl(required)]
    name: String,
    entry_type: Option<String>,
}

pub(in crate::context_bootstrap) fn performance_entry_list_get_entries_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let entries = performance_entry_list_entries(scope, args.this())
        .unwrap_or_else(|| v8::Array::new(scope, 0));
    rv.set(filtered_entry_list_entries(scope, entries, None, None).into());
}

pub(in crate::context_bootstrap) fn performance_entry_list_get_entries_by_type_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let entries = performance_entry_list_entries(scope, args.this())
        .unwrap_or_else(|| v8::Array::new(scope, 0));
    let Some(parsed) = webidl::parse_args::<PerformanceEntryListGetEntriesByTypeArgs>(scope, &args)
    else {
        return;
    };
    rv.set(filtered_entry_list_entries(scope, entries, Some(&parsed.entry_type), None).into());
}

pub(in crate::context_bootstrap) fn performance_entry_list_get_entries_by_name_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let entries = performance_entry_list_entries(scope, args.this())
        .unwrap_or_else(|| v8::Array::new(scope, 0));
    let Some(parsed) = webidl::parse_args::<PerformanceEntryListGetEntriesByNameArgs>(scope, &args)
    else {
        return;
    };
    rv.set(
        filtered_entry_list_entries(
            scope,
            entries,
            parsed.entry_type.as_deref(),
            Some(&parsed.name),
        )
        .into(),
    );
}

pub(in crate::context_bootstrap) fn filtered_entry_list_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entries: v8::Local<'s, v8::Array>,
    expected_type: Option<&str>,
    expected_name: Option<&str>,
) -> v8::Local<'s, v8::Array> {
    let mut filtered_entries: Vec<(u32, f64, v8::Local<'s, v8::Object>)> = Vec::new();
    for index in 0..entries.length() {
        let Some(entry) = entries.get_index(scope, index) else {
            continue;
        };
        let Ok(entry) = v8::Local::<v8::Object>::try_from(entry) else {
            continue;
        };
        if expected_type.is_some_and(|expected| {
            performance_entry_slot_string(scope, entry, PERFORMANCE_ENTRY_TYPE_SLOT).as_deref()
                != Some(expected)
        }) {
            continue;
        }
        if expected_name.is_some_and(|expected| {
            performance_entry_slot_string(scope, entry, PERFORMANCE_ENTRY_NAME_SLOT).as_deref()
                != Some(expected)
        }) {
            continue;
        }
        let start_time =
            performance_entry_slot_number(scope, entry, PERFORMANCE_ENTRY_START_TIME_SLOT)
                .unwrap_or(0.0);
        filtered_entries.push((index, start_time, entry));
    }
    filtered_entries.sort_by(
        |(left_index, left_start, _), (right_index, right_start, _)| {
            left_start
                .total_cmp(right_start)
                .then_with(|| left_index.cmp(right_index))
        },
    );

    serialize_v8_iter_array(
        scope,
        filtered_entries.into_iter().map(|(_, _, entry)| entry),
    )
    .unwrap_or_else(|| v8::Array::new(scope, 0))
}

fn performance_entry_list_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_value(scope, list, PERFORMANCE_ENTRY_LIST_ENTRIES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}
