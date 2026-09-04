use super::*;
use crate::util::{call_script_visible_function, get_private_value, set_private_value};
use anyhow::{Result, anyhow};

use super::overrides::current_date_locale_overrides;

const INTL_DEFAULT_LOCALE_SLOT: &str = "__moliIntlDefaultLocale";

#[derive(Clone, Copy)]
enum IntlConstructorKind {
    Collator,
    DateTimeFormat,
    DisplayNames,
    DurationFormat,
    ListFormat,
    NumberFormat,
    PluralRules,
    RelativeTimeFormat,
    Segmenter,
}

impl IntlConstructorKind {
    const ALL: &[(Self, &str)] = &[
        (Self::Collator, "Collator"),
        (Self::DateTimeFormat, "DateTimeFormat"),
        (Self::DisplayNames, "DisplayNames"),
        (Self::DurationFormat, "DurationFormat"),
        (Self::ListFormat, "ListFormat"),
        (Self::NumberFormat, "NumberFormat"),
        (Self::PluralRules, "PluralRules"),
        (Self::RelativeTimeFormat, "RelativeTimeFormat"),
        (Self::Segmenter, "Segmenter"),
    ];

    fn from_index(index: u32) -> Option<Self> {
        Self::ALL.get(index as usize).map(|(kind, _)| *kind)
    }

    fn uses_timezone(self) -> bool {
        matches!(self, Self::DateTimeFormat)
    }
}
pub(super) fn install_intl_default_override_constructors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    // Chromium changes ICU's process-wide default and notifies every isolate.
    // Moli can host independently configured targets in one renderer process,
    // so a process-global ICU mutation would leak one target's emulation into
    // another. Keep the override context-local by injecting only omitted
    // constructor defaults; explicit page-provided locale/timeZone values win.
    let Some(intl_value) = global.get(scope, v8str(scope, "Intl").into()) else {
        return Ok(());
    };
    let Ok(intl) = v8::Local::<v8::Object>::try_from(intl_value) else {
        return Ok(());
    };
    let Some(reflect) = global
        .get(scope, v8str(scope, "Reflect").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return Ok(());
    };
    let Some(reflect_apply) = reflect
        .get(scope, v8str(scope, "apply").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return Ok(());
    };
    let Some(reflect_construct) = reflect
        .get(scope, v8str(scope, "construct").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return Ok(());
    };
    for (index, (_, name)) in IntlConstructorKind::ALL.iter().enumerate() {
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
        let kind = v8::Integer::new_from_unsigned(scope, index as u32);
        let apply_data = v8::Array::new(scope, 2);
        let _ = apply_data.set_index(scope, 0, kind.into());
        let _ = apply_data.set_index(scope, 1, reflect_apply.into());
        let Some(apply) = v8::Function::builder(intl_constructor_proxy_apply_callback)
            .data(apply_data.into())
            .length(3)
            .build(scope)
        else {
            return Err(anyhow!("failed to create Intl.{name} apply trap"));
        };
        let construct_data = v8::Array::new(scope, 2);
        let _ = construct_data.set_index(scope, 0, kind.into());
        let _ = construct_data.set_index(scope, 1, reflect_construct.into());
        let Some(construct) = v8::Function::builder(intl_constructor_proxy_construct_callback)
            .data(construct_data.into())
            .length(3)
            .build(scope)
        else {
            return Err(anyhow!("failed to create Intl.{name} construct trap"));
        };
        let handler = v8::Object::new(scope);
        let _ = handler.set(scope, v8str(scope, "apply").into(), apply.into());
        let _ = handler.set(scope, v8str(scope, "construct").into(), construct.into());
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
    let Some(wrapper) = v8::Function::builder(intl_resolved_options_callback)
        .data(original.into())
        .length(0)
        .build(scope)
    else {
        return Err(anyhow!("failed to create Intl resolvedOptions wrapper"));
    };
    wrapper.set_name(key);
    let _ = prototype.set(scope, key.into(), wrapper.into());
    Ok(())
}

fn intl_constructor_proxy_apply_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((kind, reflect_apply)) = intl_constructor_callback_data(scope, args.data()) else {
        rv.set_undefined();
        return;
    };
    let Ok(arguments) = v8::Local::<v8::Array>::try_from(args.get(2)) else {
        rv.set_undefined();
        return;
    };
    let applied_locale = apply_intl_constructor_defaults(scope, arguments, kind);
    let invoke_args = [args.get(0), args.get(1), arguments.into()];
    let receiver = v8::undefined(scope);
    if let Some(result) = call_script_visible_function(
        scope,
        reflect_apply,
        receiver.into(),
        &invoke_args,
        "invoke Intl constructor through Reflect.apply",
    ) {
        tag_intl_default_locale(scope, result, applied_locale.as_deref());
        rv.set(result);
    }
}

