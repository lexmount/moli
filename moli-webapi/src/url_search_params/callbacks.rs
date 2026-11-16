use super::iterators::live_url_search_params_iterator;
use super::parse::url_search_params_pairs_from_constructor;
use super::storage::{
    initialize_url_search_params_object, set_url_search_params_pairs, url_search_params_is_object,
    url_search_params_pairs,
};
use super::*;
use crate::callbacks::invoke_synchronous_webidl_callback_function;
use moli_url::search_params::{
    SearchParams, SearchParamsIteratorKind, serialize_search_params_pairs,
};
use moli_webidl as webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "URLSearchParams")]
struct UrlSearchParamsNameArgs {
    #[webidl(required, converter = "usv_string")]
    name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "URLSearchParams")]
struct UrlSearchParamsNameValueArgs {
    #[webidl(required, converter = "usv_string")]
    name: String,
    #[webidl(required, converter = "usv_string")]
    value: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "URLSearchParams")]
struct UrlSearchParamsNameOptionalValueArgs {
    #[webidl(required, converter = "usv_string")]
    name: String,
    #[webidl(converter = "usv_string")]
    value: Option<String>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "URLSearchParams.forEach")]
struct UrlSearchParamsForEachArgs<'s> {
    #[webidl(
        required,
        converter = "callback_function",
        missing_message = "URLSearchParams.forEach requires a callback"
    )]
    callback: webidl::WebIdlCallbackFunction,
    this_arg: Option<v8::Local<'s, v8::Value>>,
}

fn require_url_search_params_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    method: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = args.this();
    if !url_search_params_is_object(scope, object) {
        throw_type_error(
            scope,
            &format!("Failed to execute '{method}' on 'URLSearchParams': Illegal invocation."),
        );
        return None;
    }
    Some(object)
}

fn require_url_search_params_property_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    if !url_search_params_is_object(scope, object) {
        throw_type_error(
            scope,
            &format!("Failed to get '{name}' on 'URLSearchParams': Illegal invocation."),
        );
        return None;
    }
    Some(object)
}

pub(super) fn url_search_params_get_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_url_search_params_receiver(scope, &args, "get") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<UrlSearchParamsNameArgs>(scope, &args) else {
        return;
    };
    let params = SearchParams::from_pairs(url_search_params_pairs(scope, object));
    if let Some(value) = params.get(&parsed.name) {
        if let Some(value) = v8_string(scope, value) {
            rv.set(value.into());
        } else {
            rv.set_null();
        }
    } else {
        rv.set_null();
    }
}

pub(super) fn url_search_params_get_all_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_url_search_params_receiver(scope, &args, "getAll") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<UrlSearchParamsNameArgs>(scope, &args) else {
        return;
    };
    let values =
        SearchParams::from_pairs(url_search_params_pairs(scope, object)).get_all(&parsed.name);
    use moli_webapi_declare::WebApiValue;
    let array = values
        .as_slice()
        .to_v8_value(scope)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .unwrap_or_else(|| v8::Array::new(scope, 0));
    rv.set(array.into());
}

pub(super) fn url_search_params_append_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_url_search_params_receiver(scope, &args, "append") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<UrlSearchParamsNameValueArgs>(scope, &args) else {
        return;
    };
    let mut params = SearchParams::from_pairs(url_search_params_pairs(scope, object));
    params.append(parsed.name, parsed.value);
    set_url_search_params_pairs(scope, object, params.as_pairs());
    rv.set_undefined();
}

pub(super) fn url_search_params_set_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_url_search_params_receiver(scope, &args, "set") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<UrlSearchParamsNameValueArgs>(scope, &args) else {
        return;
    };
    let mut params = SearchParams::from_pairs(url_search_params_pairs(scope, object));
    params.set(parsed.name, parsed.value);
    set_url_search_params_pairs(scope, object, params.as_pairs());
    rv.set_undefined();
}

