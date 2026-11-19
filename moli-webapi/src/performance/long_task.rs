use moli_v8_util::{
    callback_data_index_value, callback_data_item, get_private_value, throw_type_error, v8_string,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use crate::interfaces as web_api_interfaces;

use super::entries::{
    create_performance_entry, performance_entry_base_attribute_getter_callback,
    performance_entry_json_snapshot,
};

const ATTRIBUTION: &str = "__moliLongTaskAttribution";
const CONTAINER_TYPE: &str = "__moliTaskAttributionContainerType";
const CONTAINER_SRC: &str = "__moliTaskAttributionContainerSrc";
const CONTAINER_ID: &str = "__moliTaskAttributionContainerId";
const CONTAINER_NAME: &str = "__moliTaskAttributionContainerName";

const CONTAINER_SLOTS: [(&str, &str); 4] = [
    ("containerType", CONTAINER_TYPE),
    ("containerSrc", CONTAINER_SRC),
    ("containerId", CONTAINER_ID),
    ("containerName", CONTAINER_NAME),
];

#[derive(Clone, Debug, WebApiObject)]
#[webapi(interface = web_api_interfaces::TaskAttributionTiming)]
pub struct TaskAttribution {
    #[webapi(slot = CONTAINER_TYPE)]
    pub container_type: &'static str,
    #[webapi(slot = CONTAINER_SRC)]
    pub container_src: String,
    #[webapi(slot = CONTAINER_ID)]
    pub container_id: String,
    #[webapi(slot = CONTAINER_NAME)]
    pub container_name: String,
}

impl Default for TaskAttribution {
    fn default() -> Self {
        Self {
            container_type: "window",
            container_src: String::new(),
            container_id: String::new(),
            container_name: String::new(),
        }
    }
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::PerformanceLongTaskTiming)]
struct LongTaskSlots<'scope> {
    #[webapi(slot = ATTRIBUTION)]
    attribution: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PerformanceLongTaskTiming, enumerable)]
struct LongTaskPrototype {
    #[webapi(
        accessor_property,
        getter = performance_entry_base_attribute_getter_callback,
        data = callback_data_index_value(scope, 3)
    )]
    start_time: (),
    #[webapi(
        accessor_property,
        getter = performance_entry_base_attribute_getter_callback,
        data = callback_data_index_value(scope, 4)
    )]
    duration: (),
    #[webapi(
        accessor_property,
        getter = performance_entry_base_attribute_getter_callback,
        data = callback_data_index_value(scope, 1)
    )]
    name: (),
    #[webapi(
        accessor_property,
        getter = performance_entry_base_attribute_getter_callback,
        data = callback_data_index_value(scope, 2)
    )]
    entry_type: (),
    #[webapi(accessor_property, getter = attribution_getter)]
    attribution: (),
    #[webapi(method, name = "toJSON", length = 0, callback = long_task_to_json)]
    to_json: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TaskAttributionTiming, enumerable)]
struct TaskAttributionPrototype {
    #[webapi(
        accessor_property,
        getter = performance_entry_base_attribute_getter_callback,
        data = callback_data_index_value(scope, 3)
    )]
    start_time: (),
    #[webapi(
        accessor_property,
        getter = performance_entry_base_attribute_getter_callback,
        data = callback_data_index_value(scope, 4)
    )]
    duration: (),
    #[webapi(
        accessor_property,
        getter = performance_entry_base_attribute_getter_callback,
        data = callback_data_index_value(scope, 1)
    )]
    name: (),
    #[webapi(
        accessor_property,
        getter = performance_entry_base_attribute_getter_callback,
        data = callback_data_index_value(scope, 2)
    )]
    entry_type: (),
    #[webapi(accessor_property, getter = container_getter, data = callback_data_index_value(scope, 0))]
    container_type: (),
    #[webapi(accessor_property, getter = container_getter, data = callback_data_index_value(scope, 1))]
    container_src: (),
    #[webapi(accessor_property, getter = container_getter, data = callback_data_index_value(scope, 2))]
    container_id: (),
    #[webapi(accessor_property, getter = container_getter, data = callback_data_index_value(scope, 3))]
    container_name: (),
    #[webapi(method, name = "toJSON", length = 0, callback = task_attribution_to_json)]
    to_json: (),
}

pub fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
    interface: &str,
) {
    match interface {
        "PerformanceLongTaskTiming" => {
            LongTaskPrototype::initialize_prototype_template(scope, prototype)
        }
        "TaskAttributionTiming" => {
            TaskAttributionPrototype::initialize_prototype_template(scope, prototype)
        }
        _ => {}
    }
}

pub fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    performance: v8::Local<'s, v8::Object>,
    name: &'static str,
    start_time: f64,
    duration: f64,
    attribution: TaskAttribution,
) -> v8::Local<'s, v8::Object> {
    let context = performance
        .get_creation_context(scope)
        .expect("native Performance must retain its relevant Realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let container = create_performance_entry(
        scope,
        performance,
        "taskattribution",
        "unknown".into(),
        0.0,
        0.0,
        None,
    );
    attribution
        .initialize(scope, container)
        .expect("task attribution slots should initialize");
    let attribution = v8::Array::new_with_elements(scope, &[container.into()]);
    assert_eq!(
        attribution.set_integrity_level(scope, v8::IntegrityLevel::Frozen),
        Some(true)
    );
    let entry = create_performance_entry(
        scope,
        performance,
        "longtask",
        name.into(),
        start_time,
        duration,
        None,
    );
    LongTaskSlots {
        attribution: attribution.into(),
    }
    .initialize(scope, entry)
    .expect("long task slots should initialize");
    entry
}

fn attribution_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(value) = get_private_value(scope, args.this(), ATTRIBUTION) {
        rv.set(value);
    } else {
        throw_type_error(scope, "Illegal invocation");
    }
}

fn container_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(value) =
        callback_data_item(scope, &args, &CONTAINER_SLOTS, "task attribution slots")
            .and_then(|(_, slot)| get_private_value(scope, args.this(), slot))
    {
        rv.set(value);
    } else {
        throw_type_error(scope, "Illegal invocation");
    }
}

fn task_attribution_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let snapshot = performance_entry_json_snapshot(scope, entry)?;
    for (name, slot) in CONTAINER_SLOTS {
        let key = v8_string(scope, name)?;
        let value = get_private_value(scope, entry, slot)?;
        snapshot
            .create_data_property(scope, key.into(), value)
            .filter(|ok| *ok)?;
    }
    Some(snapshot)
}

fn task_attribution_to_json<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(snapshot) = task_attribution_snapshot(scope, args.this()) {
        rv.set(snapshot.into());
    } else {
        throw_type_error(scope, "Illegal invocation");
    }
}

fn long_task_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let snapshot = performance_entry_json_snapshot(scope, entry)?;
    let attribution = get_private_value(scope, entry, ATTRIBUTION)?;
    let key = v8_string(scope, "attribution")?;
    snapshot
        .create_data_property(scope, key.into(), attribution)
        .filter(|ok| *ok)?;
    Some(snapshot)
}

fn long_task_to_json<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(snapshot) = long_task_snapshot(scope, args.this()) {
        rv.set(snapshot.into());
    } else {
        throw_type_error(scope, "Illegal invocation");
    }
}
