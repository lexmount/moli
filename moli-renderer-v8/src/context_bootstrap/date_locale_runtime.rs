use super::window_runtime::current_date_locale_overrides;
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
    ("toLocaleString", "__moliOriginalDateToLocaleString"),
    ("toLocaleDateString", "__moliOriginalDateToLocaleDateString"),
    ("toLocaleTimeString", "__moliOriginalDateToLocaleTimeString"),
    ("getTimezoneOffset", "__moliOriginalDateGetTimezoneOffset"),
    ("getFullYear", "__moliOriginalDateGetFullYear"),
    ("getMonth", "__moliOriginalDateGetMonth"),
    ("getDate", "__moliOriginalDateGetDate"),
    ("getDay", "__moliOriginalDateGetDay"),
    ("getHours", "__moliOriginalDateGetHours"),
    ("getMinutes", "__moliOriginalDateGetMinutes"),
    ("getSeconds", "__moliOriginalDateGetSeconds"),
    ("getMilliseconds", "__moliOriginalDateGetMilliseconds"),
    ("setDate", "__moliOriginalDateSetDate"),
    ("setFullYear", "__moliOriginalDateSetFullYear"),
    ("setHours", "__moliOriginalDateSetHours"),
    ("setMilliseconds", "__moliOriginalDateSetMilliseconds"),
    ("setMinutes", "__moliOriginalDateSetMinutes"),
    ("setMonth", "__moliOriginalDateSetMonth"),
    ("setSeconds", "__moliOriginalDateSetSeconds"),
    ("setTime", "__moliOriginalDateSetTime"),
    ("setUTCDate", "__moliOriginalDateSetUTCDate"),
    ("setUTCFullYear", "__moliOriginalDateSetUTCFullYear"),
    ("setUTCHours", "__moliOriginalDateSetUTCHours"),
    ("setUTCMilliseconds", "__moliOriginalDateSetUTCMilliseconds"),
    ("setUTCMinutes", "__moliOriginalDateSetUTCMinutes"),
    ("setUTCMonth", "__moliOriginalDateSetUTCMonth"),
    ("setUTCSeconds", "__moliOriginalDateSetUTCSeconds"),
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
const ORIGINAL_DATE_LOCAL_SETTER_SLOTS: &[(&str, &str)] = &[
    ("__moliOriginalDateSetDate", "__moliOriginalDateSetUTCDate"),
    (
        "__moliOriginalDateSetFullYear",
        "__moliOriginalDateSetUTCFullYear",
    ),
    (
        "__moliOriginalDateSetHours",
        "__moliOriginalDateSetUTCHours",
    ),
    (
        "__moliOriginalDateSetMilliseconds",
        "__moliOriginalDateSetUTCMilliseconds",
    ),
    (
        "__moliOriginalDateSetMinutes",
        "__moliOriginalDateSetUTCMinutes",
    ),
    (
        "__moliOriginalDateSetMonth",
        "__moliOriginalDateSetUTCMonth",
    ),
    (
        "__moliOriginalDateSetSeconds",
        "__moliOriginalDateSetUTCSeconds",
    ),
];
const ORIGINAL_DATE_CONSTRUCTOR_SLOT: &str = "__moliOriginalDateConstructor";
const ORIGINAL_DATE_PARSE_SLOT: &str = "__moliOriginalDateParse";
const ORIGINAL_DATE_UTC_SLOT: &str = "__moliOriginalDateUtc";
const ORIGINAL_DATE_NOW_SLOT: &str = "__moliOriginalDateNow";
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
    #[webapi(method = "setDate", length = 1, callback = date_local_setter_callback, data = callback_data_index_value(scope, 0))]
    set_date: (),
    #[webapi(method = "setFullYear", length = 3, callback = date_local_setter_callback, data = callback_data_index_value(scope, 1))]
    set_full_year: (),
    #[webapi(method = "setHours", length = 4, callback = date_local_setter_callback, data = callback_data_index_value(scope, 2))]
    set_hours: (),
    #[webapi(method = "setMilliseconds", length = 1, callback = date_local_setter_callback, data = callback_data_index_value(scope, 3))]
    set_milliseconds: (),
    #[webapi(method = "setMinutes", length = 3, callback = date_local_setter_callback, data = callback_data_index_value(scope, 4))]
    set_minutes: (),
    #[webapi(method = "setMonth", length = 2, callback = date_local_setter_callback, data = callback_data_index_value(scope, 5))]
    set_month: (),
    #[webapi(method = "setSeconds", length = 2, callback = date_local_setter_callback, data = callback_data_index_value(scope, 6))]
    set_seconds: (),
}

