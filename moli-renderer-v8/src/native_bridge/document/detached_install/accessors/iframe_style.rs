use crate::{
    native_bridge::{
        ComputedStyleDescriptor, ComputedStyleTargetKey,
        element::{
            STYLE_DECLARATION_FORCED_EMPTY_COMPUTED_SLOT, STYLE_DECLARATION_PSEUDO_ELEMENT_SLOT,
            STYLE_DECLARATION_READ_DOCUMENT_SLOT, STYLE_DECLARATION_SCREEN_HEIGHT_SLOT,
            STYLE_DECLARATION_SCREEN_WIDTH_SLOT, STYLE_DECLARATION_TARGET_CONTEXT_EPOCH_SLOT,
            STYLE_DECLARATION_TARGET_EMPTY_COMPUTED_SLOT, STYLE_DECLARATION_VIEWPORT_WIDTH_SLOT,
        },
    },
    util::{context_host_ptr_from_global_bridge, set_private_value, v8_string},
};
use moli_webapi_declare::WebApiObject;

use super::super::super::detached_native_handle_for_runtime;
use super::iframe_style_viewport::detached_iframe_viewport_width;

#[derive(WebApiObject)]
#[webapi(plain)]
struct DetachedIframeWindowComputedStyleMethodDeclaration<'scope> {
    iframe: v8::Local<'scope, v8::Object>,
    #[webapi(
        method,
        length = 1,
        callback = detached_iframe_get_computed_style_callback,
        data = self.iframe
    )]
    get_computed_style: (),
}

pub(super) fn install_detached_iframe_get_computed_style<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    iframe: v8::Local<'s, v8::Object>,
) {
    let _ =
        DetachedIframeWindowComputedStyleMethodDeclaration::new(iframe).initialize(scope, window);
}

fn detached_iframe_get_computed_style_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_null();
        return;
    };
    let Ok(iframe) = v8::Local::<v8::Object>::try_from(args.data()) else {
        rv.set_null();
        return;
    };
    let Some(target) = v8::Local::<v8::Object>::try_from(args.get(0))
        .ok()
        .and_then(|target| detached_native_handle_for_runtime(scope, runtime_ptr, target))
    else {
        rv.set_null();
        return;
    };
    let crate::window_host::ComputedStylePseudoArgument {
        forced_empty,
        pseudo_element,
        pseudo_key,
    } = crate::window_host::computed_style_pseudo_argument_from_function_args(scope, &args);
    let target_key = detached_native_handle_for_runtime(scope, runtime_ptr, iframe)
        .map(ComputedStyleTargetKey::DetachedIframe)
        .unwrap_or(ComputedStyleTargetKey::Dynamic);
    let descriptor = ComputedStyleDescriptor::new(pseudo_key, target_key);
    let read_document = unsafe { &*runtime_ptr }
        .dom_host()
        .node(target)
        .and_then(crate::dom::native::Node::owner_document);
    let host = unsafe { &mut *runtime_ptr };
    let Some(style) =
        host.native_bridge_mut()
            .wrap_computed_style(scope, runtime_ptr, target, descriptor)
    else {
        rv.set_null();
        return;
    };
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_FORCED_EMPTY_COMPUTED_SLOT,
        v8::Boolean::new(scope, forced_empty).into(),
    );
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_TARGET_EMPTY_COMPUTED_SLOT,
        v8::Boolean::new(scope, false).into(),
    );
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_TARGET_CONTEXT_EPOCH_SLOT,
        v8::undefined(scope).into(),
    );
    let read_document = read_document
        .map(|handle| v8::Integer::new_from_unsigned(scope, handle.index_u32()).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_READ_DOCUMENT_SLOT,
        read_document,
    );
    let pseudo_value = pseudo_element
        .as_deref()
        .and_then(|pseudo_element| v8_string(scope, pseudo_element).map(Into::into))
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_PSEUDO_ELEMENT_SLOT,
        pseudo_value,
    );
    let width = detached_iframe_viewport_width(scope, iframe)
        .map(|width| v8::Number::new(scope, width).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(scope, style, STYLE_DECLARATION_VIEWPORT_WIDTH_SLOT, width);
    let viewport = host.style_viewport();
    let screen_width = viewport
        .screen_width
        .map(|width| v8::Number::new(scope, width).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_SCREEN_WIDTH_SLOT,
        screen_width,
    );
    let screen_height = viewport
        .screen_height
        .map(|height| v8::Number::new(scope, height).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_SCREEN_HEIGHT_SLOT,
        screen_height,
    );
    rv.set(style.into());
}
