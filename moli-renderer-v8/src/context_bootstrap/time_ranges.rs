//! Immutable time-range snapshots, independent of a media decoder.

use crate::{util::get_private_value, web_api_interfaces, webidl};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const RANGES_SLOT: &str = "__moliTimeRanges";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::TimeRanges, require_prototype)]
struct TimeRangesObjectDeclaration<'scope> {
    #[webapi(slot = RANGES_SLOT)]
    ranges: v8::Local<'scope, v8::Array>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TimeRanges, enumerable, receiver)]
struct TimeRangesPrototypeDeclaration {
    #[webapi(accessor_property, getter = length_getter)]
    length: (),
    #[webapi(method, length = 1, callback = start_callback)]
    start: (),
    #[webapi(method, length = 1, callback = end_callback)]
    end: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TimeRanges.start")]
struct StartArgs {
    #[webidl(required)]
    index: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TimeRanges.end")]
struct EndArgs {
    #[webidl(required)]
    index: u32,
}

/// Copies native, normalized intervals into a new snapshot in the current realm.
/// Adjacent/overlapping intervals must be merged by the producing media backend.
pub(crate) fn new_time_ranges_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    ranges: &[(f64, f64)],
) -> v8::Local<'s, v8::Object> {
    debug_assert!(ranges.iter().all(|(start, end)| start <= end));
    debug_assert!(ranges.windows(2).all(|pair| pair[0].1 < pair[1].0));
    let values: Vec<v8::Local<'s, v8::Value>> = ranges
        .iter()
        .flat_map(|&(start, end)| {
            [
                v8::Number::new(scope, start).into(),
                v8::Number::new(scope, end).into(),
            ]
        })
        .collect();
    let values = v8::Array::new_with_elements(scope, &values);
    TimeRangesObjectDeclaration::new(values)
        .bind(scope)
        .expect("TimeRanges snapshot binds")
}

pub(in crate::context_bootstrap) fn install_time_ranges_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name == "TimeRanges" {
        TimeRangesPrototypeDeclaration::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        );
    }
}

fn native_ranges<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    let target = moli_webapi_declare::web_api_object_target(scope, receiver)
        .expect("validated TimeRanges receiver");
    v8::Local::<v8::Array>::try_from(
        get_private_value(scope, target, RANGES_SLOT).expect("TimeRanges snapshot slot"),
    )
    .expect("TimeRanges native interval array")
}

fn length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_uint32(native_ranges(scope, args.this()).length() / 2);
}

fn start_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<StartArgs>(scope, &args) else {
        return;
    };
    endpoint(scope, args.this(), parsed.index, 0, rv);
}

fn end_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<EndArgs>(scope, &args) else {
        return;
    };
    endpoint(scope, args.this(), parsed.index, 1, rv);
}

fn endpoint<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    index: u32,
    offset: u32,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let ranges = native_ranges(scope, receiver);
    if index >= ranges.length() / 2 {
        super::constructors::throw_dom_exception_value(
            scope,
            "The index is outside the time ranges.",
            "IndexSizeError",
        );
        return;
    }
    // Check the native size before multiplying, including for wrapped uint32 indices.
    rv.set(
        ranges
            .get_index(scope, index * 2 + offset)
            .expect("native endpoint"),
    );
}
