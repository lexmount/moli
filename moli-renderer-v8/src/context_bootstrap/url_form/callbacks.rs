use super::helpers::{
    can_parse_url_input, require_url_receiver, resolve_url_constructor_input, url_href_slot,
};
use super::*;
use crate::context_bootstrap::{ensure_intrinsic_interface_constructor, shared::throw_error};
use crate::util::get_private_value;
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::URL)]
struct UrlObjectDeclaration<'s> {
    #[webapi(slot = URL_HREF_SLOT)]
    href: String,
    #[webapi(slot = URL_SEARCH_PARAMS_SLOT)]
    search_params: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "URL")]
struct UrlConstructorArgs {
    #[webidl(required, converter = "usv_string")]
    input: String,
    #[webidl(index = 1, with = url_optional_base_arg)]
    base: Option<String>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "URL.parse")]
struct UrlParseArgs {
    #[webidl(required, converter = "usv_string")]
    input: String,
    #[webidl(index = 1, with = url_optional_base_arg)]
    base: Option<String>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "URL.canParse")]
struct UrlCanParseArgs {
    #[webidl(required, converter = "usv_string")]
    input: String,
    #[webidl(index = 1, with = url_optional_base_arg)]
    base: Option<String>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "URL.revokeObjectURL")]
struct UrlRevokeObjectUrlArgs {
    #[webidl(required)]
    url: String,
}

pub(super) fn url_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'URL': Please use the 'new' operator.",
        );
        return;
    }

    let Some(parsed) = webidl::parse_args::<UrlConstructorArgs>(scope, &args) else {
        return;
    };
    let Ok(url) = resolve_url_constructor_input(&parsed.input, parsed.base.as_deref()) else {
        throw_type_error(scope, "Failed to construct 'URL': Invalid URL.");
        return;
    };

    let this = args.this();
    let href = url.as_str().to_owned();
    let has_search_params = get_private_value(scope, this, URL_SEARCH_PARAMS_SLOT)
        .is_some_and(|value| !value.is_undefined());
    let search_params = if has_search_params {
        None
    } else {
        new_url_search_params_object(scope, Some(this), Some(url_query_pairs(&url)))
    };
    UrlObjectDeclaration::new(href, search_params)
        .initialize(scope, this)
        .expect("URL declaration should initialize object");
    rv.set(this.into());
}

pub(super) fn url_to_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(this) = require_url_receiver(scope, args.this()) else {
        return;
    };
    if let Some(href) = url_href_slot(scope, this)
        && let Some(href) = v8_string(scope, &href)
    {
        rv.set(href.into());
        return;
    }
    rv.set(v8::undefined(scope).into());
}

pub(super) fn url_to_json_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    url_to_string_callback(scope, args, rv);
}

pub(super) fn url_parse_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<UrlParseArgs>(scope, &args) else {
        return;
    };
    let Ok(url) = resolve_url_constructor_input(&parsed.input, parsed.base.as_deref()) else {
        rv.set(v8::null(scope).into());
        return;
    };
    let Some(url_value) = v8_string(scope, url.as_str()) else {
        rv.set(v8::null(scope).into());
        return;
    };
    // The factory creates a URL in its own realm, regardless of author changes
    // to the public URL binding (including during argument conversion).
    let constructor = match ensure_intrinsic_interface_constructor(scope, "URL") {
        Ok(constructor) => constructor,
        Err(error) => {
            throw_error(scope, &format!("Failed to create URL: {error}"));
            return;
        }
    };
    let Some(object) = constructor.new_instance(scope, &[url_value.into()]) else {
        rv.set(v8::null(scope).into());
        return;
    };
    rv.set(object.into());
}

pub(super) fn url_can_parse_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<UrlCanParseArgs>(scope, &args) else {
        return;
    };
    rv.set(
        v8::Boolean::new(
            scope,
            can_parse_url_input(&parsed.input, parsed.base.as_deref()),
        )
        .into(),
    );
}

pub(super) fn url_create_object_url_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let origin = if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let active_child_handle = current_child_context_handle(scope)
            .or_else(|| crate::native_bridge::active_child_window_handle(scope));
        unsafe { &mut *host_ptr }
            .active_storage_context(scope, active_child_handle)
            .origin()
            .to_owned()
    } else if let Some(worker_url) = current_worker_script_url(scope) {
        moli_url::origin_ascii_serialization(&worker_url)
    } else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let Ok(object) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_type_error(
            scope,
            "Failed to execute 'createObjectURL' on 'URL': parameter 1 is not of type 'Blob'.",
        );
        return;
    };
    let Some(url) = blob::create_object_url_for_object(scope, object, &origin) else {
        throw_type_error(
            scope,
            "Failed to execute 'createObjectURL' on 'URL': parameter 1 is not of type 'Blob'.",
        );
        return;
    };
    if let Some(value) = v8_string(scope, &url) {
        rv.set(value.into());
    } else {
        rv.set(v8::undefined(scope).into());
    }
}

fn current_child_context_handle(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<crate::document_runtime::DomHandle> {
    let global = scope.get_current_context().global(scope);
    get_private_value(
        scope,
        global,
        crate::context_bootstrap::CHILD_BROWSING_CONTEXT_HANDLE_SLOT,
    )
    .and_then(|value| dom_handle_from_value(scope, value))
}

fn dom_handle_from_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<crate::document_runtime::DomHandle> {
    value
        .number_value(scope)
        .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
        .map(|value| crate::document_runtime::DomHandle::new(value as usize))
}

pub(super) fn url_revoke_object_url_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<UrlRevokeObjectUrlArgs>(scope, &args) else {
        return;
    };
    if parsed.url.starts_with("blob:") {
        blob::revoke_object_url(&parsed.url);
    }
    rv.set_undefined();
}

fn url_optional_base_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<Option<String>, webidl::WebIdlError> {
    if args.length() <= index || args.get(index).is_undefined() {
        return Ok(None);
    }
    let value = args.get(index);
    webidl::convert::<webidl::UsvString>(
        scope,
        value,
        webidl::Context::argument("URL", (index + 1) as usize),
    )
    .map(|value| Some(value.0))
}