pub(super) fn install_date_locale_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let Some(date_ctor_value) = get_private_value(scope, global, ORIGINAL_DATE_CONSTRUCTOR_SLOT)
        .or_else(|| global.get(scope, v8str(scope, "Date").into()))
    else {
        return Ok(());
    };
    let Ok(date_ctor) = v8::Local::<v8::Function>::try_from(date_ctor_value) else {
        return Ok(());
    };
    if get_private_value(scope, global, ORIGINAL_DATE_CONSTRUCTOR_SLOT).is_none() {
        set_private_value(
            scope,
            global,
            ORIGINAL_DATE_CONSTRUCTOR_SLOT,
            date_ctor.into(),
        );
    }
    let Some(date_proto_value) = date_ctor.get(scope, v8str(scope, "prototype").into()) else {
        return Ok(());
    };
    let Ok(date_proto) = v8::Local::<v8::Object>::try_from(date_proto_value) else {
        return Ok(());
    };

    preserve_original_date_methods(scope, global, date_proto);
    preserve_original_date_static_methods(scope, global, date_ctor);
    DateLocalePrototypeDeclaration::default()
        .initialize(scope, date_proto)
        .map_err(|err| anyhow!("failed to initialize Date locale methods: {err}"))?;
    install_date_parse_override(scope, date_ctor)?;
    install_date_constructor_proxy(scope, global, date_ctor, date_proto)?;
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

fn preserve_original_date_static_methods<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    constructor: v8::Local<'s, v8::Function>,
) {
    for (name, slot) in [
        ("parse", ORIGINAL_DATE_PARSE_SLOT),
        ("UTC", ORIGINAL_DATE_UTC_SLOT),
        ("now", ORIGINAL_DATE_NOW_SLOT),
    ] {
        if get_private_value(scope, global, slot).is_some() {
            continue;
        }
        if let Some(method) = constructor.get(scope, v8str(scope, name).into()) {
            set_private_value(scope, global, slot, method);
        }
    }
}

fn install_date_parse_override<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor: v8::Local<'s, v8::Function>,
) -> Result<()> {
    let Some(original) = original_date_method(scope, ORIGINAL_DATE_PARSE_SLOT) else {
        return Ok(());
    };
    let Some(wrapper) = v8::Function::builder(date_parse_callback)
        .data(original.into())
        .length(1)
        .build(scope)
    else {
        return Err(anyhow!("failed to create Date.parse override"));
    };
    let name = v8str(scope, "parse");
    wrapper.set_name(name);
    let _ = constructor.set(scope, name.into(), wrapper.into());
    Ok(())
}

fn install_date_constructor_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    original: v8::Local<'s, v8::Function>,
    prototype: v8::Local<'s, v8::Object>,
) -> Result<()> {
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
    let Some(date_now) = original_date_method(scope, ORIGINAL_DATE_NOW_SLOT) else {
        return Ok(());
    };
    let Some(date_utc) = original_date_method(scope, ORIGINAL_DATE_UTC_SLOT) else {
        return Ok(());
    };
    let Some(date_parse) = original_date_method(scope, ORIGINAL_DATE_PARSE_SLOT) else {
        return Ok(());
    };

    let apply_data = v8::Array::new(scope, 2);
    let _ = apply_data.set_index(scope, 0, reflect_apply.into());
    let _ = apply_data.set_index(scope, 1, date_now.into());
    let Some(apply) = v8::Function::builder(date_constructor_proxy_apply_callback)
        .data(apply_data.into())
        .length(3)
        .build(scope)
    else {
        return Err(anyhow!("failed to create Date constructor apply trap"));
    };

    let construct_data = v8::Array::new(scope, 3);
    let _ = construct_data.set_index(scope, 0, reflect_construct.into());
    let _ = construct_data.set_index(scope, 1, date_utc.into());
    let _ = construct_data.set_index(scope, 2, date_parse.into());
    let Some(construct) = v8::Function::builder(date_constructor_proxy_construct_callback)
        .data(construct_data.into())
        .length(3)
        .build(scope)
    else {
        return Err(anyhow!("failed to create Date constructor construct trap"));
    };

    let handler = v8::Object::new(scope);
    let _ = handler.set(scope, v8str(scope, "apply").into(), apply.into());
    let _ = handler.set(scope, v8str(scope, "construct").into(), construct.into());
    let Some(proxy) = v8::Proxy::new(scope, original.into(), handler) else {
        return Err(anyhow!("failed to create Date constructor proxy"));
    };
    let _ = global.set(scope, v8str(scope, "Date").into(), proxy.into());
    let _ = prototype.set(scope, v8str(scope, "constructor").into(), proxy.into());
    Ok(())
}

