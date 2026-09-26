use super::*;

pub(super) fn response_array_buffer_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(consumption) = begin_body_consumption_promise(scope, &args, &mut rv) else {
        return;
    };
    finish_body_consumption(
        scope,
        &mut rv,
        consumption,
        NetworkBodyConsumptionKind::ArrayBuffer,
    );
}

pub(super) fn response_blob_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(consumption) = begin_body_consumption_promise(scope, &args, &mut rv) else {
        return;
    };
    finish_body_consumption(
        scope,
        &mut rv,
        consumption,
        NetworkBodyConsumptionKind::Blob,
    );
}

pub(super) fn response_bytes_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(consumption) = begin_body_consumption_promise(scope, &args, &mut rv) else {
        return;
    };
    finish_body_consumption(
        scope,
        &mut rv,
        consumption,
        NetworkBodyConsumptionKind::Bytes,
    );
}
