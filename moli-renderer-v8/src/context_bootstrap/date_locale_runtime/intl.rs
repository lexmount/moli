use crate::util::{
    call_script_visible_function, define_v8_array_data_property, get_private_value,
    set_private_value, v8_string, v8str,
};
use crate::webidl;
use anyhow::{Result, anyhow};
use moli_webapi_declare::WebApiObject;

use super::bindings::{
    ConstructorApplyArgs, ConstructorConstructArgs, ReflectIntrinsics, callback_data,
};
use super::overrides::current_date_locale_overrides;

mod locales;
use locales::LocaleIntrinsics;

const INTL_DEFAULT_LOCALE_SLOT: &str = "__moliIntlDefaultLocale";

const INTL_CONSTRUCTORS: &[&str] = &[
    "Collator",
    "DateTimeFormat",
    "DisplayNames",
    "DurationFormat",
    "ListFormat",
    "NumberFormat",
    "PluralRules",
    "RelativeTimeFormat",
    "Segmenter",
];

#[derive(Clone, Copy, WebApiObject, webidl::WebIdlDictionary)]
#[webapi(plain, data_properties)]
struct IntlConstructorData<'s> {
    #[webidl(required)]
    reflect: v8::Local<'s, v8::Function>,
    #[webidl(required)]
    uses_timezone: bool,
    #[webidl(required)]
    canonicalize_locales: v8::Local<'s, v8::Function>,
    #[webidl(required)]
    supported_locales: v8::Local<'s, v8::Function>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct IntlConstructorHandler<'s> {
    apply_data: IntlConstructorData<'s>,
    construct_data: IntlConstructorData<'s>,
    #[webapi(method, length = 3, callback = intl_constructor_proxy_apply_callback, data = self.apply_data)]
    apply: (),
    #[webapi(method, length = 3, callback = intl_constructor_proxy_construct_callback, data = self.construct_data)]
    construct: (),
}

#[derive(WebApiObject)]
#[webapi(fragment)]
struct IntlResolvedOptionsDeclaration<'s> {
    original: v8::Local<'s, v8::Function>,
    #[webapi(method, length = 0, callback = intl_resolved_options_callback, data = self.original)]
    resolved_options: (),
}

#[derive(Clone, Copy, WebApiObject, webidl::WebIdlDictionary)]
#[webapi(plain, data_properties)]
struct TimezoneOptionsData<'s> {
    #[webidl(required)]
    original: v8::Local<'s, v8::Object>,
    #[webidl(required)]
    timezone: v8::Local<'s, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct TimezoneOptionsHandler<'s> {
    data: TimezoneOptionsData<'s>,
    #[webapi(method, length = 3, callback = intl_datetime_options_get_callback, data = self.data)]
    get: (),
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct DefaultTimezoneOptions<'s> {
    time_zone: v8::Local<'s, v8::Value>,
}

