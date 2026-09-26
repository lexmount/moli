use super::*;

const TIMING_VALUES_SLOT: &str = "__moliPerformanceTimingValues";

pub(in crate::context_bootstrap::performance_runtime) const PERFORMANCE_TIMING_ATTRIBUTE_NAMES:
    &[&str] = &[
    "navigationStart",
    "unloadEventStart",
    "unloadEventEnd",
    "redirectStart",
    "redirectEnd",
    "fetchStart",
    "domainLookupStart",
    "domainLookupEnd",
    "connectStart",
    "connectEnd",
    "secureConnectionStart",
    "requestStart",
    "responseStart",
    "responseEnd",
    "domLoading",
    "domInteractive",
    "domContentLoadedEventStart",
    "domContentLoadedEventEnd",
    "domComplete",
    "loadEventStart",
    "loadEventEnd",
];

const PERFORMANCE_NAVIGATION_JSON_KEYS: &[&str] = &["type", "redirectCount"];

const PERFORMANCE_NAVIGATION_ATTRIBUTE_SLOTS: &[&str] = &[
    PERFORMANCE_NAVIGATION_TYPE_SLOT,
    PERFORMANCE_NAVIGATION_REDIRECT_COUNT_SLOT,
];

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::PerformanceNavigation)]
pub(super) struct PerformanceNavigationObjectDeclaration {
    #[webapi(slot = PERFORMANCE_NAVIGATION_TYPE_SLOT)]
    navigation_type: f64,

    #[webapi(slot = PERFORMANCE_NAVIGATION_REDIRECT_COUNT_SLOT)]
    redirect_count: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PerformanceNavigation)]
struct PerformanceNavigationConstantsDeclaration {
    #[webapi(constant = "TYPE_NAVIGATE", value = 0.0)]
    type_navigate: (),

    #[webapi(constant = "TYPE_RELOAD", value = 1.0)]
    type_reload: (),

    #[webapi(constant = "TYPE_BACK_FORWARD", value = 2.0)]
    type_back_forward: (),

    #[webapi(constant = "TYPE_RESERVED", value = 255.0)]
    type_reserved: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PerformanceNavigation, enumerable, receiver)]
struct PerformanceNavigationPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = performance_navigation_attribute_getter_callback,
        data = callback_data_index_value(scope, 0)
    )]
    r#type: (),

    #[webapi(
        accessor_property,
        getter = performance_navigation_attribute_getter_callback,
        data = callback_data_index_value(scope, 1)
    )]
    redirect_count: (),

    #[webapi(method, name = "toJSON", length = 0, callback = performance_navigation_to_json_callback)]
    to_json: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::PerformanceTiming)]
pub(super) struct PerformanceTimingObjectDeclaration {
    #[webapi(slot = TIMING_VALUES_SLOT)]
    values: Vec<u64>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PerformanceTiming, enumerable, receiver)]
struct PerformanceTimingPrototypeDeclaration {
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 0))]
    navigation_start: (),
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 1))]
    unload_event_start: (),
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 2))]
    unload_event_end: (),
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 3))]
    redirect_start: (),
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 4))]
    redirect_end: (),
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 5))]
    fetch_start: (),
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 6))]
    domain_lookup_start: (),
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 7))]
    domain_lookup_end: (),
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 8))]
    connect_start: (),
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 9))]
    connect_end: (),
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 10))]
    secure_connection_start: (),
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 11))]
    request_start: (),
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 12))]
    response_start: (),
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 13))]
    response_end: (),
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 14))]
    dom_loading: (),
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 15))]
    dom_interactive: (),
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 16))]
    dom_content_loaded_event_start: (),
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 17))]
    dom_content_loaded_event_end: (),
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 18))]
    dom_complete: (),
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 19))]
    load_event_start: (),
    #[webapi(accessor_property, getter = performance_timing_attribute_getter_callback, data = callback_data_index_value(scope, 20))]
    load_event_end: (),
    #[webapi(method, name = "toJSON", length = 0, callback = performance_timing_to_json_callback)]
    to_json: (),
}

