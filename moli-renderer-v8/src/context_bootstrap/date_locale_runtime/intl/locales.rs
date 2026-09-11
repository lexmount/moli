//! A per-call, lazy locale list. Let the native constructor decide *when* to
//! canonicalize locales: doing it in our outer Proxy trap would run page
//! getters before newTarget.prototype, or even for calls that should throw
//! without examining arguments. Date locale methods likewise return early for
//! an invalid Date. Only the native CanonicalizeLocaleList's length read opens
//! this adapter; it never eagerly inspects the page's options.

use crate::util::{
    call_script_visible_function, get_private_value, set_private_value, v8_string, v8str,
};
use crate::webidl;
use moli_webapi_declare::WebApiObject;

use super::super::bindings::callback_data;

const DATE_LOCALE_INTRINSICS_SLOT: &str = "__moliDateLocaleListIntrinsics";
pub(super) const DEFAULT_LOCALE_SLOT: &str = "__moliLocaleListDefault";

#[derive(Clone, Copy, WebApiObject, webidl::WebIdlDictionary)]
#[webapi(plain, data_properties)]
pub(super) struct LocaleIntrinsics<'s> {
    #[webidl(required)]
    pub canonicalize: v8::Local<'s, v8::Function>,
    #[webidl(required)]
    pub supported: v8::Local<'s, v8::Function>,
}

#[derive(Clone, Copy, WebApiObject, webidl::WebIdlDictionary)]
#[webapi(plain, data_properties)]
struct LocaleListData<'s> {
    requested: Option<v8::Local<'s, v8::Value>>,
    #[webidl(required)]
    fallback: v8::Local<'s, v8::Value>,
    #[webidl(required)]
    canonicalize: v8::Local<'s, v8::Function>,
    #[webidl(required)]
    supported: v8::Local<'s, v8::Function>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct DeferredLocaleList<'s> {
    data: LocaleListData<'s>,
    #[webapi(accessor_property, getter = locale_list_length_callback, data = self.data)]
    length: (),
}

pub(super) fn defer_locale_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    original: v8::Local<'s, v8::Value>,
    fallback: &str,
    intrinsics: LocaleIntrinsics<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let fallback = v8_string(scope, fallback)?;
    DeferredLocaleList::new(LocaleListData::new(
        Some(original),
        fallback.into(),
        intrinsics.canonicalize,
        intrinsics.supported,
    ))
    .bind(scope)
    .ok()
}

pub(super) fn retain_date_locale_intrinsics<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    intrinsics: LocaleIntrinsics<'s>,
) -> Result<(), moli_webapi_declare::BindError> {
    let value = intrinsics.bind(scope)?;
    set_private_value(scope, global, DATE_LOCALE_INTRINSICS_SLOT, value.into());
    Ok(())
}

pub(super) fn defer_date_locale_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    original: v8::Local<'s, v8::Value>,
    fallback: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let data = get_private_value(scope, global, DATE_LOCALE_INTRINSICS_SLOT)?;
    let intrinsics = callback_data::<LocaleIntrinsics>(scope, data)?;
    defer_locale_list(scope, original, fallback, intrinsics)
}

fn locale_list_length_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(data) = callback_data::<LocaleListData>(scope, args.data()) else {
        return;
    };
    let receiver = v8::undefined(scope);
    let Some(canonical) = call_script_visible_function(
        scope,
        data.canonicalize,
        receiver.into(),
        &[data.requested.unwrap_or_else(|| receiver.into())],
        "canonicalize emulated Intl locales",
    )
    .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok()) else {
        return;
    };

    // Match on the native service's available locales, using only canonical
    // strings, not the page's array-like again. This records default identity
    // for resolvedOptions (e.g. Collator otherwise minimizes fr-FR to fr).
    // Actual selection remains native: preserve the ordered request list and
    // put our default last, including when a caller selects localeMatcher.
    let Some(supported) = call_script_visible_function(
        scope,
        data.supported,
        receiver.into(),
        &[canonical.into()],
        "check native Intl locale support",
    )
    .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok()) else {
        return;
    };
    if supported.length() == 0 {
        set_private_value(scope, args.this(), DEFAULT_LOCALE_SLOT, data.fallback);
    }
    for index in 0..canonical.length() {
        let Some(value) = canonical.get_index(scope, index) else {
            return;
        };
        let Some(key) = v8_string(scope, &index.to_string()) else {
            return;
        };
        if args.this().create_data_property(scope, key.into(), value) != Some(true) {
            return;
        }
    }
    let Some(key) = v8_string(scope, &canonical.length().to_string()) else {
        return;
    };
    if args
        .this()
        .create_data_property(scope, key.into(), data.fallback)
        != Some(true)
    {
        return;
    }
    let length = v8::Number::new(scope, f64::from(canonical.length()) + 1.0);
    // Replace the getter with an own data property. Repeated reads of this
    // internal list must never repeat the page's getters or coercion hooks.
    let key = v8str(scope, "length");
    if args
        .this()
        .create_data_property(scope, key.into(), length.into())
        == Some(true)
    {
        rv.set(length.into());
    }
}
