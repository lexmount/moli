use super::entries::{
    create_performance_entry, find_latest_performance_mark_start,
    initialize_performance_entry_slots,
};
use super::*;
use crate::context_bootstrap::{structured_deserialize_value, structured_serialize_value};
use crate::util::serialize_v8_iter_array;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Performance.mark")]
struct PerformanceMarkArgs {
    #[webidl(required, converter = "raw")]
    name: PerformanceEntryName,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PerformanceMark")]
struct PerformanceMarkConstructorArgs {
    #[webidl(required, converter = "raw")]
    name: PerformanceEntryName,
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
    #[webidl(converter = "raw")]
    name: Option<PerformanceEntryName>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Performance.measure")]
struct PerformanceMeasureArgs<'s> {
    #[webidl(required, converter = "raw")]
    name: PerformanceEntryName,
    #[webidl(
        converter = "raw",
        default = MeasureStartOrOptions::Dictionary(PerformanceMeasureOptions::default())
    )]
    start_or_options: MeasureStartOrOptions<'s>,
    #[webidl(converter = "raw")]
    end_mark: Option<PerformanceEntryName>,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "PerformanceMeasureOptions")]
struct PerformanceMeasureOptions<'s> {
    #[webidl(converter = "raw")]
    detail: Option<v8::Local<'s, v8::Value>>,
    #[webidl(converter = "double")]
    duration: Option<f64>,
    #[webidl(converter = "raw")]
    end: Option<MeasureBoundary>,
    #[webidl(converter = "raw")]
    start: Option<MeasureBoundary>,
}

enum MeasureStartOrOptions<'s> {
    Mark(PerformanceEntryName),
    Dictionary(PerformanceMeasureOptions<'s>),
}

enum MeasureBoundary {
    Mark(PerformanceEntryName),
    Timestamp(f64),
}

impl<'s> webidl::WebIdlConverter<'s> for MeasureStartOrOptions<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        if value.is_object() || value.is_null_or_undefined() {
            webidl::parse_dictionary::<PerformanceMeasureOptions>(scope, value, context)
                .map(|options| Self::Dictionary(options.unwrap_or_default()))
        } else {
            webidl::convert::<PerformanceEntryName>(scope, value, context).map(Self::Mark)
        }
    }
}

impl<'s> webidl::WebIdlConverter<'s> for MeasureBoundary {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        if value.is_number() {
            webidl::convert::<webidl::Double>(scope, value, context)
                .map(|time| Self::Timestamp(time.0))
        } else {
            webidl::convert::<PerformanceEntryName>(scope, value, context).map(Self::Mark)
        }
    }
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
    #[webidl(converter = "raw")]
    name: Option<PerformanceEntryName>,
}

pub(in crate::context_bootstrap) fn performance_mark_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let performance = args.this();
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
    let entry = create_performance_entry(
        scope,
        performance,
        "mark",
        parsed.name,
        start_time,
        0.0,
        detail,
    );
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
        parsed.name,
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
    name: &PerformanceEntryName,
    options: PerformanceMarkOptions<'s>,
) -> Option<(f64, Option<v8::Local<'s, v8::Value>>)> {
    let start_time = options
        .start_time
        .unwrap_or_else(|| performance_now(scope, performance));
    if start_time < 0.0 {
        webidl::throw_type_error(
            scope,
            &format!("'{name}' cannot have a negative start time."),
        );
        return None;
    }
    if install::is_window_performance(scope, performance)
        && name
            .as_str()
            .is_some_and(|name| install::PERFORMANCE_TIMING_ATTRIBUTE_NAMES.contains(&name))
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
    clone_user_timing_detail(scope, performance, options.detail).map(|detail| (start_time, detail))
}