impl From<PerformanceTimingSnapshot> for PerformanceTimingObjectDeclaration {
    fn from(snapshot: PerformanceTimingSnapshot) -> Self {
        Self {
            values: vec![
                snapshot.navigation_start,
                snapshot.unload_event_start,
                snapshot.unload_event_end,
                snapshot.redirect_start,
                snapshot.redirect_end,
                snapshot.fetch_start,
                snapshot.domain_lookup_start,
                snapshot.domain_lookup_end,
                snapshot.connect_start,
                snapshot.connect_end,
                snapshot.secure_connection_start,
                snapshot.request_start,
                snapshot.response_start,
                snapshot.response_end,
                snapshot.dom_loading,
                snapshot.dom_interactive,
                snapshot.dom_content_loaded_event_start,
                snapshot.dom_content_loaded_event_end,
                snapshot.dom_complete,
                snapshot.load_event_start,
                snapshot.load_event_end,
            ],
        }
    }
}

pub(super) fn install_legacy_performance_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "PerformanceTiming" => {
            PerformanceTimingPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "PerformanceNavigation" => {
            PerformanceNavigationConstantsDeclaration::initialize_template(scope, template);
            PerformanceNavigationConstantsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
            PerformanceNavigationPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}

fn performance_timing_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    timing: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    get_private_value(scope, timing, TIMING_VALUES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .expect("native PerformanceTiming should retain its values")
}

pub(in crate::context_bootstrap::performance_runtime) fn performance_timing_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    timing: v8::Local<'s, v8::Object>,
    name: &str,
) -> v8::Local<'s, v8::Value> {
    let index = PERFORMANCE_TIMING_ATTRIBUTE_NAMES
        .iter()
        .position(|key| *key == name)
        .expect("legacy timing attribute should be registered");
    performance_timing_values(scope, timing)
        .get_index(scope, index as u32)
        .expect("native PerformanceTiming should have every timestamp")
}

pub(super) fn set_performance_timing_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    timing: v8::Local<'s, v8::Object>,
    name: &str,
    value: f64,
) {
    let index = PERFORMANCE_TIMING_ATTRIBUTE_NAMES
        .iter()
        .position(|key| *key == name)
        .expect("legacy timing attribute should be registered");
    let values = performance_timing_values(scope, timing);
    let value = v8::Number::new(scope, value);
    // Public expandos, freezing, and prototype mutation do not affect lifecycle
    // timestamps or the internal values consumed by User Timing.
    let _ = values.set_index(scope, index as u32, value.into());
}

fn performance_timing_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        PERFORMANCE_TIMING_ATTRIBUTE_NAMES,
        "PerformanceTiming attributes",
    ) else {
        return;
    };
    rv.set(performance_timing_value(scope, args.this(), name));
}

fn performance_timing_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let output = ObjectLiteralDeclaration::bind(scope);
    for name in PERFORMANCE_TIMING_ATTRIBUTE_NAMES {
        let value = performance_timing_value(scope, args.this(), name);
        output.set_string_property(scope, name, value);
    }
    rv.set(output.into_object().into());
}

fn performance_navigation_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let output = ObjectLiteralDeclaration::bind(scope);
    for (name, slot) in PERFORMANCE_NAVIGATION_JSON_KEYS
        .iter()
        .zip(PERFORMANCE_NAVIGATION_ATTRIBUTE_SLOTS)
    {
        let value = get_private_value(scope, args.this(), slot)
            .expect("native PerformanceNavigation should retain its attributes");
        output.set_string_property(scope, name, value);
    }
    rv.set(output.into_object().into());
}

fn performance_navigation_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        PERFORMANCE_NAVIGATION_ATTRIBUTE_SLOTS,
        "PerformanceNavigation attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    rv.set(
        get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::undefined(scope).into()),
    );
}