pub(super) fn url_search_params_delete_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_url_search_params_receiver(scope, &args, "delete") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<UrlSearchParamsNameOptionalValueArgs>(scope, &args)
    else {
        return;
    };
    let mut params = SearchParams::from_pairs(url_search_params_pairs(scope, object));
    params.delete(&parsed.name, parsed.value.as_deref());
    set_url_search_params_pairs(scope, object, params.as_pairs());
    rv.set_undefined();
}

pub(super) fn url_search_params_size_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_url_search_params_property_receiver(scope, args.this(), "size")
    else {
        return;
    };
    let size = url_search_params_pairs(scope, object).len() as f64;
    rv.set(v8::Number::new(scope, size).into());
}

pub(super) fn url_search_params_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'URLSearchParams': Please use the 'new' operator.",
        );
        return;
    }

    let Some(pairs) = url_search_params_pairs_from_constructor(scope, args.get(0)) else {
        return;
    };
    initialize_url_search_params_object(scope, args.this(), None, &pairs);
    rv.set(args.this().into());
}

pub(super) fn url_search_params_has_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_url_search_params_receiver(scope, &args, "has") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<UrlSearchParamsNameOptionalValueArgs>(scope, &args)
    else {
        return;
    };
    let params = SearchParams::from_pairs(url_search_params_pairs(scope, object));
    let present = params.has(&parsed.name, parsed.value.as_deref());
    rv.set(v8::Boolean::new(scope, present).into());
}

pub(super) fn url_search_params_keys_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_url_search_params_receiver(scope, &args, "keys") else {
        return;
    };
    if let Some(iter) =
        live_url_search_params_iterator(scope, object, SearchParamsIteratorKind::Keys)
    {
        rv.set(iter);
    } else {
        rv.set(v8::undefined(scope).into());
    }
}

pub(super) fn url_search_params_values_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_url_search_params_receiver(scope, &args, "values") else {
        return;
    };
    if let Some(iter) =
        live_url_search_params_iterator(scope, object, SearchParamsIteratorKind::Values)
    {
        rv.set(iter);
    } else {
        rv.set(v8::undefined(scope).into());
    }
}

pub(super) fn url_search_params_entries_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_url_search_params_receiver(scope, &args, "entries") else {
        return;
    };
    if let Some(iter) =
        live_url_search_params_iterator(scope, object, SearchParamsIteratorKind::Entries)
    {
        rv.set(iter);
    } else {
        rv.set(v8::undefined(scope).into());
    }
}

pub(super) fn url_search_params_for_each_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_url_search_params_receiver(scope, &args, "forEach") else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<UrlSearchParamsForEachArgs>(scope, &args) else {
        return;
    };
    let this_arg = parsed
        .this_arg
        .unwrap_or_else(|| v8::undefined(scope).into());
    let callback = parsed.callback.prepare(scope);
    let mut index = 0;
    loop {
        let pairs = url_search_params_pairs(scope, object);
        let Some((key, value)) = pairs.get(index) else {
            break;
        };
        index += 1;
        let Some(key) = v8_string(scope, key) else {
            continue;
        };
        let Some(value) = v8_string(scope, value) else {
            continue;
        };
        if invoke_synchronous_webidl_callback_function(
            scope,
            &callback,
            this_arg,
            &[value.into(), key.into(), object.into()],
        )
        .is_none()
        {
            return;
        }
    }
    rv.set_undefined();
}

pub(super) fn url_search_params_sort_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_url_search_params_receiver(scope, &args, "sort") else {
        return;
    };
    let mut params = SearchParams::from_pairs(url_search_params_pairs(scope, object));
    params.sort();
    set_url_search_params_pairs(scope, object, params.as_pairs());
    rv.set_undefined();
}

pub(super) fn url_search_params_to_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(object) = require_url_search_params_receiver(scope, &args, "toString") else {
        return;
    };
    let pairs = url_search_params_pairs(scope, object);
    let serialized = serialize_search_params_pairs(&pairs).unwrap_or_default();
    if let Some(value) = v8_string(scope, &serialized) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}
