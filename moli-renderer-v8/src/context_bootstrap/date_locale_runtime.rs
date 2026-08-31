use super::window_runtime::{
    current_date_locale_overrides, date_to_locale_date_string_callback,
    date_to_locale_string_callback, date_to_locale_time_string_callback,
};
use super::*;
use crate::util::{
    call_script_visible_function, callback_data_index_value, get_private_value, set_private_value,
};
use anyhow::{Result, anyhow};
use moli_webapi_declare::WebApiObject;

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

const ORIGINAL_DATE_METHODS: &[(&str, &str)] = &[
    ("toString", "__moliOriginalDateToString"),
    ("toDateString", "__moliOriginalDateToDateString"),
    ("toTimeString", "__moliOriginalDateToTimeString"),
    ("getTimezoneOffset", "__moliOriginalDateGetTimezoneOffset"),
    ("getFullYear", "__moliOriginalDateGetFullYear"),
    ("getMonth", "__moliOriginalDateGetMonth"),
    ("getDate", "__moliOriginalDateGetDate"),
    ("getDay", "__moliOriginalDateGetDay"),
    ("getHours", "__moliOriginalDateGetHours"),
    ("getMinutes", "__moliOriginalDateGetMinutes"),
    ("getSeconds", "__moliOriginalDateGetSeconds"),
    ("getMilliseconds", "__moliOriginalDateGetMilliseconds"),
];

const ORIGINAL_DATE_LOCAL_FIELD_SLOTS: &[&str] = &[
    "__moliOriginalDateGetFullYear",
    "__moliOriginalDateGetMonth",
    "__moliOriginalDateGetDate",
    "__moliOriginalDateGetDay",
    "__moliOriginalDateGetHours",
    "__moliOriginalDateGetMinutes",
    "__moliOriginalDateGetSeconds",
    "__moliOriginalDateGetMilliseconds",
];
const ORIGINAL_INTL_RESOLVED_OPTIONS_SLOT: &str = "__moliOriginalIntlResolvedOptions";
const INTL_DEFAULT_LOCALE_SLOT: &str = "__moliIntlDefaultLocale";

#[derive(Default, WebApiObject)]
#[webapi(interface = "Date")]
struct DateLocalePrototypeDeclaration {
    #[webapi(method = "toString", length = 0, callback = date_to_string_callback)]
    to_string: (),
    #[webapi(method = "toDateString", length = 0, callback = date_to_date_string_callback)]
    to_date_string: (),
    #[webapi(method = "toTimeString", length = 0, callback = date_to_time_string_callback)]
    to_time_string: (),
    #[webapi(method, length = 0, callback = date_to_locale_string_callback)]
    to_locale_string: (),
    #[webapi(method, length = 0, callback = date_to_locale_date_string_callback)]
    to_locale_date_string: (),
    #[webapi(method, length = 0, callback = date_to_locale_time_string_callback)]
    to_locale_time_string: (),
    #[webapi(
        method = "getTimezoneOffset",
        length = 0,
        callback = date_get_timezone_offset_callback
    )]
    get_timezone_offset: (),
    #[webapi(method = "getFullYear", length = 0, callback = date_local_field_callback, data = callback_data_index_value(scope, 0))]
    get_full_year: (),
    #[webapi(method = "getMonth", length = 0, callback = date_local_field_callback, data = callback_data_index_value(scope, 1))]
    get_month: (),
    #[webapi(method = "getDate", length = 0, callback = date_local_field_callback, data = callback_data_index_value(scope, 2))]
    get_date: (),
    #[webapi(method = "getDay", length = 0, callback = date_local_field_callback, data = callback_data_index_value(scope, 3))]
    get_day: (),
    #[webapi(method = "getHours", length = 0, callback = date_local_field_callback, data = callback_data_index_value(scope, 4))]
    get_hours: (),
    #[webapi(method = "getMinutes", length = 0, callback = date_local_field_callback, data = callback_data_index_value(scope, 5))]
    get_minutes: (),
    #[webapi(method = "getSeconds", length = 0, callback = date_local_field_callback, data = callback_data_index_value(scope, 6))]
    get_seconds: (),
    #[webapi(method = "getMilliseconds", length = 0, callback = date_local_field_callback, data = callback_data_index_value(scope, 7))]
    get_milliseconds: (),
}

pub(super) fn install_date_locale_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let Some(date_ctor_value) = global.get(scope, v8str(scope, "Date").into()) else {
        return Ok(());
    };
    let Ok(date_ctor) = v8::Local::<v8::Object>::try_from(date_ctor_value) else {
        return Ok(());
    };
    let Some(date_proto_value) = date_ctor.get(scope, v8str(scope, "prototype").into()) else {
        return Ok(());
    };
    let Ok(date_proto) = v8::Local::<v8::Object>::try_from(date_proto_value) else {
        return Ok(());
    };

    preserve_original_date_methods(scope, global, date_proto);
    DateLocalePrototypeDeclaration::default()
        .initialize(scope, date_proto)
        .map_err(|err| anyhow!("failed to initialize Date locale methods: {err}"))?;
    install_intl_default_override_constructors(scope, global)
}

