//! SVG2 magnification and the SameObject DOMPoint representing currentTranslate.
use crate::{
    native_bridge::node_runtime_and_handle_from_object_or_detached,
    util::{get_private_value, set_private_value},
    webidl,
};

use super::super::geometry_runtime::{
    build_dom_point_object, dom_point_coordinates, set_dom_point_coordinates,
    set_svg_point_read_only,
};

const CURRENT_TRANSLATE: &str = "__moliSvgCurrentTranslate";
const TRANSLATE_OWNER: &str = "__moliSvgCurrentTranslateOwner";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVGSVGElement.currentScale")]
struct ScaleArgs {
    #[webidl(required, converter = "float")]
    value: f32,
}

pub(super) fn scale_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated SVGSVGElement receiver validation");
    let scale = node_runtime_and_handle_from_object_or_detached(scope, receiver)
        .ok()
        .map_or(1.0, |(runtime, handle)| {
            let host = unsafe { &*runtime }.dom_host();
            if host.svg_root_is_outermost(handle) {
                f64::from(host.svg_root_user_transform(handle).scale)
            } else {
                1.0
            }
        });
    rv.set_double(scale);
}

pub(super) fn scale_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<ScaleArgs>(scope, &args) else {
        return;
    };
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated SVGSVGElement receiver validation");
    // Resolve the native owner after ToNumber: author code may move/adopt the
    // element or replace its document while the value is being converted.
    if let Ok((runtime, handle)) = node_runtime_and_handle_from_object_or_detached(scope, receiver)
    {
        unsafe { &mut *runtime }
            .dom_host_mut()
            .set_svg_root_scale(handle, parsed.value);
    }
}

pub(super) fn translation_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated SVGSVGElement receiver validation");
    let point = get_private_value(scope, receiver, CURRENT_TRANSLATE)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .unwrap_or_else(|| {
            let owner = receiver
                .get_creation_context(scope)
                .expect("native SVG root has an owner realm");
            let scope = &mut v8::ContextScope::new(scope, owner);
            let point = build_dom_point_object(scope, 0.0, 0.0, 0.0, 1.0);
            set_private_value(scope, point, TRANSLATE_OWNER, receiver.into());
            set_private_value(scope, receiver, CURRENT_TRANSLATE, point.into());
            point
        });
    sync_translation_point(scope, point);
    rv.set(point.into());
}

fn translation_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    point: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, point, TRANSLATE_OWNER)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(super) fn sync_translation_point<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    point: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(owner) = translation_owner(scope, point) else {
        return false;
    };
    if let Ok((runtime, handle)) = node_runtime_and_handle_from_object_or_detached(scope, owner) {
        let host = unsafe { &*runtime }.dom_host();
        set_dom_point_coordinates(scope, point, host.svg_root_user_transform(handle).point);
        set_svg_point_read_only(scope, point, !host.svg_root_is_outermost(handle));
    } else {
        set_svg_point_read_only(scope, point, true);
    }
    true
}

pub(super) fn reflect_translation_point<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    point: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(owner) = translation_owner(scope, point) else {
        return false;
    };
    let coordinates = dom_point_coordinates(scope, point);
    if let Ok((runtime, handle)) = node_runtime_and_handle_from_object_or_detached(scope, owner) {
        unsafe { &mut *runtime }
            .dom_host_mut()
            .set_svg_root_translation(handle, coordinates);
    }
    true
}
