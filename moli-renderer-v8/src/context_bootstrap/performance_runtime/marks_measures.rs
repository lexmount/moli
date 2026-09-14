use super::entries::{
    create_performance_entry, find_latest_performance_mark_start,
    initialize_performance_entry_slots,
};
use super::*;
use crate::context_bootstrap::structured_clone_value;
use crate::util::serialize_v8_iter_array;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Performance.mark")]
struct PerformanceMarkArgs {
    #[webidl(required)]
    name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PerformanceMark")]
struct PerformanceMarkConstructorArgs {
    #[webidl(required)]
    name: String,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "PerformanceMarkOptions")]
struct PerformanceMarkOptions<'s> {
    #[webidl(converter = "raw")]
    detail: Option<v8::Local<'s, v8::Value>>,
    #[webidl(name = "startTime", converter = "double")]
    start_time: Option<f64>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Performance.clearMarks")]
struct PerformanceClearMarksArgs {
    name: Option<String>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Performance.measure")]
struct PerformanceMeasureArgs {
    #[webidl(required)]
    name: String,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "PerformanceMeasureOptions")]
struct PerformanceMeasureOptions<'s> {
    #[webidl(converter = "raw")]
    detail: Option<v8::Local<'s, v8::Value>>,
    #[webidl(converter = "double")]
    duration: Option<f64>,
    #[webidl(converter = "raw")]
    end: Option<v8::Local<'s, v8::Value>>,
    #[webidl(converter = "raw")]
    start: Option<v8::Local<'s, v8::Value>>,
}

enum MeasureBoundary {
    Mark(String),
    Timestamp(f64),
}

struct ParsedPerformanceMeasure<'s> {
    start: Option<MeasureBoundary>,
    duration: Option<f64>,
    end: Option<MeasureBoundary>,
    detail: Option<v8::Local<'s, v8::Value>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Performance.clearMeasures")]
struct PerformanceClearMeasuresArgs {
    name: Option<String>,
}

pub(in crate::context_bootstrap) fn performance_mark_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(performance) = require_performance_receiver(scope, args.this()) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<PerformanceMarkArgs>(scope, &args) else {
        return;
    };
    let Some(options) = parse_performance_mark_options(
        scope,
        &args,
        webidl::Context::argument("Performance.mark", 2),
    ) else {
        return;
    };
    let Some((start_time, detail)) =
        prepare_performance_mark(scope, performance, &parsed.name, options)
    else {
        return;
    };
    let entry = create_performance_entry(scope, "mark", &parsed.name, start_time, 0.0, detail);
    push_performance_entry(scope, performance, entry);
    rv.set(entry.into());
}

pub(in crate::context_bootstrap) fn performance_mark_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        webidl::throw_type_error(
            scope,
            "Failed to construct 'PerformanceMark': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<PerformanceMarkConstructorArgs>(scope, &args) else {
        return;
    };
    let Some(options) = parse_performance_mark_options(
        scope,
        &args,
        webidl::Context::argument("PerformanceMark", 2),
    ) else {
        return;
    };
    let Some(performance) = ensure_current_performance_for_api(scope) else {
        webidl::throw_type_error(scope, "Failed to construct 'PerformanceMark'.");
        return;
    };
    let Some((start_time, detail)) =
        prepare_performance_mark(scope, performance, &parsed.name, options)
    else {
        return;
    };
    initialize_performance_entry_slots(
        scope,
        args.this(),
        "mark",
        &parsed.name,
        start_time,
        0.0,
        detail,
    );
    rv.set(args.this().into());
}

