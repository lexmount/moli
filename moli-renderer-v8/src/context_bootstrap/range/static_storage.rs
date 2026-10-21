use super::object::range_backing_object;
use super::*;
use crate::native_bridge::RangeBoundarySide;
use crate::util::{get_private_value, set_private_value};
use crate::web_api_interfaces;
use crate::webidl::{self, WebIdlDictionarySerialize};
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::AbstractRange)]
struct StaticRangeBoundaryStorageDeclaration {}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "StaticRangeInit", snapshot)]
pub(in crate::context_bootstrap) struct StaticRangeInitMembers<'s> {
    #[webidl(required, name = "startContainer", with = static_range_node_member)]
    #[webidl(snapshot_slot = STATIC_RANGE_START_CONTAINER_STORAGE_KEY)]
    pub(in crate::context_bootstrap) start_container: v8::Local<'s, v8::Object>,
    #[webidl(required, name = "startOffset")]
    #[webidl(snapshot_slot = STATIC_RANGE_START_OFFSET_STORAGE_KEY)]
    pub(in crate::context_bootstrap) start_offset: u32,
    #[webidl(required, name = "endContainer", with = static_range_node_member)]
    #[webidl(snapshot_slot = STATIC_RANGE_END_CONTAINER_STORAGE_KEY)]
    pub(in crate::context_bootstrap) end_container: v8::Local<'s, v8::Object>,
    #[webidl(required, name = "endOffset")]
    #[webidl(snapshot_slot = STATIC_RANGE_END_OFFSET_STORAGE_KEY)]
    pub(in crate::context_bootstrap) end_offset: u32,
}

fn static_range_node_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    let context = webidl::Context::member("StaticRangeInit", name);
    let Some(value) = webidl::property_result(scope, object, name, context)? else {
        return Err(webidl::WebIdlError::missing_required(context));
    };
    if value.is_undefined() {
        return Err(webidl::WebIdlError::missing_required(context));
    }
    let Ok(node) = v8::Local::<v8::Object>::try_from(value) else {
        return Err(webidl::WebIdlError::custom_message(
            "StaticRangeInit member is not a Node.",
        ));
    };
    if object_number_property(scope, node, "nodeType").is_none() {
        return Err(webidl::WebIdlError::custom_message(
            "StaticRangeInit member is not a Node.",
        ));
    }
    Ok(node)
}

pub(in crate::context_bootstrap) fn initialize_static_range_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    start_container: v8::Local<'s, v8::Object>,
    start_offset: u32,
    end_container: v8::Local<'s, v8::Object>,
    end_offset: u32,
) {
    StaticRangeInitMembers {
        start_container,
        start_offset,
        end_container,
        end_offset,
    }
    .initialize_snapshot(scope, range)
    .expect("StaticRange boundaries");
    StaticRangeBoundaryStorageDeclaration::new()
        .initialize(scope, range)
        .expect("StaticRange boundary declaration should initialize object");
}

pub(in crate::context_bootstrap) fn set_static_range_boundary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    side: RangeBoundarySide,
    container: v8::Local<'s, v8::Object>,
    offset: u32,
) {
    let Some(range) = range_backing_object(scope, range) else {
        return;
    };
    set_private_value(
        scope,
        range,
        static_range_container_key_for_boundary_side(side),
        container.into(),
    );
    set_static_range_boundary_offset(scope, range, side, offset);
}

pub(in crate::context_bootstrap) fn static_range_boundary_container_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    side: RangeBoundarySide,
) -> Option<v8::Local<'s, v8::Object>> {
    let range = range_backing_object(scope, range)?;
    get_private_value(
        scope,
        range,
        static_range_container_key_for_boundary_side(side),
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(in crate::context_bootstrap) fn static_range_boundary_offset<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    side: RangeBoundarySide,
) -> f64 {
    let Some(range) = range_backing_object(scope, range) else {
        return 0.0;
    };
    get_private_value(
        scope,
        range,
        static_range_offset_key_for_boundary_side(side),
    )
    .and_then(|value| value.number_value(scope))
    .unwrap_or(0.0)
}

fn set_static_range_boundary_offset<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    side: RangeBoundarySide,
    offset: u32,
) {
    let value = v8::Number::new(scope, offset as f64);
    set_private_value(
        scope,
        range,
        static_range_offset_key_for_boundary_side(side),
        value.into(),
    );
}

fn static_range_container_key_for_boundary_side(side: RangeBoundarySide) -> &'static str {
    match side {
        RangeBoundarySide::Start => STATIC_RANGE_START_CONTAINER_STORAGE_KEY,
        RangeBoundarySide::End => STATIC_RANGE_END_CONTAINER_STORAGE_KEY,
    }
}

fn static_range_offset_key_for_boundary_side(side: RangeBoundarySide) -> &'static str {
    match side {
        RangeBoundarySide::Start => STATIC_RANGE_START_OFFSET_STORAGE_KEY,
        RangeBoundarySide::End => STATIC_RANGE_END_OFFSET_STORAGE_KEY,
    }
}
