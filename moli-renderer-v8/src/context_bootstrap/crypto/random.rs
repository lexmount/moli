use crate::web_api_interfaces;
use std::ptr;

use moli_crypto::fill_secure_random;
use uuid::Builder as UuidBuilder;

use crate::native_bridge::throw_dom_exception;

use super::helpers::is_crypto_integer_typed_array;
use super::*;

pub(super) fn crypto_get_random_values_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !web_api_interfaces::Crypto::is_instance(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }

    let target = args.get(0);
    let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(target) else {
        throw_type_error(
            scope,
            "crypto.getRandomValues requires an integer TypedArray argument",
        );
        return;
    };
    let Some(backing_store) = view.get_backing_store() else {
        throw_type_error(
            scope,
            "crypto.getRandomValues requires a valid ArrayBufferView argument",
        );
        return;
    };
    if backing_store.is_shared() {
        throw_type_error(
            scope,
            "crypto.getRandomValues does not accept SharedArrayBuffer-backed views",
        );
        return;
    }

    if !is_crypto_integer_typed_array(target) {
        throw_dom_exception(
            scope,
            "TypeMismatchError",
            17,
            "crypto.getRandomValues requires an integer TypedArray argument.",
        );
        return;
    }

    if view.byte_length() > 65_536 {
        // Crypto spec: throw QuotaExceededError DOMException (code 22) when
        // the buffer exceeds 65,536 bytes. WPT asserts the .code property
        // via assert_throws_dom, so a plain Error wouldn't satisfy.
        throw_dom_exception(
            scope,
            "QuotaExceededError",
            22,
            "crypto.getRandomValues input exceeds 65,536 bytes.",
        );
        return;
    }

    let mut bytes = vec![0; view.byte_length()];
    if let Err(error) = fill_secure_random(&mut bytes) {
        throw_error_exception(scope, &format!("getRandomValues failed: {error}"));
        return;
    }

    let data = view.data();
    if !data.is_null() && !bytes.is_empty() {
        // TypedArray views expose writable contiguous backing storage, so
        // crypto.getRandomValues can fill the caller-provided buffer in place.
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), data.cast::<u8>(), bytes.len());
        }
    }

    rv.set(target);
}

pub(super) fn crypto_random_uuid_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !web_api_interfaces::Crypto::is_instance(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }

    let mut bytes = [0_u8; 16];
    if let Err(error) = fill_secure_random(&mut bytes) {
        throw_error_exception(scope, &format!("randomUUID failed: {error}"));
        return;
    }

    let uuid = UuidBuilder::from_random_bytes(bytes)
        .into_uuid()
        .to_string();

    if let Some(value) = v8_string(scope, &uuid) {
        rv.set(value.into());
    }
}