fn parse_performance_mark_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    context: webidl::Context,
) -> Option<PerformanceMarkOptions<'s>> {
    match webidl::dictionary_arg(args, 1, context) {
        Ok(Some(options)) => {
            match webidl::parse_dictionary_object::<PerformanceMarkOptions>(scope, options) {
                Ok(options) => Some(options),
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    None
                }
            }
        }
        Ok(None) => Some(PerformanceMarkOptions::default()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn prepare_performance_mark<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    name: &str,
    options: PerformanceMarkOptions<'s>,
) -> Option<(f64, Option<v8::Local<'s, v8::Value>>)> {
    let start_time = options.start_time.unwrap_or_else(|| {
        unix_epoch_millis()
            - performance_slot_number(scope, performance, PERFORMANCE_TIME_ORIGIN_SLOT)
                .unwrap_or(0.0)
    });
    if start_time < 0.0 {
        webidl::throw_type_error(
            scope,
            &format!("'{name}' cannot have a negative start time."),
        );
        return None;
    }
    if install::is_window_performance(scope, performance)
        && install::PERFORMANCE_TIMING_ATTRIBUTE_NAMES.contains(&name)
    {
        webidl::throw_dom_exception(
            scope,
            "SyntaxError",
            &format!(
                "'{name}' is part of the PerformanceTiming interface and cannot be used as a mark name."
            ),
        );
        return None;
    }
    clone_user_timing_detail(scope, options.detail).map(|detail| (start_time, detail))
}

fn require_performance_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if performance_slot_array(scope, receiver, PERFORMANCE_ENTRIES_SLOT).is_none() {
        webidl::throw_type_error(scope, "Illegal invocation");
        return None;
    }
    Some(receiver)
}

pub(in crate::context_bootstrap) fn performance_clear_marks_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(performance) = require_performance_receiver(scope, args.this()) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<PerformanceClearMarksArgs>(scope, &args) else {
        return;
    };
    let entries = performance_slot_array(scope, performance, PERFORMANCE_ENTRIES_SLOT)
        .expect("branded Performance receiver should retain its entries slot");
    let mut next = Vec::new();
    for index in 0..entries.length() {
        let Some(entry) = entries.get_index(scope, index) else {
            continue;
        };
        let Ok(entry) = v8::Local::<v8::Object>::try_from(entry) else {
            continue;
        };
        let is_mark = performance_entry_slot_string(scope, entry, PERFORMANCE_ENTRY_TYPE_SLOT)
            .as_deref()
            == Some("mark");
        let keep = if !is_mark {
            true
        } else {
            match (
                &parsed.name,
                performance_entry_slot_string(scope, entry, PERFORMANCE_ENTRY_NAME_SLOT),
            ) {
                (Some(expected), Some(actual)) => actual != *expected,
                (Some(_), None) => true,
                (None, _) => false,
            }
        };
        if keep {
            next.push(entry);
        }
    }
    let next = serialize_v8_iter_array(scope, next).unwrap_or_else(|| v8::Array::new(scope, 0));
    set_performance_slot_value(scope, performance, PERFORMANCE_ENTRIES_SLOT, next.into());
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn performance_measure_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(performance) = require_performance_receiver(scope, args.this()) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<PerformanceMeasureArgs>(scope, &args) else {
        return;
    };
    let Some(parsed_measure) = parse_performance_measure_arguments(scope, &args) else {
        return;
    };
    let now = unix_epoch_millis()
        - performance_slot_number(scope, performance, PERFORMANCE_TIME_ORIGIN_SLOT).unwrap_or(0.0);
    let start = match parsed_measure.start.as_ref() {
        Some(start) => match resolve_measure_boundary(scope, performance, &parsed.name, start) {
            Ok(start) => start,
            Err(()) => return,
        },
        None => 0.0,
    };
    let end = match parsed_measure.end.as_ref() {
        Some(end) => match resolve_measure_boundary(scope, performance, &parsed.name, end) {
            Ok(end) => end,
            Err(()) => return,
        },
        None => now,
    };
    let (start_time, end_time) = match parsed_measure.duration {
        Some(duration) if parsed_measure.start.is_none() => (end - duration, end),
        Some(duration) => (start, start + duration),
        None => (start, end),
    };
    let detail = match clone_user_timing_detail(scope, parsed_measure.detail) {
        Some(detail) => detail,
        None => return,
    };
    let entry = create_performance_entry(
        scope,
        "measure",
        &parsed.name,
        start_time,
        end_time - start_time,
        detail,
    );
    push_performance_entry(scope, performance, entry);
    rv.set(entry.into());
}