fn preserve_original_date_methods<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    prototype: v8::Local<'s, v8::Object>,
) {
    for (name, slot) in ORIGINAL_DATE_METHODS {
        if get_private_value(scope, global, slot).is_some() {
            continue;
        }
        if let Some(method) = prototype.get(scope, v8str(scope, name).into()) {
            set_private_value(scope, global, slot, method);
        }
    }
}

fn install_intl_default_override_constructors<'s>(
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
        let data = v8::Array::new(scope, 2);
        let _ = data.set_index(scope, 0, original.into());
        let kind = v8::Integer::new_from_unsigned(scope, index as u32);
        let _ = data.set_index(scope, 1, kind.into());
        let length = original
            .get(scope, v8str(scope, "length").into())
            .and_then(|value| value.int32_value(scope))
            .unwrap_or(0);
        let Some(wrapper) = v8::Function::builder(intl_constructor_with_defaults_callback)
            .data(data.into())
            .length(length)
            .build(scope)
        else {
            return Err(anyhow!("failed to create Intl.{name} override wrapper"));
        };
        wrapper.set_name(key);
        for property in ["prototype", "supportedLocalesOf"] {
            let property_key = v8str(scope, property);
            if let Some(value) = original.get(scope, property_key.into()) {
                let _ = wrapper.set(scope, property_key.into(), value);
            }
        }
        let _ = intl.set(scope, key.into(), wrapper.into());
        if let Some(prototype) = original
            .get(scope, v8str(scope, "prototype").into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            let _ = prototype.set(scope, v8str(scope, "constructor").into(), wrapper.into());
        }
    }
    Ok(())
}

fn install_intl_resolved_options_override<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let key = v8str(scope, "resolvedOptions");
    let original = if let Some(original) =
        get_private_value(scope, prototype, ORIGINAL_INTL_RESOLVED_OPTIONS_SLOT)
    {
        original
    } else {
        let Some(original) = prototype.get(scope, key.into()) else {
            return Ok(());
        };
        set_private_value(
            scope,
            prototype,
            ORIGINAL_INTL_RESOLVED_OPTIONS_SLOT,
            original,
        );
        original
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

fn intl_constructor_with_defaults_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((original, kind)) = intl_constructor_callback_data(scope, args.data()) else {
        rv.set_undefined();
        return;
    };
    let (locale_override, timezone_override) = current_date_locale_overrides(scope);
    let mut forwarded = (0..args.length())
        .map(|index| args.get(index))
        .collect::<Vec<_>>();
    let mut applied_default_locale = None;
    if let Some(locale) = locale_override.as_deref()
        && forwarded.first().is_none_or(|value| value.is_undefined())
        && let Some(locale) = v8_string(scope, locale)
    {
        applied_default_locale = locale_override;
        if forwarded.is_empty() {
            forwarded.push(locale.into());
        } else {
            forwarded[0] = locale.into();
        }
    }
    if kind.uses_timezone()
        && let Some(timezone) = timezone_override.as_deref()
        && let Some(options) =
            intl_datetime_options_with_default_timezone(scope, forwarded.get(1).copied(), timezone)
    {
        while forwarded.len() < 2 {
            forwarded.push(v8::undefined(scope).into());
        }
        forwarded[1] = options;
    }
    let result = if args.is_construct_call() {
        original
            .new_instance(scope, &forwarded)
            .map(v8::Local::<v8::Value>::from)
    } else {
        crate::util::call_script_visible_function(
            scope,
            original,
            args.this().into(),
            &forwarded,
            "invoke the retained native Intl constructor",
        )
    };
    if let Some(result) = result {
        // Supplying the emulated default as an explicit locale can minimize a
        // service's resolved tag (for example Collator may report `fr`). ICU's
        // overridden default, and therefore Chromium, retains `fr-FR`. Tag the
        // instance so resolvedOptions can expose that same default identity.
        if let Some(locale) = applied_default_locale.as_deref()
            && let Ok(instance) = v8::Local::<v8::Object>::try_from(result)
            && let Some(locale) = v8_string(scope, locale)
        {
            set_private_value(scope, instance, INTL_DEFAULT_LOCALE_SLOT, locale.into());
        }
        rv.set(result);
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
) -> Option<(v8::Local<'s, v8::Function>, IntlConstructorKind)> {
    let data = v8::Local::<v8::Array>::try_from(value).ok()?;
    let original = data
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let index = data.get_index(scope, 1)?.uint32_value(scope)?;
    Some((original, IntlConstructorKind::from_index(index)?))
}

fn intl_datetime_options_with_default_timezone<'s>(
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
        let object = original.to_object(scope)?;
        let configured_timezone = object.get(scope, key.into())?;
        if !configured_timezone.is_undefined() {
            return Some(original);
        }
        let options = v8::Object::new(scope);
        let _ = options.set_prototype(scope, object.into());
        let timezone = v8_string(scope, timezone)?;
        let _ = options.set(scope, key.into(), timezone.into());
        return Some(options.into());
    }
    let options = v8::Object::new(scope);
    let timezone = v8_string(scope, timezone)?;
    let _ = options.set(scope, key.into(), timezone.into());
    Some(options.into())
}

