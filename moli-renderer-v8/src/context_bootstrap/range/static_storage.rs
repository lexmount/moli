use super::object::range_backing_object;
use crate::native_bridge::RangeBoundarySide;
use crate::util::{get_private_value, set_private_value};
use moli_webapi::static_range::{
    STATIC_RANGE_END_CONTAINER_STORAGE_KEY, STATIC_RANGE_END_OFFSET_STORAGE_KEY,
    STATIC_RANGE_START_CONTAINER_STORAGE_KEY, STATIC_RANGE_START_OFFSET_STORAGE_KEY,
};

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