pub(super) fn install_intl_default_override_constructors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    // Chromium changes ICU's process-wide default and notifies every isolate.
    // Moli can host independently configured targets in one renderer process,
    // so a process-global ICU mutation would leak one target's emulation into
    // another. Keep the override context-local; native locale matching tries
    // the page's canonical requests before our fallback, and explicit timeZone
    // values win. The locale adapter is lazy to preserve native getter order.
    let Some(intl_value) = global.get(scope, v8str(scope, "Intl").into()) else {
        return Ok(());
    };
    let Ok(intl) = v8::Local::<v8::Object>::try_from(intl_value) else {
        return Ok(());
    };
    let Some(reflect) = ReflectIntrinsics::from_global(scope, global) else {
        return Ok(());
    };
    let canonicalize = intl
        .get(scope, v8str(scope, "getCanonicalLocales").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .ok_or_else(|| anyhow!("missing native Intl.getCanonicalLocales"))?;
    for &name in INTL_CONSTRUCTORS {
        let key = v8str(scope, name);
        let Some(original_value) = intl.get(scope, key.into()) else {
            continue;
        };
        let Ok(original) = v8::Local::<v8::Function>::try_from(original_value) else {
            continue;
        };
        if let Some(prototype) = original
            .get(scope, v8str(scope, "prototype").into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            install_intl_resolved_options_override(scope, prototype)?;
        }
        let uses_timezone = name == "DateTimeFormat";
        let supported = original
            .get(scope, v8str(scope, "supportedLocalesOf").into())
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
            .ok_or_else(|| anyhow!("missing native Intl.{name}.supportedLocalesOf"))?;
        if uses_timezone {
            locales::retain_date_locale_intrinsics(
                scope,
                global,
                LocaleIntrinsics::new(canonicalize, supported),
            )?;
        }
        let handler = IntlConstructorHandler::new(
            IntlConstructorData::new(reflect.apply, uses_timezone, canonicalize, supported),
            IntlConstructorData::new(reflect.construct, uses_timezone, canonicalize, supported),
        )
        .bind(scope)?;
        let Some(proxy) = v8::Proxy::new(scope, original.into(), handler) else {
            return Err(anyhow!("failed to create Intl.{name} constructor proxy"));
        };
        let _ = intl.set(scope, key.into(), proxy.into());
        if let Some(prototype) = original
            .get(scope, v8str(scope, "prototype").into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            let _ = prototype.set(scope, v8str(scope, "constructor").into(), proxy.into());
        }
    }
    Ok(())
}

fn install_intl_resolved_options_override<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let key = v8str(scope, "resolvedOptions");
    let Some(original) = prototype.get(scope, key.into()) else {
        return Ok(());
    };
    let Ok(original) = v8::Local::<v8::Function>::try_from(original) else {
        return Ok(());
    };
    IntlResolvedOptionsDeclaration::new(original).initialize(scope, prototype)?;
    Ok(())
}

fn intl_constructor_proxy_apply_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(data) = callback_data::<IntlConstructorData>(scope, args.data()) else {
        return;
    };
    let Some(args) = webidl::parse_args::<ConstructorApplyArgs>(scope, &args) else {
        return;
    };
    let locale_list = apply_intl_constructor_defaults(scope, args.arguments, data);
    let invoke_args = [args.target, args.receiver, args.arguments.into()];
    let receiver = v8::undefined(scope);
    if let Some(result) = call_script_visible_function(
        scope,
        data.reflect,
        receiver.into(),
        &invoke_args,
        "invoke Intl constructor through Reflect.apply",
    ) {
        tag_intl_default_locale(scope, result, locale_list);
        rv.set(result);
    }
}

fn intl_constructor_proxy_construct_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(data) = callback_data::<IntlConstructorData>(scope, args.data()) else {
        return;
    };
    let Some(args) = webidl::parse_args::<ConstructorConstructArgs>(scope, &args) else {
        return;
    };
    let locale_list = apply_intl_constructor_defaults(scope, args.arguments, data);
    let invoke_args = [args.target, args.arguments.into(), args.new_target];
    let receiver = v8::undefined(scope);
    if let Some(result) = call_script_visible_function(
        scope,
        data.reflect,
        receiver.into(),
        &invoke_args,
        "invoke Intl constructor through Reflect.construct",
    ) {
        tag_intl_default_locale(scope, result, locale_list);
        rv.set(result);
    }
}

fn apply_intl_constructor_defaults<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    arguments: v8::Local<'s, v8::Array>,
    data: IntlConstructorData<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let (locale_override, timezone_override) = current_date_locale_overrides(scope);
    let locale_list = locale_override.as_deref().and_then(|locale| {
        // The Proxy trap array is internal and dense. Reading beyond its
        // length would incorrectly consult the page's Array.prototype.
        let original = if arguments.length() == 0 {
            v8::undefined(scope).into()
        } else {
            arguments.get_index(scope, 0)?
        };
        let list = locales::defer_locale_list(
            scope,
            original,
            locale,
            LocaleIntrinsics::new(data.canonicalize_locales, data.supported_locales),
        )?;
        define_v8_array_data_property(scope, arguments, 0, list.into())?;
        Some(list)
    });
    if data.uses_timezone
        && let Some(timezone) = timezone_override.as_deref()
        && let Some(options) = intl_datetime_options_with_default_timezone(
            scope,
            (arguments.length() > 1)
                .then(|| arguments.get_index(scope, 1))
                .flatten(),
            timezone,
        )
    {
        let _ = define_v8_array_data_property(scope, arguments, 1, options);
    }
    locale_list
}