pub(in crate::context_bootstrap) fn performance_clear_measures_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(performance) = require_performance_receiver(scope, args.this()) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<PerformanceClearMeasuresArgs>(scope, &args) else {
        return;
    };
    let entries = performance_slot_array(scope, performance, PERFORMANCE_ENTRIES_SLOT)
        .expect("branded Performance receiver should retain its entries slot");
    let mut next = Vec::new();
    for index in 0..entries.length() {
        let Some(entry) = entries.get_index(scope, index) else {
            continue;
        };
        let Ok(entry) = v8::Local::<v8::Object>::try_from(entry) else {
            continue;
        };
        let is_measure = performance_entry_slot_string(scope, entry, PERFORMANCE_ENTRY_TYPE_SLOT)
            .as_deref()
            == Some("measure");
        let keep = if !is_measure {
            true
        } else {
            match (
                &parsed.name,
                performance_entry_slot_string(scope, entry, PERFORMANCE_ENTRY_NAME_SLOT),
            ) {
                (Some(expected), Some(actual)) => actual != *expected,
                (Some(_), None) => true,
                (None, _) => false,
            }
        };
        if keep {
            next.push(entry);
        }
    }
    let next = serialize_v8_iter_array(scope, next).unwrap_or_else(|| v8::Array::new(scope, 0));
    set_performance_slot_value(scope, performance, PERFORMANCE_ENTRIES_SLOT, next.into());
    rv.set_undefined();
}

fn parse_performance_measure_arguments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<ParsedPerformanceMeasure<'s>> {
    let second = (args.length() > 1).then(|| args.get(1));
    let options = second
        .filter(|value| !webidl::is_nullish(*value) && value.is_object())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());

    if let Some(options) = options {
        let options =
            match webidl::parse_dictionary_object::<PerformanceMeasureOptions>(scope, options) {
                Ok(options) => options,
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return None;
                }
            };
        let end = match options.end {
            Some(end) => match parse_measure_boundary_value(
                scope,
                end,
                webidl::Context::member("PerformanceMeasureOptions", "end"),
            ) {
                Ok(end) => Some(end),
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return None;
                }
            },
            None => None,
        };
        let start = match options.start {
            Some(start) => match parse_measure_boundary_value(
                scope,
                start,
                webidl::Context::member("PerformanceMeasureOptions", "start"),
            ) {
                Ok(start) => Some(start),
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return None;
                }
            },
            None => None,
        };
        let legacy_end = parse_optional_legacy_end(scope, args)?;
        let non_empty = options.detail.is_some()
            || options.duration.is_some()
            || start.is_some()
            || end.is_some();
        if !non_empty {
            return Some(ParsedPerformanceMeasure {
                start: None,
                duration: None,
                end: legacy_end.map(MeasureBoundary::Mark),
                detail: None,
            });
        }
        if legacy_end.is_some() {
            webidl::throw_type_error(
                scope,
                "If a non-empty PerformanceMeasureOptions object was passed, endMark must not be passed.",
            );
            return None;
        }
        if start.is_none() && end.is_none() {
            webidl::throw_type_error(
                scope,
                "A non-empty PerformanceMeasureOptions object must contain start or end.",
            );
            return None;
        }
        if start.is_some() && options.duration.is_some() && end.is_some() {
            webidl::throw_type_error(
                scope,
                "PerformanceMeasureOptions must not define start, duration, and end together.",
            );
            return None;
        }
        return Some(ParsedPerformanceMeasure {
            start,
            duration: options.duration,
            end,
            detail: options.detail,
        });
    }

    let start = match second.filter(|value| !webidl::is_nullish(*value)) {
        Some(start) => match parse_dom_string(
            scope,
            start,
            webidl::Context::argument("Performance.measure", 2),
        ) {
            Ok(start) => Some(MeasureBoundary::Mark(start)),
            Err(error) => {
                webidl::throw_error(scope, &error);
                return None;
            }
        },
        None => None,
    };
    let end = parse_optional_legacy_end(scope, args)?.map(MeasureBoundary::Mark);
    Some(ParsedPerformanceMeasure {
        start,
        duration: None,
        end,
        detail: None,
    })
}

