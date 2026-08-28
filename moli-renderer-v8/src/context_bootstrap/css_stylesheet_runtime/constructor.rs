use super::*;
use crate::{
    native_bridge::OwnerDispatchScope,
    util::{callable_relevant_context, throw_type_error},
};

pub(crate) fn css_style_sheet_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "Constructor must be called with new");
        return;
    }
    let Some(relevant_context) = callable_relevant_context(scope, args.new_target()) else {
        throw_type_error(scope, "CSSStyleSheet constructor realm is unavailable");
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, relevant_context);
    construct_css_style_sheet(scope, &args, rv);
}

fn construct_css_style_sheet<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    initialize_css_style_sheet_object(scope, args.this());
    let Some(media_text) = constructor_media_option(scope, args) else {
        return;
    };
    let disabled = constructor_disabled_option(scope, args);
    let constructor_document =
        current_css_style_sheet_constructor_document_handle_for_context(scope);
    let Some(base_url) = constructor_base_url_option(scope, args, constructor_document) else {
        return;
    };
    set_private_value(
        scope,
        args.this(),
        CSS_STYLE_SHEET_CONSTRUCTED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    if let Some(handle) = constructor_document {
        set_css_style_sheet_constructor_document_handle(scope, args.this(), handle);
    }
    if let Some(base_url) = base_url {
        set_private_string(
            scope,
            args.this(),
            CSS_STYLE_SHEET_BASE_URL_SLOT,
            base_url.as_str(),
        );
    }
    sync_constructed_css_style_sheet_rules_from_text(scope, args.this(), "");
    if !media_text.is_empty() {
        sync_style_sheet_media_text(scope, args.this(), &media_text);
    }
    if disabled {
        require_css_style_sheet_live_stylesheet(scope, args.this()).set_disabled(true);
    }
    rv.set(args.this().into());
}

pub(crate) fn current_css_style_sheet_constructor_document_handle_for_context(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<DomHandle> {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return child_handle_marker(scope);
    };
    let host = unsafe { &*host_ptr };
    let identity = host.current_runtime_window_execution_context_identity(scope)?;
    match identity.dispatch_scope() {
        OwnerDispatchScope::Child(handle) => host.child_browsing_context_document_handle(handle),
        OwnerDispatchScope::Top => Some(host.document_handle()),
    }
}

fn child_handle_marker(scope: &mut v8::PinScope<'_, '_>) -> Option<DomHandle> {
    let global = scope.get_current_context().global(scope);
    get_private_value(
        scope,
        global,
        crate::context_bootstrap::CHILD_BROWSING_CONTEXT_HANDLE_SLOT,
    )
    .and_then(|value| dom_handle_from_marker_value(scope, value))
}

fn constructor_media_option<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<String> {
    if args.length() == 0 || webidl::is_nullish(args.get(0)) || !args.get(0).is_object() {
        return Some(String::new());
    }
    let Some(media) = v8::Local::<v8::Object>::try_from(args.get(0))
        .ok()
        .and_then(|options| webidl::property(scope, options, "media"))
    else {
        return Some(String::new());
    };
    if media.is_undefined() {
        return Some(String::new());
    }
    cssom_dom_string_property_value(scope, media, "CSSStyleSheetInit", "media")
}

fn constructor_disabled_option<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> bool {
    if args.length() == 0 || webidl::is_nullish(args.get(0)) || !args.get(0).is_object() {
        return false;
    }
    v8::Local::<v8::Object>::try_from(args.get(0))
        .ok()
        .and_then(|options| webidl::property(scope, options, "disabled"))
        .is_some_and(|value| value.boolean_value(scope))
}

fn constructor_base_url_option<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    constructor_document: Option<DomHandle>,
) -> Option<Option<url::Url>> {
    if args.length() == 0 || webidl::is_nullish(args.get(0)) || !args.get(0).is_object() {
        return Some(None);
    }
    let Some(base_url_value) = v8::Local::<v8::Object>::try_from(args.get(0))
        .ok()
        .and_then(|options| webidl::property(scope, options, "baseURL"))
    else {
        return Some(None);
    };
    if base_url_value.is_undefined() {
        return Some(None);
    }
    let base_url_text =
        cssom_dom_string_property_value(scope, base_url_value, "CSSStyleSheetInit", "baseURL")?;
    let document_base_url = constructor_document_base_url(scope, constructor_document)?;
    match url::Url::options()
        .base_url(Some(&document_base_url))
        .parse(&base_url_text)
    {
        Ok(url) if url.username().is_empty() && url.password().is_none() => Some(Some(url)),
        _ => {
            webidl::throw_dom_exception(scope, "NotAllowedError", "Invalid CSSStyleSheet baseURL.");
            None
        }
    }
}