fn date_get_timezone_offset_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let (_, timezone_override) = current_date_locale_overrides(scope);
    // Preserve V8's complete native Date behavior outside emulation. The
    // target-local path below exists because V8's Date cache is also driven by
    // process-global timezone state.
    if timezone_override.is_none()
        && set_original_date_method_result(
            scope,
            &args,
            &mut rv,
            "__moliOriginalDateGetTimezoneOffset",
            "Date.prototype.getTimezoneOffset",
        )
    {
        return;
    }
    let Ok(date) = v8::Local::<v8::Date>::try_from(args.this()) else {
        throw_type_error(
            scope,
            "Method Date.prototype.getTimezoneOffset called on incompatible receiver.",
        );
        return;
    };
    let timestamp = date.value_of();
    let offset_minutes = timezone_override
        .as_deref()
        .and_then(|timezone| moli_time::time_zone_offset_seconds(timestamp, timezone))
        .or_else(|| moli_time::local_time_zone_offset_seconds(timestamp))
        .map(|seconds| -f64::from(seconds) / 60.0)
        .unwrap_or(f64::NAN);
    rv.set(v8::Number::new(scope, offset_minutes).into());
}

fn date_local_field_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let field = args.data().uint32_value(scope).unwrap_or(u32::MAX);
    let (_, timezone_override) = current_date_locale_overrides(scope);
    if timezone_override.is_none()
        && let Some(slot) = ORIGINAL_DATE_LOCAL_FIELD_SLOTS.get(field as usize)
        && set_original_date_method_result(scope, &args, &mut rv, slot, "Date local field getter")
    {
        return;
    }
    let Ok(date) = v8::Local::<v8::Date>::try_from(args.this()) else {
        throw_type_error(scope, "Date method called on incompatible receiver.");
        return;
    };
    let Some(fields) = moli_time::local_date_fields(date.value_of(), timezone_override.as_deref())
    else {
        rv.set(v8::Number::new(scope, f64::NAN).into());
        return;
    };
    let value = match field {
        0 => f64::from(fields.year),
        1 => f64::from(fields.month_zero_based),
        2 => f64::from(fields.day),
        3 => f64::from(fields.weekday_sunday_zero),
        4 => f64::from(fields.hour),
        5 => f64::from(fields.minute),
        6 => f64::from(fields.second),
        7 => f64::from(fields.millisecond),
        _ => f64::NAN,
    };
    rv.set(v8::Number::new(scope, value).into());
}

fn date_to_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_date_local_string_result(
        scope,
        args,
        rv,
        moli_time::format_date_local_string,
        "__moliOriginalDateToString",
        "Date.prototype.toString",
    );
}

fn date_to_date_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_date_local_string_result(
        scope,
        args,
        rv,
        moli_time::format_date_local_date_string,
        "__moliOriginalDateToDateString",
        "Date.prototype.toDateString",
    );
}

fn date_to_time_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_date_local_string_result(
        scope,
        args,
        rv,
        moli_time::format_date_local_time_string,
        "__moliOriginalDateToTimeString",
        "Date.prototype.toTimeString",
    );
}

fn set_date_local_string_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    format: fn(f64, Option<&str>) -> String,
    original_slot: &str,
    action: &str,
) {
    let (_, timezone_override) = current_date_locale_overrides(scope);
    if timezone_override.is_none()
        && set_original_date_method_result(scope, &args, &mut rv, original_slot, action)
    {
        return;
    }
    let Ok(date) = v8::Local::<v8::Date>::try_from(args.this()) else {
        throw_type_error(scope, "Date method called on incompatible receiver.");
        return;
    };
    let value = format(date.value_of(), timezone_override.as_deref());
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    }
}

fn set_original_date_method_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    rv: &mut v8::ReturnValue<'s, v8::Value>,
    slot: &str,
    action: &str,
) -> bool {
    let global = scope.get_current_context().global(scope);
    let Some(method) = get_private_value(scope, global, slot)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return false;
    };
    if let Some(value) =
        call_script_visible_function(scope, method, args.this().into(), &[], action)
    {
        rv.set(value);
    }
    true
}
