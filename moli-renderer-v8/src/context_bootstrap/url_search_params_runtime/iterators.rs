use super::*;
use crate::context_bootstrap::url_search_params_runtime::storage::url_search_params_pairs;
use crate::util::{
    get_private_object, get_private_value, materialize_hidden_function_template_prototype,
    set_private_value,
};
use crate::web_api_interfaces;
use moli_url::search_params::{
    SearchParamsIteratorKind, SearchParamsIteratorValue, search_params_iterator_step,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const URL_SEARCH_PARAMS_ITERATOR_TARGET_SLOT: &str = "__moliUrlSearchParamsIteratorTarget";
const URL_SEARCH_PARAMS_ITERATOR_INDEX_SLOT: &str = "__moliUrlSearchParamsIteratorIndex";
const URL_SEARCH_PARAMS_ITERATOR_KIND_SLOT: &str = "__moliUrlSearchParamsIteratorKind";
const URL_SEARCH_PARAMS_ITERATOR_PROTOTYPE_SLOT: &str = "__moliUrlSearchParamsIteratorPrototype";

#[derive(WebApiObject)]
#[webapi(prototype = "Object", interface = web_api_interfaces::URLSearchParamsIterator)]
struct UrlSearchParamsIteratorDeclaration<'scope> {
    #[webapi(slot = URL_SEARCH_PARAMS_ITERATOR_TARGET_SLOT)]
    target: v8::Local<'scope, v8::Object>,
    #[webapi(slot = URL_SEARCH_PARAMS_ITERATOR_INDEX_SLOT)]
    index: i32,
    #[webapi(slot = URL_SEARCH_PARAMS_ITERATOR_KIND_SLOT)]
    kind: &'static str,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::URLSearchParamsIterator,
    intrinsic_prototype_parent = v8::Intrinsic::IteratorPrototype,
    prototype_to_string_tag = "URLSearchParams Iterator",
    readonly_prototype,
    enumerable
)]
struct UrlSearchParamsIteratorPrototypeDeclaration {
    #[webapi(
        method = "next",
        callback = url_search_params_iterator_next_callback
    )]
    next: (),
}

#[derive(WebApiObject)]
#[webapi(record)]
struct UrlSearchParamsIteratorResultDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    done: bool,
    #[webapi(data_property, enumerable)]
    value: v8::Local<'scope, v8::Value>,
}

pub(super) fn live_url_search_params_iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    kind: SearchParamsIteratorKind,
) -> Option<v8::Local<'s, v8::Value>> {
    let iterator = UrlSearchParamsIteratorDeclaration::new(target, 0, kind.as_str())
        .bind(scope)
        .ok()?;
    let prototype = url_search_params_iterator_prototype(scope)?;
    iterator.set_prototype(scope, prototype.into())?;
    Some(iterator.into())
}

fn url_search_params_iterator_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    if let Some(existing) =
        get_private_value(scope, global, URL_SEARCH_PARAMS_ITERATOR_PROTOTYPE_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Some(existing);
    }

    let template = UrlSearchParamsIteratorPrototypeDeclaration::build(scope);
    let prototype = materialize_hidden_function_template_prototype(scope, template)?;
    set_private_value(
        scope,
        global,
        URL_SEARCH_PARAMS_ITERATOR_PROTOTYPE_SLOT,
        prototype.into(),
    );
    Some(prototype)
}

fn url_search_params_iterator_next_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let iterator = args.this();
    let Some(target) = get_private_object(scope, iterator, URL_SEARCH_PARAMS_ITERATOR_TARGET_SLOT)
    else {
        throw_type_error(
            scope,
            "URLSearchParams Iterator.prototype.next called on incompatible receiver.",
        );
        return;
    };
    let Some(kind_value) = get_private_value(scope, iterator, URL_SEARCH_PARAMS_ITERATOR_KIND_SLOT)
    else {
        rv.set_null();
        return;
    };
    let Some(kind_name) = callback_value_string(scope, kind_value) else {
        rv.set_null();
        return;
    };
    let Some(index_value) =
        get_private_value(scope, iterator, URL_SEARCH_PARAMS_ITERATOR_INDEX_SLOT)
    else {
        rv.set_null();
        return;
    };
    let index = index_value.integer_value(scope).unwrap_or(0).max(0) as usize;
    let pairs = url_search_params_pairs(scope, target);
    let kind =
        SearchParamsIteratorKind::parse(&kind_name).unwrap_or(SearchParamsIteratorKind::Entries);
    let step = search_params_iterator_step(&pairs, index, kind);
    if step.done {
        let result = url_search_params_iterator_result(scope, v8::undefined(scope).into(), true);
        rv.set(result.into());
        return;
    }

    let value: Option<v8::Local<'_, v8::Value>> = match step.value {
        Some(SearchParamsIteratorValue::String(value)) => v8_string(scope, &value).map(Into::into),
        Some(SearchParamsIteratorValue::Pair((key, value))) => {
            let Some(key) = v8_string(scope, &key) else {
                rv.set_null();
                return;
            };
            let Some(value) = v8_string(scope, &value) else {
                rv.set_null();
                return;
            };
            Some(v8::Array::new_with_elements(scope, &[key.into(), value.into()]).into())
        }
        None => None,
    };
    let Some(value) = value else {
        rv.set_null();
        return;
    };

    set_private_value(
        scope,
        iterator,
        URL_SEARCH_PARAMS_ITERATOR_INDEX_SLOT,
        v8::Integer::new(scope, step.next_index.min(i32::MAX as usize) as i32).into(),
    );
    let result = url_search_params_iterator_result(scope, value, false);
    rv.set(result.into());
}

fn url_search_params_iterator_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    done: bool,
) -> v8::Local<'s, v8::Object> {
    UrlSearchParamsIteratorResultDeclaration::new(done, value)
        .bind(scope)
        .expect("URLSearchParams iterator result declaration should bind")
}