fn date_constructor_proxy_apply_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Array>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    let Some(reflect_apply) = data
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        rv.set_undefined();
        return;
    };
    let Some(date_now) = data
        .get_index(scope, 1)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        rv.set_undefined();
        return;
    };
    let (_, timezone_override) = current_date_locale_overrides(scope);
    let Some(timezone) = timezone_override.as_deref() else {
        let invoke_args = [args.get(0), args.get(1), args.get(2)];
        let receiver = v8::undefined(scope);
        if let Some(result) = call_script_visible_function(
            scope,
            reflect_apply,
            receiver.into(),
            &invoke_args,
            "invoke Date through Reflect.apply",
        ) {
            rv.set(result);
        }
        return;
    };

    // Calling Date as a function ignores every argument and returns the same
    // local representation as a freshly constructed Date's toString(). Use
    // the retained native clock, then apply only the target-local timezone.
    let receiver = v8::undefined(scope);
    let Some(now) = call_script_visible_function(
        scope,
        date_now,
        receiver.into(),
        &[],
        "read Date.now for the Date function",
    ) else {
        return;
    };
    let Some(timestamp_ms) = now.number_value(scope) else {
        return;
    };
    let value = moli_time::format_date_local_string(timestamp_ms, Some(timezone));
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    }
}

fn date_constructor_proxy_construct_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Array>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    let Some(reflect_construct) = data
        .get_index(scope, 0)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        rv.set_undefined();
        return;
    };
    let Some(date_utc) = data
        .get_index(scope, 1)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        rv.set_undefined();
        return;
    };
    let Some(date_parse) = data
        .get_index(scope, 2)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        rv.set_undefined();
        return;
    };
    let Ok(arguments) = v8::Local::<v8::Array>::try_from(args.get(1)) else {
        rv.set_undefined();
        return;
    };
    let (_, timezone_override) = current_date_locale_overrides(scope);
    let replacement = match timezone_override.as_deref() {
        Some(timezone) => match date_constructor_timezone_arguments(
            scope, arguments, date_utc, date_parse, timezone,
        ) {
            Ok(replacement) => replacement,
            Err(()) => return,
        },
        None => None,
    };
    let arguments = replacement.unwrap_or(arguments);
    let invoke_args = [args.get(0), arguments.into(), args.get(2)];
    let receiver = v8::undefined(scope);
    if let Some(result) = call_script_visible_function(
        scope,
        reflect_construct,
        receiver.into(),
        &invoke_args,
        "invoke Date through Reflect.construct",
    ) {
        rv.set(result);
    }
}

fn date_constructor_timezone_arguments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    arguments: v8::Local<'s, v8::Array>,
    date_utc: v8::Local<'s, v8::Function>,
    date_parse: v8::Local<'s, v8::Function>,
    timezone: &str,
) -> Result<Option<v8::Local<'s, v8::Array>>, ()> {
    let wall_clock = if arguments.length() >= 2 {
        let forwarded = (0..arguments.length())
            .filter_map(|index| arguments.get_index(scope, index))
            .collect::<Vec<_>>();
        let receiver = v8::undefined(scope);
        call_script_visible_function(
            scope,
            date_utc,
            receiver.into(),
            &forwarded,
            "normalize local Date constructor fields with Date.UTC",
        )
        .ok_or(())?
    } else if arguments.length() == 1 {
        let input = arguments.get_index(scope, 0).ok_or(())?;
        if !input.is_string() {
            return Ok(None);
        }
        let input = input.to_rust_string_lossy(scope);
        let Some(utc_input) = local_date_parse_input_as_utc(&input) else {
            return Ok(None);
        };
        let utc_input = v8_string(scope, &utc_input).ok_or(())?;
        let receiver = v8::undefined(scope);
        call_script_visible_function(
            scope,
            date_parse,
            receiver.into(),
            &[utc_input.into()],
            "parse a local Date constructor string as UTC fields",
        )
        .ok_or(())?
    } else {
        return Ok(None);
    };
    Ok(Some(single_date_epoch_argument(
        scope,
        wall_clock.number_value(scope),
        timezone,
    )))
}