fn parse_optional_legacy_end<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<Option<String>> {
    if args.length() <= 2 || webidl::is_nullish(args.get(2)) {
        return Some(None);
    }
    match parse_dom_string(
        scope,
        args.get(2),
        webidl::Context::argument("Performance.measure", 3),
    ) {
        Ok(end) => Some(Some(end)),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn parse_dom_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
) -> Result<String, webidl::WebIdlError> {
    webidl::convert::<webidl::DomString>(scope, value, context).map(Into::into)
}

fn parse_measure_boundary_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
) -> Result<MeasureBoundary, webidl::WebIdlError> {
    if value.is_number() {
        return webidl::convert::<webidl::Double>(scope, value, context)
            .map(|value| MeasureBoundary::Timestamp(value.0));
    }
    parse_dom_string(scope, value, context).map(MeasureBoundary::Mark)
}

fn resolve_measure_boundary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    measure_name: &str,
    boundary: &MeasureBoundary,
) -> Result<f64, ()> {
    match boundary {
        MeasureBoundary::Timestamp(timestamp) if *timestamp < 0.0 => {
            webidl::throw_type_error(
                scope,
                &format!("'{measure_name}' cannot have a negative time stamp."),
            );
            Err(())
        }
        MeasureBoundary::Timestamp(timestamp) => Ok(*timestamp),
        MeasureBoundary::Mark(name) => resolve_named_measure_boundary(scope, performance, name),
    }
}

fn resolve_named_measure_boundary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<f64, ()> {
    if !install::PERFORMANCE_TIMING_ATTRIBUTE_NAMES.contains(&name) {
        if let Some(start_time) = find_latest_performance_mark_start(scope, performance, name) {
            return Ok(start_time);
        }
        webidl::throw_dom_exception(
            scope,
            "SyntaxError",
            &format!("The mark '{name}' does not exist."),
        );
        return Err(());
    }

    if !install::is_window_performance(scope, performance) {
        webidl::throw_type_error(
            scope,
            "PerformanceTiming names can only be resolved in a Window global.",
        );
        return Err(());
    }
    let timing = match super::lazy_subobjects::ensure_performance_subobject(
        scope,
        performance,
        super::lazy_subobjects::PerformanceSubobject::Timing,
    ) {
        Ok(value) => match v8::Local::<v8::Object>::try_from(value) {
            Ok(timing) => timing,
            Err(_) => {
                webidl::throw_type_error(scope, "Failed to materialize PerformanceTiming.");
                return Err(());
            }
        },
        Err(error) => {
            webidl::throw_type_error(scope, &error.to_string());
            return Err(());
        }
    };
    let Some(key) = v8_string(scope, name) else {
        return Err(());
    };
    let value = timing
        .get(scope, key.into())
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite())
        .unwrap_or(0.0);
    if value == 0.0 {
        webidl::throw_dom_exception(
            scope,
            "InvalidAccessError",
            &format!("'{name}' is empty because the event has not happened or is unavailable."),
        );
        return Err(());
    }
    let navigation_start = timing
        .get(scope, v8str(scope, "navigationStart").into())
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite())
        .unwrap_or(0.0);
    Ok(value - navigation_start)
}

fn clone_user_timing_detail<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    detail: Option<v8::Local<'s, v8::Value>>,
) -> Option<Option<v8::Local<'s, v8::Value>>> {
    match detail {
        Some(detail) => structured_clone_value(scope, detail).map(Some),
        None => Some(None),
    }
}
