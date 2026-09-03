use super::window_runtime::{
    current_date_locale_overrides, date_to_locale_date_string_callback,
    date_to_locale_string_callback, date_to_locale_time_string_callback,
    preserve_original_date_locale_methods,
};
use super::*;
use crate::util::{call_script_visible_function, get_private_value, set_private_value};
use anyhow::{Result, anyhow};
use moli_webapi_declare::WebApiObject;

const ORIGINAL_INTL_DATETIME_FORMAT_RESOLVED_OPTIONS_SLOT: &str =
    "__moliOriginalIntlDateTimeFormatResolvedOptions";

#[derive(Default, WebApiObject)]
#[webapi(interface = "Date")]
struct DateLocalePrototypeDeclaration {
    #[webapi(method, length = 0, callback = date_to_locale_string_callback)]
    to_locale_string: (),
    #[webapi(method, length = 0, callback = date_to_locale_date_string_callback)]
    to_locale_date_string: (),
    #[webapi(method, length = 0, callback = date_to_locale_time_string_callback)]
    to_locale_time_string: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Intl.DateTimeFormat")]
struct IntlDateTimeFormatPrototypeDeclaration {
    #[webapi(
        method = "resolvedOptions",
        length = 0,
        callback = intl_date_time_format_resolved_options_callback
    )]
    resolved_options: (),
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

    preserve_original_date_locale_methods(scope, global, date_proto);
    DateLocalePrototypeDeclaration::default()
        .initialize(scope, date_proto)
        .map_err(|err| anyhow!("failed to initialize Date locale methods: {err}"))?;
    install_intl_date_time_format_resolved_options(scope, global)
}

fn install_intl_date_time_format_resolved_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let Some(intl_value) = global.get(scope, v8str(scope, "Intl").into()) else {
        return Ok(());
    };
    let Ok(intl) = v8::Local::<v8::Object>::try_from(intl_value) else {
        return Ok(());
    };
    let Some(date_time_format_value) = intl.get(scope, v8str(scope, "DateTimeFormat").into())
    else {
        return Ok(());
    };
    let Ok(date_time_format) = v8::Local::<v8::Object>::try_from(date_time_format_value) else {
        return Ok(());
    };
    let Some(prototype_value) = date_time_format.get(scope, v8str(scope, "prototype").into())
    else {
        return Ok(());
    };
    let Ok(prototype) = v8::Local::<v8::Object>::try_from(prototype_value) else {
        return Ok(());
    };
    if get_private_value(
        scope,
        prototype,
        ORIGINAL_INTL_DATETIME_FORMAT_RESOLVED_OPTIONS_SLOT,
    )
    .is_none()
        && let Some(original) = prototype.get(scope, v8str(scope, "resolvedOptions").into())
    {
        set_private_value(
            scope,
            prototype,
            ORIGINAL_INTL_DATETIME_FORMAT_RESOLVED_OPTIONS_SLOT,
            original,
        );
    }
    IntlDateTimeFormatPrototypeDeclaration::default()
        .initialize(scope, prototype)
        .map_err(|err| anyhow!("failed to initialize Intl.DateTimeFormat locale methods: {err}"))
}

fn intl_date_time_format_resolved_options_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(original) = original_intl_date_time_format_resolved_options(scope) else {
        rv.set_undefined();
        return;
    };
    let Some(result) = call_script_visible_function(
        scope,
        original,
        args.this().into(),
        &[],
        "Intl.DateTimeFormat.resolvedOptions",
    ) else {
        return;
    };
    let Ok(options) = v8::Local::<v8::Object>::try_from(result) else {
        rv.set(result);
        return;
    };
    let (locale_override, timezone_override) = current_date_locale_overrides(scope);
    if let Some(locale) = locale_override.as_deref()
        && let Some(value) = v8_string(scope, locale)
    {
        let _ = options.set(scope, v8str(scope, "locale").into(), value.into());
    }
    if let Some(timezone) = timezone_override.as_deref()
        && let Some(value) = v8_string(scope, timezone)
    {
        let _ = options.set(scope, v8str(scope, "timeZone").into(), value.into());
    }
    rv.set(options.into());
}

fn original_intl_date_time_format_resolved_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Function>> {
    let global = scope.get_current_context().global(scope);
    let intl = global.get(scope, v8str(scope, "Intl").into())?;
    let intl = v8::Local::<v8::Object>::try_from(intl).ok()?;
    let date_time_format = intl.get(scope, v8str(scope, "DateTimeFormat").into())?;
    let date_time_format = v8::Local::<v8::Object>::try_from(date_time_format).ok()?;
    let prototype = date_time_format.get(scope, v8str(scope, "prototype").into())?;
    let prototype = v8::Local::<v8::Object>::try_from(prototype).ok()?;
    let original = get_private_value(
        scope,
        prototype,
        ORIGINAL_INTL_DATETIME_FORMAT_RESOLVED_OPTIONS_SLOT,
    )?;
    v8::Local::<v8::Function>::try_from(original).ok()
}
