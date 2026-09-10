use super::keys::install_crypto_key_template_bindings;
use super::random::{crypto_get_random_values_callback, crypto_random_uuid_callback};
use super::subtle::{
    crypto_subtle_decrypt_callback, crypto_subtle_derive_bits_callback,
    crypto_subtle_derive_key_callback, crypto_subtle_digest_callback,
    crypto_subtle_encrypt_callback, crypto_subtle_export_key_callback,
    crypto_subtle_generate_key_callback, crypto_subtle_get_public_key_callback,
    crypto_subtle_import_key_callback, crypto_subtle_sign_callback,
    crypto_subtle_supports_callback, crypto_subtle_unwrap_key_callback,
    crypto_subtle_verify_callback, crypto_subtle_wrap_key_callback,
};
use super::*;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, set_private_value,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const CRYPTO_SUBTLE_SLOT: &str = "__moliCryptoSubtle";
const CRYPTO_SUBTLE_AVAILABLE_SLOT: &str = "__moliCryptoSubtleAvailable";
const WINDOW_CRYPTO_SUBTLE_AVAILABLE_SLOT: &str = "__moliWindowCryptoSubtleAvailable";

#[derive(Default, WebApiObject)]
#[webapi(allow_empty, interface = "Crypto")]
struct CryptoObjectDeclaration {}

#[derive(Default, WebApiObject)]
#[webapi(allow_empty, interface = "SubtleCrypto")]
struct SubtleCryptoObjectDeclaration {}

#[derive(Default, WebApiObject)]
#[webapi(unbranded, interface = "Crypto")]
struct CryptoPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = crypto_attribute_getter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    subtle: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(name = "Crypto")]
struct CryptoPrototypeOperationsDeclaration {
    #[webapi(
        method = "getRandomValues",
        enumerable,
        length = 1,
        callback = crypto_get_random_values_callback
    )]
    _get_random_values: (),
}

#[derive(Default, WebApiObject)]
#[webapi(unbranded, interface = "Crypto")]
struct CryptoSecurePrototypeOperationsDeclaration {
    #[webapi(
        method = "randomUUID",
        enumerable,
        length = 0,
        callback = crypto_random_uuid_callback
    )]
    _random_uuid: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(name = "SubtleCrypto")]
struct SubtleCryptoPrototypeOperationsDeclaration {
    #[webapi(method, enumerable, length = 2, callback = crypto_subtle_digest_callback)]
    digest: (),
    #[webapi(method = "generateKey", enumerable, length = 3, callback = crypto_subtle_generate_key_callback)]
    _generate_key: (),
    #[webapi(method, enumerable, length = 3, callback = crypto_subtle_encrypt_callback)]
    encrypt: (),
    #[webapi(method, enumerable, length = 3, callback = crypto_subtle_decrypt_callback)]
    decrypt: (),
    #[webapi(method, enumerable, length = 3, callback = crypto_subtle_sign_callback)]
    sign: (),
    #[webapi(method, enumerable, length = 4, callback = crypto_subtle_verify_callback)]
    verify: (),
    #[webapi(method = "deriveBits", enumerable, length = 2, callback = crypto_subtle_derive_bits_callback)]
    _derive_bits: (),
    #[webapi(method = "deriveKey", enumerable, length = 5, callback = crypto_subtle_derive_key_callback)]
    _derive_key: (),
    #[webapi(method = "getPublicKey", enumerable, length = 2, callback = crypto_subtle_get_public_key_callback)]
    _get_public_key: (),
    #[webapi(method = "importKey", enumerable, length = 5, callback = crypto_subtle_import_key_callback)]
    _import_key: (),
    #[webapi(method = "exportKey", enumerable, length = 2, callback = crypto_subtle_export_key_callback)]
    _export_key: (),
    #[webapi(method = "wrapKey", enumerable, length = 4, callback = crypto_subtle_wrap_key_callback)]
    _wrap_key: (),
    #[webapi(method = "unwrapKey", enumerable, length = 7, callback = crypto_subtle_unwrap_key_callback)]
    _unwrap_key: (),
}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(name = "SubtleCrypto")]
struct SubtleCryptoStaticOperationsDeclaration {
    #[webapi(
        static_method,
        enumerable,
        length = 2,
        callback = crypto_subtle_supports_callback
    )]
    supports: (),
}

pub(in crate::context_bootstrap) fn install_crypto_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "Crypto" => {
            CryptoPrototypeOperationsDeclaration::initialize_prototype_template(scope, prototype);
        }
        "SubtleCrypto" => {
            SubtleCryptoPrototypeOperationsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
            SubtleCryptoStaticOperationsDeclaration::initialize_template(scope, template);
        }
        "CryptoKey" => install_crypto_key_template_bindings(scope, template),
        _ => {}
    }
}

pub(in crate::context_bootstrap) fn install_window_crypto_runtime_state(
    scope: &mut v8::PinScope<'_, '_>,
    global: v8::Local<'_, v8::Object>,
    subtle_crypto_available: bool,
) -> Result<()> {
    let available = v8::Boolean::new(scope, subtle_crypto_available);
    set_private_value(
        scope,
        global,
        WINDOW_CRYPTO_SUBTLE_AVAILABLE_SLOT,
        available.into(),
    );
    Ok(())
}

