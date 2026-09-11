use super::{DateIntrinsic, original_date_method};
use crate::util::{call_script_visible_function, throw_type_error, v8_string, v8str};
use crate::webidl;
use anyhow::{Result, anyhow};
use moli_webapi_declare::WebApiObject;

use super::super::bindings::{
    ConstructorApplyArgs, ConstructorConstructArgs, ReflectIntrinsics, callback_data,
};
use super::super::overrides::current_date_locale_overrides;
use super::parse_input::local_date_parse_input_as_utc;

#[derive(WebApiObject)]
#[webapi(fragment)]
struct DateParseDeclaration<'s> {
    original: v8::Local<'s, v8::Function>,
    #[webapi(method, length = 1, callback = date_parse_callback, data = self.original)]
    parse: (),
}

#[derive(Clone, Copy, WebApiObject, webidl::WebIdlDictionary)]
#[webapi(plain, data_properties)]
struct DateApplyData<'s> {
    #[webidl(required)]
    reflect_apply: v8::Local<'s, v8::Function>,
    #[webidl(required)]
    date_now: v8::Local<'s, v8::Function>,
}

#[derive(Clone, Copy, WebApiObject, webidl::WebIdlDictionary)]
#[webapi(plain, data_properties)]
struct DateConstructData<'s> {
    #[webidl(required)]
    reflect_construct: v8::Local<'s, v8::Function>,
    #[webidl(required)]
    date_utc: v8::Local<'s, v8::Function>,
    #[webidl(required)]
    date_parse: v8::Local<'s, v8::Function>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct DateConstructorHandler<'s> {
    apply_data: DateApplyData<'s>,
    construct_data: DateConstructData<'s>,
    #[webapi(method, length = 3, callback = date_constructor_proxy_apply_callback, data = self.apply_data)]
    apply: (),
    #[webapi(method, length = 3, callback = date_constructor_proxy_construct_callback, data = self.construct_data)]
    construct: (),
}

pub(super) fn install_date_parse_override<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor: v8::Local<'s, v8::Function>,
) -> Result<()> {
    let Some(original) = original_date_method(scope, DateIntrinsic::Parse) else {
        return Ok(());
    };
    DateParseDeclaration::new(original).initialize(scope, constructor.into())?;
    Ok(())
}

pub(super) fn install_date_constructor_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    original: v8::Local<'s, v8::Function>,
    prototype: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let Some(reflect) = ReflectIntrinsics::from_global(scope, global) else {
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

    let handler = DateConstructorHandler::new(
        DateApplyData::new(reflect.apply, date_now),
        DateConstructData::new(reflect.construct, date_utc, date_parse),
    )
    .bind(scope)?;
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
    let Some(data) = callback_data::<DateApplyData>(scope, args.data()) else {
        return;
    };
    let Some(args) = webidl::parse_args::<ConstructorApplyArgs>(scope, &args) else {
        return;
    };
    let (_, timezone_override) = current_date_locale_overrides(scope);
    let Some(timezone) = timezone_override.as_deref() else {
        let invoke_args = [args.target, args.receiver, args.arguments.into()];
        let receiver = v8::undefined(scope);
        if let Some(result) = call_script_visible_function(
            scope,
            data.reflect_apply,
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
        data.date_now,
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
    let Some(data) = callback_data::<DateConstructData>(scope, args.data()) else {
        return;
    };
    let Some(args) = webidl::parse_args::<ConstructorConstructArgs>(scope, &args) else {
        return;
    };
    let (_, timezone_override) = current_date_locale_overrides(scope);
    let replacement = match timezone_override.as_deref() {
        Some(timezone) => match date_constructor_timezone_arguments(
            scope,
            args.arguments,
            data.date_utc,
            data.date_parse,
            timezone,
        ) {
            Ok(replacement) => replacement,
            Err(()) => return,
        },
        None => None,
    };
    let arguments = replacement.unwrap_or(args.arguments);
    let invoke_args = [args.target, arguments.into(), args.new_target];
    let receiver = v8::undefined(scope);
    if let Some(result) = call_script_visible_function(
        scope,
        data.reflect_construct,
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
        // Date copies another Date's [[DateValue]] without invoking even an
        // own @@toPrimitive getter. Every other object is converted with the
        // default hint, then strings are parsed and other primitives become
        // numeric epochs. Forward the primitive, never the original object,
        // so V8 cannot run the page's conversion hooks a second time.
        if input.is_date() {
            return Ok(None);
        }
        let input = date_constructor_primitive(scope, input).ok_or(())?;
        if !input.is_string() {
            return Ok(Some(v8::Array::new_with_elements(scope, &[input])));
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

/// ECMA-262 ToPrimitive with no preferred type (Date constructor step 3.b.i).
/// rusty_v8 does not expose this abstract operation. Keep the small conversion
/// boundary here rather than using ToString/ToNumber, which change both the
/// hint and which branch the native Date constructor must take.
fn date_constructor_primitive<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    input: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    if !input.is_object() {
        return Some(input);
    }
    let object = v8::Local::<v8::Object>::try_from(input).ok()?;
    let key = v8::Symbol::get_to_primitive(scope);
    let exotic = object.get(scope, key.into())?;
    if !exotic.is_null_or_undefined() {
        let Ok(method) = v8::Local::<v8::Function>::try_from(exotic) else {
            throw_type_error(scope, "Symbol.toPrimitive is not callable");
            return None;
        };
        let hint = v8str(scope, "default");
        let result = call_script_visible_function(
            scope,
            method,
            input,
            &[hint.into()],
            "Date constructor ToPrimitive",
        )?;
        if !result.is_object() {
            return Some(result);
        }
    } else {
        for key in ["valueOf", "toString"] {
            let method = object.get(scope, v8str(scope, key).into())?;
            if let Ok(method) = v8::Local::<v8::Function>::try_from(method) {
                let result = call_script_visible_function(
                    scope,
                    method,
                    input,
                    &[],
                    "Date constructor OrdinaryToPrimitive",
                )?;
                if !result.is_object() {
                    return Some(result);
                }
            }
        }
    }
    throw_type_error(scope, "Cannot convert object to primitive value");
    None
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
