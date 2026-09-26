use super::*;
use crate::interfaces as web_api_interfaces;
use moli_v8_util::{
    callback_data_index_value, callback_data_item, get_private_value, throw_type_error, v8str,
};
use moli_webapi_declare::{ObjectLiteralDeclaration, WebApiFunctionTemplate, WebApiObject};

const TIMING_VALUES_SLOT: &str = "__moliPerformanceTimingValues";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::PerformanceNavigation)]
pub struct PerformanceNavigationObjectDeclaration {
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
#[webapi(interface = web_api_interfaces::PerformanceNavigation, enumerable)]
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
pub struct PerformanceTimingObjectDeclaration {
    #[webapi(slot = TIMING_VALUES_SLOT)]
    values: Vec<u64>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PerformanceTiming, enumerable)]
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

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::PerformanceNavigationTiming)]
pub struct PerformanceNavigationTimingSlotDeclaration {
    #[webapi(slot = NAV_UNLOAD_EVENT_START_SLOT, constructor_default)]
    unload_event_start: f64,
    #[webapi(slot = NAV_UNLOAD_EVENT_END_SLOT, constructor_default)]
    unload_event_end: f64,
    #[webapi(slot = NAV_DOM_INTERACTIVE_SLOT, constructor_default)]
    dom_interactive: f64,
    #[webapi(slot = NAV_DOM_CONTENT_LOADED_EVENT_START_SLOT, constructor_default)]
    dom_content_loaded_event_start: f64,
    #[webapi(slot = NAV_DOM_CONTENT_LOADED_EVENT_END_SLOT, constructor_default)]
    dom_content_loaded_event_end: f64,
    #[webapi(slot = NAV_DOM_COMPLETE_SLOT, constructor_default)]
    dom_complete: f64,
    #[webapi(slot = NAV_LOAD_EVENT_START_SLOT, constructor_default)]
    load_event_start: f64,
    #[webapi(slot = NAV_LOAD_EVENT_END_SLOT, constructor_default)]
    load_event_end: f64,
    #[webapi(slot = NAV_TYPE_SLOT)]
    navigation_type: &'static str,
    #[webapi(slot = NAV_REDIRECT_COUNT_SLOT, constructor_default)]
    redirect_count: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PerformanceNavigationTiming, enumerable)]
struct PerformanceNavigationTimingPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 0)
    )]
    unload_event_start: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 1)
    )]
    unload_event_end: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 2)
    )]
    dom_interactive: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 3)
    )]
    dom_content_loaded_event_start: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 4)
    )]
    dom_content_loaded_event_end: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 5)
    )]
    dom_complete: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 6)
    )]
    load_event_start: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 7)
    )]
    load_event_end: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 8)
    )]
    r#type: (),
    #[webapi(
        accessor_property,
        getter = performance_navigation_timing_attribute_getter_callback,
        data = callback_data_index_value(scope, 9)
    )]
    redirect_count: (),
    #[webapi(method, name = "toJSON", length = 0, callback = performance_navigation_timing_to_json_callback)]
    to_json: (),
}

#[derive(Default)]
pub struct PerformanceTimingSnapshot {
    pub navigation_start: u64,
    pub unload_event_start: u64,
    pub unload_event_end: u64,
    pub redirect_start: u64,
    pub redirect_end: u64,
    pub fetch_start: u64,
    pub domain_lookup_start: u64,
    pub domain_lookup_end: u64,
    pub connect_start: u64,
    pub connect_end: u64,
    pub secure_connection_start: u64,
    pub request_start: u64,
    pub response_start: u64,
    pub response_end: u64,
    pub dom_loading: u64,
    pub dom_interactive: u64,
    pub dom_content_loaded_event_start: u64,
    pub dom_content_loaded_event_end: u64,
    pub dom_complete: u64,
    pub load_event_start: u64,
    pub load_event_end: u64,
}

pub const NAV_UNLOAD_EVENT_START_SLOT: &str = "__moliPerformanceNavigationTimingUnloadEventStart";
pub const NAV_UNLOAD_EVENT_END_SLOT: &str = "__moliPerformanceNavigationTimingUnloadEventEnd";
pub const NAV_DOM_INTERACTIVE_SLOT: &str = "__moliPerformanceNavigationTimingDomInteractive";
pub const NAV_DOM_CONTENT_LOADED_EVENT_START_SLOT: &str =
    "__moliPerformanceNavigationTimingDomContentLoadedEventStart";
pub const NAV_DOM_CONTENT_LOADED_EVENT_END_SLOT: &str =
    "__moliPerformanceNavigationTimingDomContentLoadedEventEnd";
pub const NAV_DOM_COMPLETE_SLOT: &str = "__moliPerformanceNavigationTimingDomComplete";
pub const NAV_LOAD_EVENT_START_SLOT: &str = "__moliPerformanceNavigationTimingLoadEventStart";
pub const NAV_LOAD_EVENT_END_SLOT: &str = "__moliPerformanceNavigationTimingLoadEventEnd";
pub const NAV_TYPE_SLOT: &str = "__moliPerformanceNavigationTimingType";
pub const NAV_REDIRECT_COUNT_SLOT: &str = "__moliPerformanceNavigationTimingRedirectCount";