pub(in crate::context_bootstrap) fn performance_clear_marks_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let performance = args.this();
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
            match (&parsed.name, PerformanceEntryName::from_entry(scope, entry)) {
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
    let performance = args.this();
    let Some(parsed) = webidl::parse_args::<PerformanceMeasureArgs>(scope, &args) else {
        return;
    };
    let Some(parsed_measure) =
        prepare_performance_measure(scope, parsed.start_or_options, parsed.end_mark)
    else {
        return;
    };
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
        None => performance_now(scope, performance),
    };
    let (start_time, end_time) = match parsed_measure.duration {
        Some(duration) if parsed_measure.start.is_none() => (end - duration, end),
        Some(duration) => (start, start + duration),
        None => (start, end),
    };
    let detail = match clone_user_timing_detail(scope, performance, parsed_measure.detail) {
        Some(detail) => detail,
        None => return,
    };
    let entry = create_performance_entry(
        scope,
        performance,
        "measure",
        parsed.name,
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
    let performance = args.this();
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
            match (&parsed.name, PerformanceEntryName::from_entry(scope, entry)) {
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

fn prepare_performance_measure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    start_or_options: MeasureStartOrOptions<'s>,
    end_mark: Option<PerformanceEntryName>,
) -> Option<ParsedPerformanceMeasure<'s>> {
    let options = match start_or_options {
        MeasureStartOrOptions::Dictionary(options) => options,
        MeasureStartOrOptions::Mark(start) => {
            return Some(ParsedPerformanceMeasure {
                start: Some(MeasureBoundary::Mark(start)),
                duration: None,
                end: end_mark.map(MeasureBoundary::Mark),
                detail: None,
            });
        }
    };
    let PerformanceMeasureOptions {
        detail,
        duration,
        end,
        start,
    } = options;
    let non_empty = detail.is_some() || duration.is_some() || start.is_some() || end.is_some();
    if !non_empty {
        return Some(ParsedPerformanceMeasure {
            start: None,
            duration: None,
            end: end_mark.map(MeasureBoundary::Mark),
            detail: None,
        });
    }
    if end_mark.is_some() {
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
    if start.is_some() && duration.is_some() && end.is_some() {
        webidl::throw_type_error(
            scope,
            "PerformanceMeasureOptions must not define start, duration, and end together.",
        );
        return None;
    }
    Some(ParsedPerformanceMeasure {
        start,
        duration,
        end,
        detail,
    })
}

fn resolve_measure_boundary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    measure_name: &PerformanceEntryName,
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
    name: &PerformanceEntryName,
) -> Result<f64, ()> {
    let Some(timing_name) = name
        .as_str()
        .filter(|name| install::PERFORMANCE_TIMING_ATTRIBUTE_NAMES.contains(name))
    else {
        if let Some(start_time) = find_latest_performance_mark_start(scope, performance, name) {
            return Ok(start_time);
        }
        webidl::throw_dom_exception(
            scope,
            "SyntaxError",
            &format!("The mark '{name}' does not exist."),
        );
        return Err(());
    };

    if !install::is_window_performance(scope, performance) {
        webidl::throw_type_error(
            scope,
            "PerformanceTiming names can only be resolved in a Window global.",
        );
        return Err(());
    }
    if timing_name == "navigationStart" {
        return Ok(0.0);
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
    let value = install::performance_timing_value(scope, timing, timing_name)
        .number_value(scope)
        .expect("native PerformanceTiming attributes should be numeric");
    if value == 0.0 {
        webidl::throw_dom_exception(
            scope,
            "InvalidAccessError",
            &format!("'{name}' is empty because the event has not happened or is unavailable."),
        );
        return Err(());
    }
    let navigation_start = install::performance_timing_value(scope, timing, "navigationStart")
        .number_value(scope)
        .expect("native navigationStart should be numeric");
    Ok(value - navigation_start)
}

fn clone_user_timing_detail<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    detail: Option<v8::Local<'s, v8::Value>>,
) -> Option<Option<v8::Local<'s, v8::Value>>> {
    match detail {
        Some(detail) => {
            // Serialization can invoke getters or throw DataCloneError. Keep
            // those exceptions in the method Realm; only the clone's native
            // objects and prototypes are created in the receiver's Realm.
            let payload = structured_serialize_value(scope, detail)?;
            let context = performance.get_creation_context(scope)?;
            let scope = &mut v8::ContextScope::new(scope, context);
            structured_deserialize_value(scope, &payload).map(Some)
        }
        None => Some(None),
    }
}
