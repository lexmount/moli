use super::helpers::{
    can_parse_url_input, require_url_receiver, resolve_url_constructor_input, url_href_slot,
};
use super::*;
use crate::interfaces as web_api_interfaces;
use moli_v8_util::get_private_value;
use moli_v8_util::{global_constructor_object, throw_error};
use moli_webapi_declare::WebApiObject;
use moli_webidl as webidl;

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
    #[webidl(index = 1, converter = "usv_string")]
    base: Option<String>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "URL.parse")]
struct UrlParseArgs {
    #[webidl(required, converter = "usv_string")]
    input: String,
    #[webidl(index = 1, converter = "usv_string")]
    base: Option<String>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "URL.canParse")]
struct UrlCanParseArgs {
    #[webidl(required, converter = "usv_string")]
    input: String,
    #[webidl(index = 1, converter = "usv_string")]
    base: Option<String>,
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
    let Some(constructor) = global_constructor_object(scope, "URL")
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        throw_error(
            scope,
            "Failed to create URL: intrinsic constructor is unavailable",
        );
        return;
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