fn tag_intl_default_locale<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: v8::Local<'s, v8::Value>,
    locale_list: Option<v8::Local<'s, v8::Object>>,
) {
    // Supplying the emulated default as an explicit locale can minimize a
    // service's resolved tag (for example Collator may report `fr`). ICU's
    // overridden default, and therefore Chromium, retains `fr-FR`. Tag the
    // instance so resolvedOptions can expose that same default identity.
    if let Some(list) = locale_list
        && let Some(locale) = get_private_value(scope, list, locales::DEFAULT_LOCALE_SLOT)
        && let Ok(instance) = v8::Local::<v8::Object>::try_from(result)
    {
        set_private_value(scope, instance, INTL_DEFAULT_LOCALE_SLOT, locale);
    }
}

pub(super) fn intl_date_locales_with_default<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    original: v8::Local<'s, v8::Value>,
    locale: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    locales::defer_date_locale_list(scope, original, locale).map(Into::into)
}

fn intl_resolved_options_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(original) = v8::Local::<v8::Function>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    let Some(result) = call_script_visible_function(
        scope,
        original,
        args.this().into(),
        &[],
        "Intl resolvedOptions",
    ) else {
        return;
    };
    let Ok(options) = v8::Local::<v8::Object>::try_from(result) else {
        rv.set(result);
        return;
    };
    if let Some(locale) = get_private_value(scope, args.this(), INTL_DEFAULT_LOCALE_SLOT) {
        let _ = options.set(scope, v8str(scope, "locale").into(), locale);
    }
    rv.set(options.into());
}

pub(super) fn intl_datetime_options_with_default_timezone<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    original: Option<v8::Local<'s, v8::Value>>,
    timezone: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    if let Some(original) = original.filter(|value| !value.is_undefined()) {
        if original.is_null() {
            // Native code owns when null throws (an invalid Date, for example,
            // returns before consulting options). Other primitives are boxed
            // by CoerceOptionsToObject and must receive the default too.
            return Some(original);
        }
        let original = original.to_object(scope)?;
        // Do not inspect `timeZone` here. Reading it before V8 processes the
        // remaining options changes observable getter/Proxy ordering and can
        // read an explicit accessor twice. The transparent proxy supplies the
        // default exactly when V8 performs its ordinary [[Get]]. Do not use
        // the page's object as the Proxy target: substituting a default for a
        // frozen own `timeZone: undefined` would violate Proxy [[Get]]'s
        // SameValue invariant. The empty target has no such own properties;
        // callback data retains the original object only for forwarding reads.
        let target = v8::Object::new(scope);
        let timezone = v8_string(scope, timezone)?;
        let handler =
            TimezoneOptionsHandler::new(TimezoneOptionsData::new(original, timezone.into()))
                .bind(scope)
                .ok()?;
        return v8::Proxy::new(scope, target, handler).map(Into::into);
    }
    let timezone = v8_string(scope, timezone)?;
    DefaultTimezoneOptions::new(timezone.into())
        .bind(scope)
        .ok()
        .map(Into::into)
}

fn intl_datetime_options_get_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(data) = callback_data::<TimezoneOptionsData>(scope, args.data()) else {
        return;
    };
    let key = args.get(1);
    // The outer proxy is an implementation detail. Forward with the page's
    // original options object as receiver so an accessor observes exactly the
    // same `this` value it would have seen without emulation.
    let Some(value) = data.original.get(scope, key) else {
        return;
    };
    if key.strict_equals(v8str(scope, "timeZone").into()) && value.is_undefined() {
        rv.set(data.timezone);
    } else {
        rv.set(value);
    }
}
