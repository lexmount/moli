use super::*;
use crate::util::call_script_visible_function;

use super::super::{
    intl::intl_datetime_options_with_default_timezone, overrides::current_date_locale_overrides,
};

const DATE_LOCAL_FIELD_INTRINSICS: &[DateIntrinsic] = &[
    DateIntrinsic::GetFullYear,
    DateIntrinsic::GetMonth,
    DateIntrinsic::GetDate,
    DateIntrinsic::GetDay,
    DateIntrinsic::GetHours,
    DateIntrinsic::GetMinutes,
    DateIntrinsic::GetSeconds,
    DateIntrinsic::GetMilliseconds,
];

const DATE_LOCAL_SETTER_INTRINSICS: &[(DateIntrinsic, DateIntrinsic)] = &[
    (DateIntrinsic::SetDate, DateIntrinsic::SetUtcDate),
    (DateIntrinsic::SetFullYear, DateIntrinsic::SetUtcFullYear),
    (DateIntrinsic::SetHours, DateIntrinsic::SetUtcHours),
    (
        DateIntrinsic::SetMilliseconds,
        DateIntrinsic::SetUtcMilliseconds,
    ),
    (DateIntrinsic::SetMinutes, DateIntrinsic::SetUtcMinutes),
    (DateIntrinsic::SetMonth, DateIntrinsic::SetUtcMonth),
    (DateIntrinsic::SetSeconds, DateIntrinsic::SetUtcSeconds),
];

pub(super) fn date_to_locale_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_date_locale_method_result(
        scope,
        args,
        rv,
        DateIntrinsic::ToLocaleString,
        "Date.prototype.toLocaleString",
    );
}

pub(super) fn date_to_locale_date_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_date_locale_method_result(
        scope,
        args,
        rv,
        DateIntrinsic::ToLocaleDateString,
        "Date.prototype.toLocaleDateString",
    );
}

pub(super) fn date_to_locale_time_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_date_locale_method_result(
        scope,
        args,
        rv,
        DateIntrinsic::ToLocaleTimeString,
        "Date.prototype.toLocaleTimeString",
    );
}

fn set_date_locale_method_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    original: DateIntrinsic,
    action: &str,
) {
    let Some(original) = original_date_method(scope, original) else {
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

pub(super) fn date_get_timezone_offset_callback<'s>(
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
            DateIntrinsic::GetTimezoneOffset,
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

pub(super) fn date_local_field_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let field = args.data().uint32_value(scope).unwrap_or(u32::MAX);
    let (_, timezone_override) = current_date_locale_overrides(scope);
    if timezone_override.is_none()
        && let Some(&intrinsic) = DATE_LOCAL_FIELD_INTRINSICS.get(field as usize)
        && set_original_date_method_result(
            scope,
            &args,
            &mut rv,
            intrinsic,
            "Date local field getter",
        )
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

pub(super) fn date_local_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let setter = args.data().uint32_value(scope).unwrap_or(u32::MAX);
    let Some(&(local_intrinsic, utc_intrinsic)) = DATE_LOCAL_SETTER_INTRINSICS.get(setter as usize)
    else {
        rv.set(v8::Number::new(scope, f64::NAN).into());
        return;
    };
    let forwarded = (0..args.length())
        .map(|index| args.get(index))
        .collect::<Vec<_>>();
    let (_, timezone_override) = current_date_locale_overrides(scope);
    let Some(timezone) = timezone_override.as_deref() else {
        let Some(method) = original_date_method(scope, local_intrinsic) else {
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
    let Some(utc_setter) = original_date_method(scope, utc_intrinsic) else {
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
    let Some(set_time) = original_date_method(scope, DateIntrinsic::SetTime) else {
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

pub(super) fn date_to_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_date_local_string_result(
        scope,
        args,
        rv,
        moli_time::format_date_local_string,
        DateIntrinsic::ToString,
        "Date.prototype.toString",
    );
}

pub(super) fn date_to_date_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_date_local_string_result(
        scope,
        args,
        rv,
        moli_time::format_date_local_date_string,
        DateIntrinsic::ToDateString,
        "Date.prototype.toDateString",
    );
}

pub(super) fn date_to_time_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_date_local_string_result(
        scope,
        args,
        rv,
        moli_time::format_date_local_time_string,
        DateIntrinsic::ToTimeString,
        "Date.prototype.toTimeString",
    );
}

fn set_date_local_string_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    format: fn(f64, Option<&str>) -> String,
    original: DateIntrinsic,
    action: &str,
) {
    let (_, timezone_override) = current_date_locale_overrides(scope);
    if timezone_override.is_none()
        && set_original_date_method_result(scope, &args, &mut rv, original, action)
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
    intrinsic: DateIntrinsic,
    action: &str,
) -> bool {
    let Some(method) = original_date_method(scope, intrinsic) else {
        return false;
    };
    if let Some(value) =
        call_script_visible_function(scope, method, args.this().into(), &[], action)
    {
        rv.set(value);
    }
    true
}
