use super::backing_store::{canvas_like_has_context, canvas_like_pixels_copy};
use crate::{
    context_bootstrap::new_dom_exception_value, util::throw_type_error, web_api_interfaces,
};
pub(super) use moli_webapi::image_bitmap::{
    build_image_bitmap_object, install_image_bitmap_template_bindings,
};
pub(crate) fn window_create_image_bitmap_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.is_construct_call() {
        throw_type_error(scope, "createImageBitmap is not a constructor");
        return;
    }
    if args.length() == 0 {
        throw_type_error(
            scope,
            "Failed to execute 'createImageBitmap' on 'Window': 1 argument required, but only 0 present.",
        );
        return;
    }
    let Ok(source) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_type_error(
            scope,
            "Failed to execute 'createImageBitmap' on 'Window': The provided value is not a valid image source.",
        );
        return;
    };
    if !web_api_interfaces::OffscreenCanvas::is_instance(scope, source) {
        throw_type_error(
            scope,
            "Failed to execute 'createImageBitmap' on 'Window': The provided value is not a valid image source.",
        );
        return;
    }

    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let promise = resolver.get_promise(scope);
    if !canvas_like_has_context(scope, source) {
        let error = new_dom_exception_value(
            scope,
            "Failed to execute 'createImageBitmap' on 'Window': The ImageBitmap could not be allocated.",
            "InvalidStateError",
        );
        let _ = resolver.reject(scope, error);
        rv.set(promise.into());
        return;
    }

    let Some((_pixels, width, height)) = canvas_like_pixels_copy(scope, source) else {
        let error = new_dom_exception_value(
            scope,
            "Failed to execute 'createImageBitmap' on 'Window': The ImageBitmap could not be allocated.",
            "InvalidStateError",
        );
        let _ = resolver.reject(scope, error);
        rv.set(promise.into());
        return;
    };
    let Some(bitmap) = build_image_bitmap_object(scope, width, height) else {
        let error = new_dom_exception_value(
            scope,
            "Failed to execute 'createImageBitmap' on 'Window': The ImageBitmap could not be allocated.",
            "InvalidStateError",
        );
        let _ = resolver.reject(scope, error);
        rv.set(promise.into());
        return;
    };
    let _ = resolver.resolve(scope, bitmap.into());
    rv.set(promise.into());
}
