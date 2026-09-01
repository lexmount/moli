use super::*;
use crate::util::{callback_data_index_value, get_private_value, set_private_value};
use anyhow::{Result, anyhow};
use moli_webapi_declare::WebApiObject;

mod constructor;
mod prototype;

use constructor::{install_date_constructor_proxy, install_date_parse_override};
use prototype::{
    date_get_timezone_offset_callback, date_local_field_callback, date_local_setter_callback,
    date_to_date_string_callback, date_to_locale_date_string_callback,
    date_to_locale_string_callback, date_to_locale_time_string_callback, date_to_string_callback,
    date_to_time_string_callback,
};

const RETAINED_DATE_INTRINSICS_SLOT: &str = "__moliRetainedDateIntrinsics";
#[derive(Clone, Copy)]
#[repr(u32)]
enum DateIntrinsic {
    Parse,
    Utc,
    Now,
    ToString,
    ToDateString,
    ToTimeString,
    ToLocaleString,
    ToLocaleDateString,
    ToLocaleTimeString,
    GetTimezoneOffset,
    GetFullYear,
    GetMonth,
    GetDate,
    GetDay,
    GetHours,
    GetMinutes,
    GetSeconds,
    GetMilliseconds,
    SetDate,
    SetFullYear,
    SetHours,
    SetMilliseconds,
    SetMinutes,
    SetMonth,
    SetSeconds,
    SetTime,
    SetUtcDate,
    SetUtcFullYear,
    SetUtcHours,
    SetUtcMilliseconds,
    SetUtcMinutes,
    SetUtcMonth,
    SetUtcSeconds,
}

impl DateIntrinsic {
    const COUNT: i32 = Self::SetUtcSeconds as i32 + 1;

    const fn index(self) -> u32 {
        self as u32
    }
}

const DATE_STATIC_INTRINSICS: &[(DateIntrinsic, &str)] = &[
    (DateIntrinsic::Parse, "parse"),
    (DateIntrinsic::Utc, "UTC"),
    (DateIntrinsic::Now, "now"),
];

const DATE_PROTOTYPE_INTRINSICS: &[(DateIntrinsic, &str)] = &[
    (DateIntrinsic::ToString, "toString"),
    (DateIntrinsic::ToDateString, "toDateString"),
    (DateIntrinsic::ToTimeString, "toTimeString"),
    (DateIntrinsic::ToLocaleString, "toLocaleString"),
    (DateIntrinsic::ToLocaleDateString, "toLocaleDateString"),
    (DateIntrinsic::ToLocaleTimeString, "toLocaleTimeString"),
    (DateIntrinsic::GetTimezoneOffset, "getTimezoneOffset"),
    (DateIntrinsic::GetFullYear, "getFullYear"),
    (DateIntrinsic::GetMonth, "getMonth"),
    (DateIntrinsic::GetDate, "getDate"),
    (DateIntrinsic::GetDay, "getDay"),
    (DateIntrinsic::GetHours, "getHours"),
    (DateIntrinsic::GetMinutes, "getMinutes"),
    (DateIntrinsic::GetSeconds, "getSeconds"),
    (DateIntrinsic::GetMilliseconds, "getMilliseconds"),
    (DateIntrinsic::SetDate, "setDate"),
    (DateIntrinsic::SetFullYear, "setFullYear"),
    (DateIntrinsic::SetHours, "setHours"),
    (DateIntrinsic::SetMilliseconds, "setMilliseconds"),
    (DateIntrinsic::SetMinutes, "setMinutes"),
    (DateIntrinsic::SetMonth, "setMonth"),
    (DateIntrinsic::SetSeconds, "setSeconds"),
    (DateIntrinsic::SetTime, "setTime"),
    (DateIntrinsic::SetUtcDate, "setUTCDate"),
    (DateIntrinsic::SetUtcFullYear, "setUTCFullYear"),
    (DateIntrinsic::SetUtcHours, "setUTCHours"),
    (DateIntrinsic::SetUtcMilliseconds, "setUTCMilliseconds"),
    (DateIntrinsic::SetUtcMinutes, "setUTCMinutes"),
    (DateIntrinsic::SetUtcMonth, "setUTCMonth"),
    (DateIntrinsic::SetUtcSeconds, "setUTCSeconds"),
];

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

pub(super) fn install_date_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    if get_private_value(scope, global, RETAINED_DATE_INTRINSICS_SLOT).is_some() {
        return Ok(());
    }
    let Some(date_ctor_value) = global.get(scope, v8str(scope, "Date").into()) else {
        return Ok(());
    };
    let Ok(date_ctor) = v8::Local::<v8::Function>::try_from(date_ctor_value) else {
        return Ok(());
    };
    let Some(date_proto_value) = date_ctor.get(scope, v8str(scope, "prototype").into()) else {
        return Ok(());
    };
    let Ok(date_proto) = v8::Local::<v8::Object>::try_from(date_proto_value) else {
        return Ok(());
    };

    let Some(retained) = retain_date_intrinsics(scope, date_ctor, date_proto) else {
        return Ok(());
    };
    set_private_value(
        scope,
        global,
        RETAINED_DATE_INTRINSICS_SLOT,
        retained.into(),
    );
    DateLocalePrototypeDeclaration::default()
        .initialize(scope, date_proto)
        .map_err(|err| anyhow!("failed to initialize Date locale methods: {err}"))?;
    install_date_parse_override(scope, date_ctor)?;
    install_date_constructor_proxy(scope, global, date_ctor, date_proto)
}

fn retain_date_intrinsics<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor: v8::Local<'s, v8::Function>,
    prototype: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    let retained = v8::Array::new(scope, DateIntrinsic::COUNT);
    for &(intrinsic, name) in DATE_STATIC_INTRINSICS {
        let value = constructor.get(scope, v8str(scope, name).into())?;
        let _ = retained.set_index(scope, intrinsic.index(), value);
    }
    for &(intrinsic, name) in DATE_PROTOTYPE_INTRINSICS {
        let value = prototype.get(scope, v8str(scope, name).into())?;
        let _ = retained.set_index(scope, intrinsic.index(), value);
    }
    Some(retained)
}

fn original_date_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    intrinsic: DateIntrinsic,
) -> Option<v8::Local<'s, v8::Function>> {
    let global = scope.get_current_context().global(scope);
    get_private_value(scope, global, RETAINED_DATE_INTRINSICS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .and_then(|retained| retained.get_index(scope, intrinsic.index()))
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
}
