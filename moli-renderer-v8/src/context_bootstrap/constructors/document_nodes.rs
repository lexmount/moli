use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Text")]
struct TextConstructorArgs {
    #[webidl(default = "")]
    data: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Comment")]
struct CommentConstructorArgs {
    #[webidl(default = "")]
    data: String,
}

pub(in crate::context_bootstrap) fn document_constructor_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'Document': Please use the 'new' operator.",
        );
        return;
    }
    let origin_document = current_document_object(scope);
    match call_global_bridge_method(scope, "__createDetachedDocument", &[]) {
        Some(value) => {
            let Ok(document) = v8::Local::<v8::Object>::try_from(value) else {
                rv.set_undefined();
                return;
            };
            if let Some(origin_document) = origin_document {
                crate::native_bridge::document::inherit_detached_document_origin(
                    scope,
                    document,
                    origin_document,
                );
            }
            if crate::context_bootstrap::install_constructed_document_location_runtime_state(
                scope, document,
            )
            .is_err()
            {
                rv.set_undefined();
                return;
            }
            rv.set(document.into());
        }
        None => rv.set_undefined(),
    }
}

fn current_document_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    global
        .get(scope, v8str(scope, "document").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(in crate::context_bootstrap) fn document_fragment_constructor_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'DocumentFragment': Please use the 'new' operator.",
        );
        return;
    }
    if let Some(value) = call_current_document_method(scope, "createDocumentFragment", &[]) {
        rv.set(value);
        return;
    }
    match call_global_bridge_method(scope, "__createDetachedDocumentFragment", &[]) {
        Some(value) => rv.set(value),
        None => rv.set_undefined(),
    }
}

pub(in crate::context_bootstrap) fn text_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'Text': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<TextConstructorArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    let Some(data) = v8_string(scope, &parsed.data).map(Into::into) else {
        rv.set_undefined();
        return;
    };
    if let Some(value) = call_current_document_method(scope, "createTextNode", &[data]) {
        rv.set(value);
        return;
    }
    match call_global_bridge_method(scope, "__createDetachedText", &[data]) {
        Some(value) => rv.set(value),
        None => rv.set_undefined(),
    }
}

pub(in crate::context_bootstrap) fn comment_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'Comment': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<CommentConstructorArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    let Some(data) = v8_string(scope, &parsed.data).map(Into::into) else {
        rv.set_undefined();
        return;
    };
    if let Some(value) = call_current_document_method(scope, "createComment", &[data]) {
        rv.set(value);
        return;
    }
    match call_global_bridge_method(scope, "__createDetachedComment", &[data]) {
        Some(value) => rv.set(value),
        None => rv.set_undefined(),
    }
}

fn call_current_document_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    method_name: &str,
    args: &[v8::Local<'s, v8::Value>],
) -> Option<v8::Local<'s, v8::Value>> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let scope = try_catch.init();
    let global = scope.get_current_context().global(&scope);
    let document_key = v8str(&scope, "document");
    let document = global.get(&scope, document_key.into())?;
    let document = v8::Local::<v8::Object>::try_from(document).ok()?;
    let method_key = v8_string(&scope, method_name)?;
    let method = document.get(&scope, method_key.into())?;
    let method = v8::Local::<v8::Function>::try_from(method).ok()?;
    method.call(&scope, document.into(), args)
}