fn single_date_epoch_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wall_clock_utc_ms: Option<f64>,
    timezone: &str,
) -> v8::Local<'s, v8::Array> {
    let epoch_ms = wall_clock_utc_ms
        .filter(|value| value.is_finite())
        .and_then(|value| moli_time::epoch_millis_for_local_wall_clock(value, timezone))
        .unwrap_or(f64::NAN);
    let epoch_ms = v8::Number::new(scope, epoch_ms);
    v8::Array::new_with_elements(scope, &[epoch_ms.into()])
}

fn date_parse_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(original) = v8::Local::<v8::Function>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    // Date.parse performs ToString exactly once. Convert here before deciding
    // whether the string denotes local time so observable conversion hooks are
    // neither skipped nor invoked twice.
    let Some(input) = args.get(0).to_string(scope) else {
        return;
    };
    let (_, timezone_override) = current_date_locale_overrides(scope);
    let utc_input = timezone_override
        .as_deref()
        .and_then(|_| local_date_parse_input_as_utc(&input.to_rust_string_lossy(scope)));
    let parse_input: v8::Local<'s, v8::Value> = match utc_input.as_deref() {
        Some(utc_input) => {
            let Some(utc_input) = v8_string(scope, utc_input) else {
                return;
            };
            utc_input.into()
        }
        None => input.into(),
    };
    let receiver = v8::undefined(scope);
    let Some(parsed) = call_script_visible_function(
        scope,
        original,
        receiver.into(),
        &[parse_input],
        "Date.parse",
    ) else {
        return;
    };
    let Some(timezone) = timezone_override.as_deref() else {
        rv.set(parsed);
        return;
    };
    if utc_input.is_none() {
        rv.set(parsed);
        return;
    }
    let epoch = single_date_epoch_argument(scope, parsed.number_value(scope), timezone);
    if let Some(epoch) = epoch.get_index(scope, 0) {
        rv.set(epoch);
    }
}

fn local_date_parse_input_as_utc(input: &str) -> Option<String> {
    let input = input.trim();
    if input.is_empty() || iso_date_only_uses_utc(input) {
        return None;
    }
    let lower = input.to_ascii_lowercase();
    if lower.ends_with('z') || lower.contains("gmt") || lower.contains("utc") {
        return None;
    }
    let time_start = input
        .find(['T', 't'])
        .or_else(|| input.find(':'))
        .unwrap_or(input.len());
    if input[time_start..].contains(['+', '-']) {
        return None;
    }
    if input.contains(['T', 't']) {
        Some(format!("{input}Z"))
    } else {
        // Legacy date grammars generally recognize a UTC suffix more reliably
        // than a trailing ISO `Z`.
        Some(format!("{input} UTC"))
    }
}

fn iso_date_only_uses_utc(input: &str) -> bool {
    let (year_len, rest) = match input.as_bytes().first() {
        Some(b'+' | b'-') if input.len() >= 7 => (7, &input[7..]),
        _ if input.len() >= 4 => (4, &input[4..]),
        _ => return false,
    };
    if !input.as_bytes()[..year_len]
        .iter()
        .enumerate()
        .all(|(index, byte)| (index == 0 && matches!(byte, b'+' | b'-')) || byte.is_ascii_digit())
    {
        return false;
    }
    rest.is_empty()
        || (rest.len() == 3
            && rest.as_bytes()[0] == b'-'
            && rest.as_bytes()[1..3].iter().all(u8::is_ascii_digit))
        || (rest.len() == 6
            && rest.as_bytes()[0] == b'-'
            && rest.as_bytes()[3] == b'-'
            && rest.as_bytes()[1..3].iter().all(u8::is_ascii_digit)
            && rest.as_bytes()[4..6].iter().all(u8::is_ascii_digit))
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

fn date_to_locale_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_date_locale_method_result(
        scope,
        args,
        rv,
        "__moliOriginalDateToLocaleString",
        "Date.prototype.toLocaleString",
    );
}