pub const PERFORMANCE_NAVIGATION_TIMING_ATTRIBUTE_SLOTS: &[&str] = &[
    NAV_UNLOAD_EVENT_START_SLOT,
    NAV_UNLOAD_EVENT_END_SLOT,
    NAV_DOM_INTERACTIVE_SLOT,
    NAV_DOM_CONTENT_LOADED_EVENT_START_SLOT,
    NAV_DOM_CONTENT_LOADED_EVENT_END_SLOT,
    NAV_DOM_COMPLETE_SLOT,
    NAV_LOAD_EVENT_START_SLOT,
    NAV_LOAD_EVENT_END_SLOT,
    NAV_TYPE_SLOT,
    NAV_REDIRECT_COUNT_SLOT,
];

pub const PERFORMANCE_TIMING_ATTRIBUTE_NAMES: &[&str] = &[
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

pub const PERFORMANCE_NAVIGATION_JSON_KEYS: &[&str] = &["type", "redirectCount"];

pub const PERFORMANCE_NAVIGATION_ATTRIBUTE_SLOTS: &[&str] = &[
    PERFORMANCE_NAVIGATION_TYPE_SLOT,
    PERFORMANCE_NAVIGATION_REDIRECT_COUNT_SLOT,
];

pub fn legacy_timing_epoch_millis(epoch_millis: f64) -> u64 {
    // PerformanceTiming is the legacy Navigation Timing 1 surface. Blink
    // projects its pseudo-wall timestamps through uint64_t while keeping
    // PerformanceNavigationTiming as a DOMHighResTimeStamp.
    epoch_millis as u64
}

pub fn performance_navigation_timing_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        PERFORMANCE_NAVIGATION_TIMING_ATTRIBUTE_SLOTS,
        "PerformanceNavigationTiming attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    rv.set(
        performance_entry_slot_value(scope, args.this(), slot)
            .unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

pub fn performance_navigation_attribute_getter_callback<'s>(
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

fn performance_timing_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    timing: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    get_private_value(scope, timing, TIMING_VALUES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .expect("native PerformanceTiming should retain its values")
}

pub fn performance_timing_value<'s>(
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

pub fn set_performance_timing_value<'s>(
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

pub fn performance_timing_to_json_callback<'s>(
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

pub fn performance_navigation_to_json_callback<'s>(
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

pub fn performance_navigation_timing_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(output) = super::entries::resource_timing_json_snapshot(scope, args.this()) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    // Inherited and navigation-only attributes are serialized from the same
    // native slots used by their getters, without consulting author properties.
    for (name, slot) in [
        "unloadEventStart",
        "unloadEventEnd",
        "domInteractive",
        "domContentLoadedEventStart",
        "domContentLoadedEventEnd",
        "domComplete",
        "loadEventStart",
        "loadEventEnd",
        "type",
        "redirectCount",
    ]
    .iter()
    .zip(PERFORMANCE_NAVIGATION_TIMING_ATTRIBUTE_SLOTS)
    {
        let value = get_private_value(scope, args.this(), slot)
            .expect("native PerformanceNavigationTiming should retain its attributes");
        let _ = output.create_data_property(scope, v8str(scope, name).into(), value);
    }
    rv.set(output.into());
}

impl PerformanceTimingSnapshot {
    pub fn new(time_origin: f64) -> Self {
        let time_origin = legacy_timing_epoch_millis(time_origin);
        Self {
            navigation_start: time_origin,
            unload_event_start: 0,
            unload_event_end: 0,
            redirect_start: 0,
            redirect_end: 0,
            fetch_start: time_origin,
            domain_lookup_start: time_origin,
            domain_lookup_end: time_origin,
            connect_start: time_origin,
            connect_end: time_origin,
            secure_connection_start: 0,
            request_start: time_origin,
            response_start: time_origin,
            response_end: time_origin,
            dom_loading: time_origin,
            dom_interactive: 0,
            dom_content_loaded_event_start: 0,
            dom_content_loaded_event_end: 0,
            dom_complete: 0,
            load_event_start: 0,
            load_event_end: 0,
        }
    }
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

pub fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface {
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
        "PerformanceNavigationTiming" => {
            PerformanceNavigationTimingPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            )
        }
        _ => {}
    }
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct PerformanceJsonSnapshotDeclaration<'scope> {
    time_origin: Option<v8::Local<'scope, v8::Value>>,
    timing: Option<v8::Local<'scope, v8::Value>>,
    navigation: Option<v8::Local<'scope, v8::Value>>,
}

pub fn new_performance_json_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    time_origin: Option<v8::Local<'s, v8::Value>>,
    timing: Option<v8::Local<'s, v8::Value>>,
    navigation: Option<v8::Local<'s, v8::Value>>,
) -> v8::Local<'s, v8::Object> {
    PerformanceJsonSnapshotDeclaration {
        time_origin,
        timing,
        navigation,
    }
    .bind(scope)
    .expect("Performance toJSON snapshot declaration should bind")
}
