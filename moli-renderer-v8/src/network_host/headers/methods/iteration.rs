use super::super::store::headers_entries;
use super::*;
use crate::web_api_interfaces;
use crate::{
    callback_invocation::invoke_synchronous_webidl_callback_function,
    util::{
        get_private_object, get_private_value, materialize_hidden_function_template_prototype,
        set_private_value,
    },
    webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const HEADERS_ITERATOR_TARGET_SLOT: &str = "__lmHeadersIteratorTarget";
const HEADERS_ITERATOR_INDEX_SLOT: &str = "__lmHeadersIteratorIndex";
const HEADERS_ITERATOR_KIND_SLOT: &str = "__lmHeadersIteratorKind";
const HEADERS_ITERATOR_PROTOTYPE_SLOT: &str = "__lmHeadersIteratorPrototype";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Headers.forEach")]
struct HeadersForEachArgs<'s> {
    #[webidl(
        required,
        converter = "callback_function",
        missing_message = "Headers.forEach requires a callback"
    )]
    callback: webidl::WebIdlCallbackFunction,
    this_arg: Option<v8::Local<'s, v8::Value>>,
}

#[derive(Clone, Copy)]
enum HeadersIteratorKind {
    Keys,
    Values,
    Entries,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct HeadersIteratorResultDeclaration<'scope> {
    done: bool,
    value: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(prototype = "Object", interface = web_api_interfaces::HeadersIterator)]
struct HeadersIteratorDeclaration<'scope> {
    #[webapi(slot = HEADERS_ITERATOR_TARGET_SLOT)]
    target: v8::Local<'scope, v8::Object>,
    #[webapi(slot = HEADERS_ITERATOR_INDEX_SLOT)]
    index: v8::Local<'scope, v8::Integer>,
    #[webapi(slot = HEADERS_ITERATOR_KIND_SLOT)]
    kind: v8::Local<'scope, v8::String>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::HeadersIterator,
    intrinsic_prototype_parent = v8::Intrinsic::IteratorPrototype,
    prototype_to_string_tag = "Headers Iterator",
    readonly_prototype,
    enumerable
)]
struct HeadersIteratorPrototypeDeclaration {
    #[webapi(method, callback = headers_iterator_next_callback)]
    next: (),
}

pub(in crate::network_host::headers) fn headers_keys_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(this) = require_headers_receiver(scope, args.this()) else {
        return;
    };
    if let Some(iter) = live_headers_iterator(scope, this, HeadersIteratorKind::Keys) {
        rv.set(iter);
    } else {
        rv.set_undefined();
    }
}

pub(in crate::network_host::headers) fn headers_values_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(this) = require_headers_receiver(scope, args.this()) else {
        return;
    };
    if let Some(iter) = live_headers_iterator(scope, this, HeadersIteratorKind::Values) {
        rv.set(iter);
    } else {
        rv.set_undefined();
    }
}

pub(in crate::network_host::headers) fn headers_entries_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(this) = require_headers_receiver(scope, args.this()) else {
        return;
    };
    if let Some(iter) = live_headers_iterator(scope, this, HeadersIteratorKind::Entries) {
        rv.set(iter);
    } else {
        rv.set_undefined();
    }
}

pub(in crate::network_host::headers) fn headers_for_each_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(this) = require_headers_receiver(scope, args.this()) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<HeadersForEachArgs>(scope, &args) else {
        return;
    };
    let this_arg = parsed
        .this_arg
        .unwrap_or_else(|| v8::undefined(scope).into());
    let callback = parsed.callback.prepare(scope);
    for (name, value) in headers_entries(scope, this) {
        let Some(name) = v8_string(scope, &name) else {
            continue;
        };
        let Some(value) = v8_string(scope, &value) else {
            continue;
        };
        if invoke_synchronous_webidl_callback_function(
            scope,
            &callback,
            this_arg,
            &[value.into(), name.into(), this.into()],
        )
        .is_none()
        {
            return;
        }
    }
    rv.set_undefined();
}

fn live_headers_iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    kind: HeadersIteratorKind,
) -> Option<v8::Local<'s, v8::Value>> {
    let kind = v8_string(scope, headers_iterator_kind_name(kind))?;
    let iterator = HeadersIteratorDeclaration::new(target, v8::Integer::new(scope, 0), kind)
        .bind(scope)
        .ok()?;

    let prototype = headers_iterator_prototype(scope)?;
    iterator.set_prototype(scope, prototype.into())?;
    Some(iterator.into())
}

fn headers_iterator_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    if let Some(existing) = get_private_value(scope, global, HEADERS_ITERATOR_PROTOTYPE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Some(existing);
    }
    let template = HeadersIteratorPrototypeDeclaration::build(scope);
    let prototype = materialize_hidden_function_template_prototype(scope, template)?;
    set_private_value(
        scope,
        global,
        HEADERS_ITERATOR_PROTOTYPE_SLOT,
        prototype.into(),
    );
    Some(prototype)
}

fn headers_iterator_next_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let iterator = args.this();
    let Some(target) = get_private_object(scope, iterator, HEADERS_ITERATOR_TARGET_SLOT) else {
        rv.set_null();
        return;
    };
    let Some(kind_value) = get_private_value(scope, iterator, HEADERS_ITERATOR_KIND_SLOT) else {
        rv.set_null();
        return;
    };
    let Some(kind_name) = callback_value_string(scope, kind_value) else {
        rv.set_null();
        return;
    };
    let Some(index_value) = get_private_value(scope, iterator, HEADERS_ITERATOR_INDEX_SLOT) else {
        rv.set_null();
        return;
    };
    let index = index_value.integer_value(scope).unwrap_or(0).max(0) as usize;
    let entries = headers_entries(scope, target);
    if index >= entries.len() {
        let result = HeadersIteratorResultDeclaration::new(true, v8::undefined(scope).into())
            .bind(scope)
            .expect("Headers iterator result declaration should bind");
        rv.set(result.into());
        return;
    }

    let (name, value) = &entries[index];
    let value = match kind_name.as_str() {
        "keys" => v8_string(scope, name).map(Into::into),
        "values" => v8_string(scope, value).map(Into::into),
        _ => {
            let Some(name) = v8_string(scope, name) else {
                rv.set_null();
                return;
            };
            let Some(value) = v8_string(scope, value) else {
                rv.set_null();
                return;
            };
            Some(v8::Array::new_with_elements(scope, &[name.into(), value.into()]).into())
        }
    };
    let Some(value) = value else {
        rv.set_null();
        return;
    };

    set_private_value(
        scope,
        iterator,
        HEADERS_ITERATOR_INDEX_SLOT,
        v8::Integer::new(scope, (index + 1) as i32).into(),
    );
    let result = HeadersIteratorResultDeclaration::new(false, value)
        .bind(scope)
        .expect("Headers iterator result declaration should bind");
    rv.set(result.into());
}

fn callback_value_string(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<String> {
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

fn headers_iterator_kind_name(kind: HeadersIteratorKind) -> &'static str {
    match kind {
        HeadersIteratorKind::Keys => "keys",
        HeadersIteratorKind::Values => "values",
        HeadersIteratorKind::Entries => "entries",
    }
}
