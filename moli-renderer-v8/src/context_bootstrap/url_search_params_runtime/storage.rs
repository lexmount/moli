use super::*;
use moli_url::search_params::{SearchParamPair, SearchParams, serialize_search_params_pairs};
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = "URLSearchParams")]
struct UrlSearchParamsObjectDeclaration<'s> {
    #[webapi(slot = URL_SEARCH_PARAMS_OWNER_SLOT)]
    owner: v8::Local<'s, v8::Value>,
    #[webapi(slot = URL_SEARCH_PARAMS_PAIRS_SLOT)]
    pairs: v8::Local<'s, v8::Array>,
}

pub(in crate::context_bootstrap) fn new_url_search_params_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: Option<v8::Local<'s, v8::Object>>,
    initial_pairs: Option<Vec<SearchParamPair>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let pairs = initial_pairs.as_deref().unwrap_or(&[]);
    let global = scope.get_current_context().global(scope);
    if let Some(object) = global
        .get(scope, v8str(scope, "URLSearchParams").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .and_then(|ctor| ctor.new_instance(scope, &[]))
    {
        initialize_url_search_params_object(scope, object, owner, pairs);
        return Some(object);
    }
    UrlSearchParamsObjectDeclaration::new(
        owner
            .map(Into::into)
            .unwrap_or_else(|| v8::null(scope).into()),
        new_string_pairs_array(scope, pairs),
    )
    .bind(scope)
    .ok()
}

pub(super) fn initialize_url_search_params_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    owner: Option<v8::Local<'s, v8::Object>>,
    pairs: &[SearchParamPair],
) {
    let pairs_array = new_string_pairs_array(scope, pairs);
    UrlSearchParamsObjectDeclaration::new(
        owner
            .map(Into::into)
            .unwrap_or_else(|| v8::null(scope).into()),
        pairs_array,
    )
    .initialize(scope, object)
    .expect("URLSearchParams declaration should initialize object");
}

pub(super) fn url_search_params_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, object, URL_SEARCH_PARAMS_OWNER_SLOT)
}

pub(super) fn url_search_params_pairs_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_value(scope, object, URL_SEARCH_PARAMS_PAIRS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

pub(super) fn url_search_params_pairs<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Vec<SearchParamPair> {
    if let Some(owner) = url_search_params_owner(scope, object)
        && let Some(url) = url_object_value(scope, owner)
    {
        return SearchParams::from_url(&url).into_pairs();
    }
    let array = url_search_params_pairs_array(scope, object);
    string_pairs_from_array(scope, array)
}

pub(super) fn set_url_search_params_pairs<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    pairs: &[SearchParamPair],
) {
    if let Some(owner) = url_search_params_owner(scope, object)
        && let Some(mut url) = url_object_value(scope, owner)
    {
        let serialized: Option<String> = serialize_search_params_pairs(pairs);
        url.set_query(serialized.as_deref());
        apply_url_update(scope, owner, &url);
        return;
    }
    let next = new_string_pairs_array(scope, pairs);
    set_private_value(scope, object, URL_SEARCH_PARAMS_PAIRS_SLOT, next.into());
}

pub(super) fn url_search_params_is_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    // The internal slot is the WebIDL brand. A constructed subclass has this
    // slot even though its direct prototype is not URLSearchParams.prototype.
    url_search_params_pairs_array(scope, object).is_some()
}

pub(crate) fn url_search_params_request_body<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    if !url_search_params_is_object(scope, object) {
        return None;
    }
    Some(serialize_search_params_pairs(&url_search_params_pairs(scope, object)).unwrap_or_default())
}

fn new_string_pairs_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pairs: &[SearchParamPair],
) -> v8::Local<'s, v8::Array> {
    crate::util::serialize_v8_array(scope, pairs).unwrap_or_else(|| v8::Array::new(scope, 0))
}

fn string_pairs_from_array(
    scope: &mut v8::PinScope<'_, '_>,
    array: Option<v8::Local<'_, v8::Array>>,
) -> Vec<SearchParamPair> {
    let Some(array) = array else {
        return Vec::new();
    };
    let mut pairs = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        let Some(pair) = array
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        else {
            continue;
        };
        let Some(key) = pair
            .get_index(scope, 0)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
        else {
            continue;
        };
        let value = pair
            .get_index(scope, 1)
            .and_then(|value| value.to_string(scope))
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_default();
        pairs.push((key, value));
    }
    pairs
}