pub(in crate::context_bootstrap) fn install_worker_crypto_runtime_state(
    scope: &mut v8::PinScope<'_, '_>,
    global: v8::Local<'_, v8::Object>,
    subtle_crypto_available: bool,
) -> Result<()> {
    install_window_crypto_runtime_state(scope, global, subtle_crypto_available)?;
    if !subtle_crypto_available {
        remove_subtle_crypto_globals(scope, global);
    }
    Ok(())
}

pub(in crate::context_bootstrap) fn ensure_worker_crypto_for_global<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Object>> {
    if let Some(crypto) = get_private_value(scope, global, WINDOW_CRYPTO_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Ok(crypto);
    }
    let crypto = build_window_crypto_for_receiver(scope, global)?;
    set_private_value(scope, global, WINDOW_CRYPTO_SLOT, crypto.into());
    Ok(crypto)
}

pub(in crate::context_bootstrap) fn build_window_crypto_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Object>> {
    let subtle_crypto_available =
        get_private_value(scope, window, WINDOW_CRYPTO_SUBTLE_AVAILABLE_SLOT)
            .is_some_and(|value| value.boolean_value(scope));
    build_crypto_object(scope, subtle_crypto_available)
}

fn build_crypto_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    subtle_crypto_available: bool,
) -> Result<v8::Local<'s, v8::Object>> {
    let crypto = CryptoObjectDeclaration::default()
        .bind(scope)
        .map_err(|error| anyhow!(error))?;
    let available = v8::Boolean::new(scope, subtle_crypto_available);
    set_private_value(
        scope,
        crypto,
        CRYPTO_SUBTLE_AVAILABLE_SLOT,
        available.into(),
    );
    Ok(crypto)
}

pub(in crate::context_bootstrap) fn finalize_crypto_realm_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let global = scope.get_current_context().global(scope);
    let subtle_crypto_available =
        get_private_value(scope, global, WINDOW_CRYPTO_SUBTLE_AVAILABLE_SLOT)
            .is_some_and(|value| value.boolean_value(scope));
    if !subtle_crypto_available {
        return Ok(());
    }
    CryptoPrototypeAccessorsDeclaration::default()
        .initialize(scope, prototype)
        .map_err(|error| anyhow!(error))?;
    CryptoSecurePrototypeOperationsDeclaration::default()
        .initialize(scope, prototype)
        .map_err(|error| anyhow!(error))
}

fn remove_subtle_crypto_globals(
    scope: &mut v8::PinScope<'_, '_>,
    global: v8::Local<'_, v8::Object>,
) {
    for name in ["SubtleCrypto", "CryptoKey"] {
        let _ = global.delete(scope, v8str(scope, name).into());
    }
}

fn crypto_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(scope, &args, CRYPTO_ATTRIBUTE_SLOTS, "Crypto slots")
    else {
        rv.set_undefined();
        return;
    };
    if !moli_webapi_declare::implements_interface(scope, args.this(), "Crypto") {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    if slot == CRYPTO_SUBTLE_SLOT {
        match ensure_crypto_subtle(scope, args.this()) {
            Ok(Some(subtle)) => rv.set(subtle.into()),
            Ok(None) => rv.set_undefined(),
            Err(error) => throw_error(
                scope,
                &format!("Failed to materialize Crypto.subtle: {error}"),
            ),
        }
        return;
    }
    rv.set(
        get_private_value(scope, args.this(), slot).unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

const CRYPTO_ATTRIBUTE_SLOTS: &[&str] = &[CRYPTO_SUBTLE_SLOT];

fn ensure_crypto_subtle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    crypto: v8::Local<'s, v8::Object>,
) -> Result<Option<v8::Local<'s, v8::Object>>> {
    if let Some(subtle) = get_private_value(scope, crypto, CRYPTO_SUBTLE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Ok(Some(subtle));
    }
    if get_private_value(scope, crypto, CRYPTO_SUBTLE_AVAILABLE_SLOT)
        .is_none_or(|value| !value.boolean_value(scope))
    {
        return Ok(None);
    }
    let relevant_context = crypto
        .get_creation_context(scope)
        .ok_or_else(|| anyhow!("Crypto receiver has no creation context"))?;
    if relevant_context == scope.get_current_context() {
        return build_crypto_subtle_in_current_realm(scope, crypto).map(Some);
    }
    let crypto = v8::Global::new(scope, crypto);
    let subtle = {
        let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
        let target_crypto = v8::Local::new(target_scope, &crypto);
        let subtle = build_crypto_subtle_in_current_realm(target_scope, target_crypto)?;
        v8::Global::new(target_scope, subtle)
    };
    Ok(Some(v8::Local::new(scope, &subtle)))
}

fn build_crypto_subtle_in_current_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    crypto: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Object>> {
    let subtle = SubtleCryptoObjectDeclaration::default()
        .bind(scope)
        .map_err(|error| anyhow!(error))?;
    set_private_value(scope, crypto, CRYPTO_SUBTLE_SLOT, subtle.into());
    Ok(subtle)
}