fn intl_constructor_proxy_construct_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((kind, reflect_construct)) = intl_constructor_callback_data(scope, args.data()) else {
        rv.set_undefined();
        return;
    };
    let Ok(arguments) = v8::Local::<v8::Array>::try_from(args.get(1)) else {
        rv.set_undefined();
        return;
    };
    let applied_locale = apply_intl_constructor_defaults(scope, arguments, kind);
    let invoke_args = [args.get(0), arguments.into(), args.get(2)];
    let receiver = v8::undefined(scope);
    if let Some(result) = call_script_visible_function(
        scope,
        reflect_construct,
        receiver.into(),
        &invoke_args,
        "invoke Intl constructor through Reflect.construct",
    ) {
        tag_intl_default_locale(scope, result, applied_locale.as_deref());
        rv.set(result);
    }
}

fn apply_intl_constructor_defaults<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    arguments: v8::Local<'s, v8::Array>,
    kind: IntlConstructorKind,
) -> Option<String> {
    let (locale_override, timezone_override) = current_date_locale_overrides(scope);
    let mut applied_locale = None;
    let locale_argument = arguments.get_index(scope, 0);
    if let Some(locale) = locale_override.as_deref()
        && locale_argument.is_none_or(|value| value.is_undefined())
        && let Some(value) = v8_string(scope, locale)
    {
        let _ = arguments.set_index(scope, 0, value.into());
        applied_locale = locale_override;
    }
    if kind.uses_timezone()
        && let Some(timezone) = timezone_override.as_deref()
        && let Some(options) = intl_datetime_options_with_default_timezone(
            scope,
            arguments.get_index(scope, 1),
            timezone,
        )
    {
        let _ = arguments.set_index(scope, 1, options);
    }
    applied_locale
}

fn tag_intl_default_locale(
    scope: &mut v8::PinScope<'_, '_>,
    result: v8::Local<'_, v8::Value>,
    locale: Option<&str>,
) {
    // Supplying the emulated default as an explicit locale can minimize a
    // service's resolved tag (for example Collator may report `fr`). ICU's
    // overridden default, and therefore Chromium, retains `fr-FR`. Tag the
    // instance so resolvedOptions can expose that same default identity.
    if let Some(locale) = locale
        && let Ok(instance) = v8::Local::<v8::Object>::try_from(result)
        && let Some(locale) = v8_string(scope, locale)
    {
        set_private_value(scope, instance, INTL_DEFAULT_LOCALE_SLOT, locale.into());
    }
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

fn intl_constructor_callback_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<(IntlConstructorKind, v8::Local<'s, v8::Function>)> {
    let data = v8::Local::<v8::Array>::try_from(value).ok()?;
    let index = data.get_index(scope, 0)?.uint32_value(scope)?;
    let builtin = data
        .get_index(scope, 1)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    Some((IntlConstructorKind::from_index(index)?, builtin))
}

pub(super) fn intl_datetime_options_with_default_timezone<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    original: Option<v8::Local<'s, v8::Value>>,
    timezone: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let key = v8str(scope, "timeZone");
    if let Some(original) = original.filter(|value| !value.is_undefined()) {
        if !original.is_object() {
            // Let the native constructor preserve its TypeError for null and
            // primitive options instead of converting them on the wrapper's
            // behalf.
            return Some(original);
        }
        let target = original.to_object(scope)?;
        // Do not inspect `timeZone` here. Reading it before V8 processes the
        // remaining options changes observable getter/Proxy ordering and can
        // read an explicit accessor twice. The transparent proxy supplies the
        // default exactly when V8 performs its ordinary [[Get]].
        let handler = v8::Object::new(scope);
        let timezone = v8_string(scope, timezone)?;
        let get = v8::Function::builder(intl_datetime_options_get_callback)
            .data(timezone.into())
            .length(3)
            .build(scope)?;
        let _ = handler.set(scope, v8str(scope, "get").into(), get.into());
        return v8::Proxy::new(scope, target, handler).map(Into::into);
    }
    let options = v8::Object::new(scope);
    let timezone = v8_string(scope, timezone)?;
    let _ = options.set(scope, key.into(), timezone.into());
    Some(options.into())
}

fn intl_datetime_options_get_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_undefined();
        return;
    };
    let key = args.get(1);
    // The outer proxy is an implementation detail. Forward with the page's
    // original options object as receiver so an accessor observes exactly the
    // same `this` value it would have seen without emulation.
    let Some(value) = target.get(scope, key) else {
        return;
    };
    if key.strict_equals(v8str(scope, "timeZone").into()) && value.is_undefined() {
        rv.set(args.data());
    } else {
        rv.set(value);
    }
}