fn date_to_locale_date_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_date_locale_method_result(
        scope,
        args,
        rv,
        "__moliOriginalDateToLocaleDateString",
        "Date.prototype.toLocaleDateString",
    );
}

fn date_to_locale_time_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_date_locale_method_result(
        scope,
        args,
        rv,
        "__moliOriginalDateToLocaleTimeString",
        "Date.prototype.toLocaleTimeString",
    );
}

fn set_date_locale_method_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    original_slot: &str,
    action: &str,
) {
    let Some(original) = original_date_method(scope, original_slot) else {
        rv.set_undefined();
        return;
    };
    let (locale_override, timezone_override) = current_date_locale_overrides(scope);
    let mut forwarded = (0..args.length())
        .map(|index| args.get(index))
        .collect::<Vec<_>>();
    if let Some(locale) = locale_override.as_deref()
        && forwarded.first().is_none_or(|value| value.is_undefined())
        && let Some(locale) = v8_string(scope, locale)
    {
        if forwarded.is_empty() {
            forwarded.push(locale.into());
        } else {
            forwarded[0] = locale.into();
        }
    }
    if let Some(timezone) = timezone_override.as_deref()
        && let Some(options) =
            intl_datetime_options_with_default_timezone(scope, forwarded.get(1).copied(), timezone)
    {
        while forwarded.len() < 2 {
            forwarded.push(v8::undefined(scope).into());
        }
        forwarded[1] = options;
    }
    if let Some(value) =
        call_script_visible_function(scope, original, args.this().into(), &forwarded, action)
    {
        rv.set(value);
    }
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

fn date_local_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let setter = args.data().uint32_value(scope).unwrap_or(u32::MAX);
    let Some((local_slot, utc_slot)) = ORIGINAL_DATE_LOCAL_SETTER_SLOTS.get(setter as usize) else {
        rv.set(v8::Number::new(scope, f64::NAN).into());
        return;
    };
    let forwarded = (0..args.length())
        .map(|index| args.get(index))
        .collect::<Vec<_>>();
    let (_, timezone_override) = current_date_locale_overrides(scope);
    let Some(timezone) = timezone_override.as_deref() else {
        let Some(method) = original_date_method(scope, local_slot) else {
            rv.set_undefined();
            return;
        };
        if let Some(value) = call_script_visible_function(
            scope,
            method,
            args.this().into(),
            &forwarded,
            "Date local field setter",
        ) {
            rv.set(value);
        }
        return;
    };
    let Ok(date) = v8::Local::<v8::Date>::try_from(args.this()) else {
        throw_type_error(scope, "Date method called on incompatible receiver.");
        return;
    };
    let wall_clock =
        moli_time::local_wall_clock_as_utc_millis(date.value_of(), timezone).unwrap_or(f64::NAN);
    let Some(temporary) = v8::Date::new(scope, wall_clock) else {
        rv.set(v8::Number::new(scope, f64::NAN).into());
        return;
    };
    let Some(utc_setter) = original_date_method(scope, utc_slot) else {
        rv.set_undefined();
        return;
    };
    let Some(next_wall_clock) = call_script_visible_function(
        scope,
        utc_setter,
        temporary.into(),
        &forwarded,
        "normalize a Date local setter through its UTC counterpart",
    ) else {
        return;
    };
    let epoch_ms = next_wall_clock
        .number_value(scope)
        .filter(|value| value.is_finite())
        .and_then(|value| moli_time::epoch_millis_for_local_wall_clock(value, timezone))
        .unwrap_or(f64::NAN);
    let Some(set_time) = original_date_method(scope, "__moliOriginalDateSetTime") else {
        rv.set_undefined();
        return;
    };
    let epoch_ms = v8::Number::new(scope, epoch_ms);
    if let Some(value) = call_script_visible_function(
        scope,
        set_time,
        args.this().into(),
        &[epoch_ms.into()],
        "commit a Date local setter",
    ) {
        rv.set(value);
    }
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
    let Some(method) = original_date_method(scope, slot) else {
        return false;
    };
    if let Some(value) =
        call_script_visible_function(scope, method, args.this().into(), &[], action)
    {
        rv.set(value);
    }
    true
}

fn original_date_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Function>> {
    let global = scope.get_current_context().global(scope);
    get_private_value(scope, global, slot)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
}
