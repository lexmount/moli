use super::*;
use crate::util::call_script_visible_function;
use anyhow::{Result, anyhow};

use super::super::overrides::current_date_locale_overrides;
use super::parse_input::local_date_parse_input_as_utc;

pub(super) fn install_date_parse_override<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor: v8::Local<'s, v8::Function>,
) -> Result<()> {
    let Some(original) = original_date_method(scope, DateIntrinsic::Parse) else {
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

pub(super) fn install_date_constructor_proxy<'s>(
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
    let Some(date_now) = original_date_method(scope, DateIntrinsic::Now) else {
        return Ok(());
    };
    let Some(date_utc) = original_date_method(scope, DateIntrinsic::Utc) else {
        return Ok(());
    };
    let Some(date_parse) = original_date_method(scope, DateIntrinsic::Parse) else {
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
        let input = v8::Local::<v8::String>::try_from(input).map_err(|_| ())?;
        let epoch = parse_date_string(scope, date_parse, input, Some(timezone)).ok_or(())?;
        return Ok(Some(v8::Array::new_with_elements(scope, &[epoch])));
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
    let epoch_ms = epoch_for_local_wall_clock(wall_clock_utc_ms, timezone);
    let epoch_ms = v8::Number::new(scope, epoch_ms);
    v8::Array::new_with_elements(scope, &[epoch_ms.into()])
}

fn epoch_for_local_wall_clock(wall_clock_utc_ms: Option<f64>, timezone: &str) -> f64 {
    wall_clock_utc_ms
        .filter(|value| value.is_finite())
        .and_then(|value| moli_time::epoch_millis_for_local_wall_clock(value, timezone))
        .unwrap_or(f64::NAN)
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
    if let Some(parsed) = parse_date_string(scope, original, input, timezone_override.as_deref()) {
        rv.set(parsed);
    }
}

fn parse_date_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    original: v8::Local<'s, v8::Function>,
    input: v8::Local<'s, v8::String>,
    timezone: Option<&str>,
) -> Option<v8::Local<'s, v8::Value>> {
    let receiver = v8::undefined(scope);
    // V8 owns acceptance of both ISO and implementation-defined legacy input.
    // Never turn an originally invalid string into a valid one by adding a
    // suffix. The argument is already a String, so neither native parse invokes
    // the page's ToPrimitive/ToString hooks a second time.
    let parsed = call_script_visible_function(
        scope,
        original,
        receiver.into(),
        &[input.into()],
        "Date.parse",
    )?;
    let Some(timezone) = timezone else {
        return Some(parsed);
    };
    if !parsed.number_value(scope)?.is_finite() {
        return Some(parsed);
    }
    let Some(utc_input) = local_date_parse_input_as_utc(&input.to_rust_string_lossy(scope)) else {
        return Some(parsed);
    };
    let utc_input = v8_string(scope, &utc_input)?;
    let wall_clock = call_script_visible_function(
        scope,
        original,
        receiver.into(),
        &[utc_input.into()],
        "read local Date string fields through native UTC parsing",
    )?;
    let epoch = epoch_for_local_wall_clock(wall_clock.number_value(scope), timezone);
    Some(v8::Number::new(scope, epoch).into())
}
