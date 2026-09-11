use super::*;
use crate::util::{
    callback_data_index_value, get_private_value, global_constructor_object,
    serialize_v8_iter_array, set_private_value,
};
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct NamedAlgorithmDeclaration<'scope> {
    name: Option<v8::Local<'scope, v8::String>>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct AlgorithmNameCloneDeclaration<'scope> {
    name: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct AlgorithmCloneDeclaration<'scope> {
    name: v8::Local<'scope, v8::Value>,
    hash: Option<v8::Local<'scope, v8::Object>>,
    length: Option<v8::Local<'scope, v8::Value>>,
    named_curve: Option<v8::Local<'scope, v8::Value>>,
    modulus_length: Option<v8::Local<'scope, v8::Value>>,
    public_exponent: Option<v8::Local<'scope, v8::Uint8Array>>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct HmacAlgorithmDeclaration<'scope> {
    name: Option<v8::Local<'scope, v8::String>>,
    hash: v8::Local<'scope, v8::Object>,
    length: usize,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct SymmetricAlgorithmDeclaration<'scope> {
    name: Option<v8::Local<'scope, v8::String>>,
    length: usize,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct RsaAlgorithmDeclaration<'scope> {
    name: Option<v8::Local<'scope, v8::String>>,
    hash: v8::Local<'scope, v8::Object>,
    modulus_length: usize,
    public_exponent: Option<v8::Local<'scope, v8::Uint8Array>>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct NamedCurveAlgorithmDeclaration<'scope> {
    name: Option<v8::Local<'scope, v8::String>>,
    named_curve: Option<v8::Local<'scope, v8::String>>,
}

pub(super) fn build_named_algorithm_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> v8::Local<'s, v8::Object> {
    NamedAlgorithmDeclaration::new(v8_string(scope, name))
        .bind(scope)
        .expect("named algorithm declaration should bind")
}

pub(super) fn build_hmac_algorithm_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    hash_name: &str,
    length_bits: usize,
) -> v8::Local<'s, v8::Object> {
    let hash_name = hash_name.to_ascii_uppercase();
    let hash = build_named_algorithm_object(scope, &hash_name);
    HmacAlgorithmDeclaration::new(v8_string(scope, "HMAC"), hash, length_bits)
        .bind(scope)
        .expect("HMAC algorithm declaration should bind")
}

pub(super) fn build_symmetric_algorithm_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
    length_bits: usize,
) -> v8::Local<'s, v8::Object> {
    SymmetricAlgorithmDeclaration::new(v8_string(scope, name), length_bits)
        .bind(scope)
        .expect("symmetric algorithm declaration should bind")
}

pub(super) fn build_rsa_algorithm_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
    hash_name: &str,
    modulus_length_bits: usize,
    public_exponent: &[u8],
) -> v8::Local<'s, v8::Object> {
    let hash_name = hash_name.to_ascii_uppercase();
    let hash = build_named_algorithm_object(scope, &hash_name);
    let public_exponent = blob::array_buffer_from_bytes(scope, public_exponent.to_vec())
        .and_then(|buffer| v8::Uint8Array::new(scope, buffer, 0, buffer.byte_length()));
    RsaAlgorithmDeclaration::new(
        v8_string(scope, name),
        hash,
        modulus_length_bits,
        public_exponent,
    )
    .bind(scope)
    .expect("RSA algorithm declaration should bind")
}

pub(super) fn build_named_curve_algorithm_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
    named_curve: &str,
) -> v8::Local<'s, v8::Object> {
    NamedCurveAlgorithmDeclaration::new(v8_string(scope, name), v8_string(scope, named_curve))
        .bind(scope)
        .expect("named curve algorithm declaration should bind")
}

const CRYPTO_KEY_ATTRIBUTE_SLOTS: &[&str] = &[
    CRYPTO_KEY_KIND_SLOT,
    CRYPTO_KEY_EXTRACTABLE_SLOT,
    CRYPTO_KEY_ALGORITHM_SLOT,
    CRYPTO_KEY_USAGES_SLOT,
];
const CRYPTO_KEY_USAGE_ORDER: &[&str] = &[
    "encrypt",
    "decrypt",
    "sign",
    "verify",
    "deriveKey",
    "deriveBits",
    "wrapKey",
    "unwrapKey",
];

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CryptoKey, require_prototype, scope_lifetime = 'scope)]
struct CryptoKeyObjectDeclaration<'scope, 'value> {
    #[webapi(slot = CRYPTO_KEY_KIND_SLOT)]
    key_type: &'value str,

    #[webapi(slot = CRYPTO_KEY_ALGORITHM_SLOT)]
    algorithm: v8::Local<'scope, v8::Object>,

    #[webapi(slot = CRYPTO_KEY_EXTRACTABLE_SLOT)]
    extractable: bool,

    #[webapi(slot = CRYPTO_KEY_USAGES_SLOT)]
    usages: v8::Local<'scope, v8::Array>,

    #[webapi(slot = CRYPTO_KEY_BYTES_SLOT)]
    bytes: v8::Local<'scope, v8::ArrayBuffer>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CryptoKey)]
struct CryptoKeyPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        name = "type",
        getter = crypto_key_attribute_getter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    key_type: (),
    #[webapi(
        accessor_property,
        getter = crypto_key_attribute_getter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    extractable: (),
    #[webapi(
        accessor_property,
        getter = crypto_key_attribute_getter_callback,
        data = callback_data_index_value(scope, 2),
        enumerable
    )]
    algorithm: (),
    #[webapi(
        accessor_property,
        getter = crypto_key_attribute_getter_callback,
        data = callback_data_index_value(scope, 3),
        enumerable
    )]
    usages: (),
}

#[derive(Clone, Debug)]
pub(crate) struct CryptoKeyClonePayload {
    pub(crate) key_type: String,
    pub(crate) algorithm: CryptoKeyAlgorithmClonePayload,
    pub(crate) extractable: bool,
    pub(crate) usages: Vec<String>,
    pub(crate) key_bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct CryptoKeyAlgorithmClonePayload {
    pub(crate) name: String,
    pub(crate) hash_name: Option<String>,
    pub(crate) length_bits: Option<usize>,
    pub(crate) named_curve: Option<String>,
    pub(crate) modulus_length_bits: Option<usize>,
    pub(crate) public_exponent: Option<Vec<u8>>,
}

pub(super) fn install_crypto_key_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    CryptoKeyPrototypeAccessorsDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

fn crypto_key_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(slot) =
        callback_data_item(scope, &args, CRYPTO_KEY_ATTRIBUTE_SLOTS, "CryptoKey slots")
    else {
        rv.set_undefined();
        return;
    };
    let receiver = args.this();
    let value = match slot {
        CRYPTO_KEY_ALGORITHM_SLOT => {
            crypto_key_visible_algorithm_object(scope, receiver).map(Into::into)
        }
        CRYPTO_KEY_USAGES_SLOT => crypto_key_visible_usages_array(scope, receiver).map(Into::into),
        _ => get_private_value(scope, receiver, slot),
    };
    match value {
        Some(value) => rv.set(value),
        None => throw_type_error(scope, "Illegal invocation"),
    }
}

pub(super) fn new_crypto_key_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key_type: &str,
    algorithm: v8::Local<'s, v8::Object>,
    extractable: bool,
    usages: &[String],
    key_bytes: &[u8],
) -> Option<v8::Local<'s, v8::Object>> {
    let normalized_usages = normalized_key_usages(usages);
    let usages_array = crate::util::serialize_v8_array(scope, normalized_usages.as_slice())?;
    let bytes = blob::array_buffer_from_bytes(scope, key_bytes.to_vec())?;
    CryptoKeyObjectDeclaration::new(key_type, algorithm, extractable, usages_array, bytes)
        .bind(scope)
        .ok()
}

fn normalized_key_usages(usages: &[String]) -> Vec<String> {
    CRYPTO_KEY_USAGE_ORDER
        .iter()
        .copied()
        .filter(|expected| usages.iter().any(|usage| usage == expected))
        .map(str::to_owned)
        .collect()
}

pub(crate) fn is_crypto_key_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
) -> bool {
    moli_webapi_declare::implements_interface(scope, key, "CryptoKey")
}

pub(crate) fn crypto_key_clone_payload_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
) -> Option<CryptoKeyClonePayload> {
    Some(CryptoKeyClonePayload {
        key_type: crypto_key_kind(scope, key)?,
        algorithm: crypto_key_algorithm_clone_payload(scope, key)?,
        extractable: crypto_key_extractable(scope, key)?,
        usages: crypto_key_usages(scope, key)?,
        key_bytes: crypto_key_bytes(scope, key)?,
    })
}

pub(crate) fn crypto_key_object_from_clone_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: CryptoKeyClonePayload,
) -> Option<v8::Local<'s, v8::Object>> {
    global_constructor_object(scope, "CryptoKey")?;
    if !clone_payload_algorithm_is_supported(&payload.key_type, &payload.algorithm) {
        return None;
    }
    let algorithm = match payload.algorithm.name.as_str() {
        "HMAC" => build_hmac_algorithm_object(
            scope,
            payload.algorithm.hash_name.as_deref()?,
            payload.algorithm.length_bits?,
        ),
        "AES-CBC" | "AES-CTR" | "AES-GCM" | "AES-KW" => build_symmetric_algorithm_object(
            scope,
            &payload.algorithm.name,
            payload.algorithm.length_bits?,
        ),
        "ChaCha20-Poly1305" => build_named_algorithm_object(scope, &payload.algorithm.name),
        "RSA-OAEP" | "RSA-PSS" | "RSASSA-PKCS1-v1_5" => build_rsa_algorithm_object(
            scope,
            &payload.algorithm.name,
            payload.algorithm.hash_name.as_deref()?,
            payload.algorithm.modulus_length_bits?,
            payload.algorithm.public_exponent.as_deref()?,
        ),
        "ECDSA" | "ECDH" => build_named_curve_algorithm_object(
            scope,
            &payload.algorithm.name,
            payload.algorithm.named_curve.as_deref()?,
        ),
        _ => build_named_algorithm_object(scope, &payload.algorithm.name),
    };
    new_crypto_key_object(
        scope,
        &payload.key_type,
        algorithm,
        payload.extractable,
        &payload.usages,
        &payload.key_bytes,
    )
}

fn clone_payload_algorithm_is_supported(
    key_type: &str,
    algorithm: &CryptoKeyAlgorithmClonePayload,
) -> bool {
    match (key_type, algorithm.name.as_str()) {
        ("secret", "HMAC") => algorithm.hash_name.is_some() && algorithm.length_bits.is_some(),
        ("secret", "AES-CBC" | "AES-CTR" | "AES-GCM" | "AES-KW") => algorithm.length_bits.is_some(),
        ("secret", "ChaCha20-Poly1305") => true,
        ("secret", "HKDF" | "PBKDF2") => true,
        ("private" | "public", "RSA-OAEP" | "RSA-PSS" | "RSASSA-PKCS1-v1_5") => {
            algorithm.hash_name.is_some()
                && algorithm.modulus_length_bits.is_some()
                && algorithm.public_exponent.is_some()
        }
        ("private" | "public", "ECDSA" | "ECDH") => algorithm.named_curve.is_some(),
        ("private" | "public", "Ed25519" | "Ed448" | "X25519" | "X448") => true,
        _ => false,
    }
}

fn crypto_key_algorithm_clone_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
) -> Option<CryptoKeyAlgorithmClonePayload> {
    let algorithm = crypto_key_algorithm_object(scope, key)?;
    let name = data_property_string(scope, algorithm, "name")?;
    let hash_name = algorithm
        .get(scope, v8str(scope, "hash").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|hash| data_property_string(scope, hash, "name"));
    let length_bits = data_property_usize(scope, algorithm, "length");
    let named_curve = data_property_string(scope, algorithm, "namedCurve");
    let modulus_length_bits = data_property_usize(scope, algorithm, "modulusLength");
    let public_exponent = algorithm
        .get(scope, v8str(scope, "publicExponent").into())
        .and_then(|value| buffer_source_value_bytes(scope, value));
    Some(CryptoKeyAlgorithmClonePayload {
        name,
        hash_name,
        length_bits,
        named_curve,
        modulus_length_bits,
        public_exponent,
    })
}

fn data_property_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Option<String> {
    object
        .get(scope, v8str(scope, name).into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

fn data_property_usize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Option<usize> {
    let value = object.get(scope, v8str(scope, name).into())?;
    if value.is_undefined() {
        return None;
    }
    value.number_value(scope).map(|value| value as usize)
}

pub(super) fn crypto_key_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
) -> Option<Vec<u8>> {
    get_private_value(scope, key, CRYPTO_KEY_BYTES_SLOT).and_then(|value| {
        if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
            let view = v8::Uint8Array::new(scope, buffer, 0, buffer.byte_length())?;
            let mut bytes = vec![0; view.byte_length()];
            let written = view.copy_contents(&mut bytes);
            bytes.truncate(written);
            return Some(bytes);
        }
        if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
            let mut bytes = vec![0; view.byte_length()];
            let written = view.copy_contents(&mut bytes);
            bytes.truncate(written);
            return Some(bytes);
        }
        None
    })
}

pub(super) fn crypto_key_algorithm_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, key, CRYPTO_KEY_ALGORITHM_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(super) fn crypto_key_kind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
) -> Option<String> {
    get_private_value(scope, key, CRYPTO_KEY_KIND_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(super) fn crypto_key_extractable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
) -> Option<bool> {
    get_private_value(scope, key, CRYPTO_KEY_EXTRACTABLE_SLOT)
        .map(|value| value.boolean_value(scope))
}

pub(super) fn crypto_key_usages<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
) -> Option<Vec<String>> {
    let array = crypto_key_usages_array(scope, key)?;
    let mut usages = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        let value = array.get_index(scope, index)?;
        let usage = value.to_string(scope)?.to_rust_string_lossy(scope);
        usages.push(usage);
    }
    Some(usages)
}

pub(super) fn crypto_key_has_usage<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
    expected: &str,
) -> bool {
    crypto_key_usages(scope, key).is_some_and(|usages| usages.iter().any(|usage| usage == expected))
}

fn crypto_key_usages_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_value(scope, key, CRYPTO_KEY_USAGES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

fn crypto_key_visible_algorithm_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    // WebCrypto marks CryptoKey.algorithm as SameObject-like observable state:
    // repeated getter calls return the same ECMAScript object for a given key.
    // Keep that visible wrapper separate from the immutable internal algorithm
    // slot so page mutations to key.algorithm never affect crypto operations.
    if let Some(cached) = get_private_value(scope, key, CRYPTO_KEY_VISIBLE_ALGORITHM_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Some(cached);
    }
    let algorithm = crypto_key_algorithm_object(scope, key)?;
    let visible = cloned_algorithm_object(scope, algorithm)?;
    set_private_value(
        scope,
        key,
        CRYPTO_KEY_VISIBLE_ALGORITHM_SLOT,
        visible.into(),
    );
    Some(visible)
}

fn crypto_key_visible_usages_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    if let Some(cached) = get_private_value(scope, key, CRYPTO_KEY_VISIBLE_USAGES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    {
        return Some(cached);
    }
    let usages = crypto_key_usages_array(scope, key)?;
    let visible = cloned_usages_array(scope, usages)?;
    set_private_value(scope, key, CRYPTO_KEY_VISIBLE_USAGES_SLOT, visible.into());
    Some(visible)
}

pub(super) fn cloned_algorithm_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let name = algorithm.get(scope, v8str(scope, "name").into())?;
    if name.is_undefined() {
        return None;
    }
    let hash = cloned_algorithm_hash_object(scope, algorithm)?;
    AlgorithmCloneDeclaration::new(
        name,
        hash,
        optional_data_property_value(scope, algorithm, "length"),
        optional_data_property_value(scope, algorithm, "namedCurve"),
        optional_data_property_value(scope, algorithm, "modulusLength"),
        cloned_public_exponent_view(scope, algorithm),
    )
    .bind(scope)
    .ok()
}

fn cloned_algorithm_hash_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm: v8::Local<'s, v8::Object>,
) -> Option<Option<v8::Local<'s, v8::Object>>> {
    let Some(hash) = algorithm
        .get(scope, v8str(scope, "hash").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return Some(None);
    };
    let hash_name = hash.get(scope, v8str(scope, "name").into())?;
    if hash_name.is_undefined() {
        return None;
    }
    AlgorithmNameCloneDeclaration::new(hash_name)
        .bind(scope)
        .ok()
        .map(Some)
}

fn optional_data_property_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    let value = object.get(scope, v8str(scope, name).into())?;
    (!value.is_undefined()).then_some(value)
}

fn cloned_public_exponent_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Uint8Array>> {
    algorithm
        .get(scope, v8str(scope, "publicExponent").into())
        .and_then(|value| buffer_source_value_bytes(scope, value))
        .and_then(|bytes| blob::array_buffer_from_bytes(scope, bytes))
        .and_then(|buffer| v8::Uint8Array::new(scope, buffer, 0, buffer.byte_length()))
}

fn buffer_source_value_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<Vec<u8>> {
    if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
        let view = v8::Uint8Array::new(scope, buffer, 0, buffer.byte_length())?;
        let mut bytes = vec![0; view.byte_length()];
        let written = view.copy_contents(&mut bytes);
        bytes.truncate(written);
        return Some(bytes);
    }
    if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
        let mut bytes = vec![0; view.byte_length()];
        let written = view.copy_contents(&mut bytes);
        bytes.truncate(written);
        return Some(bytes);
    }
    None
}

fn cloned_usages_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    usages: v8::Local<'s, v8::Array>,
) -> Option<v8::Local<'s, v8::Array>> {
    let values = (0..usages.length())
        .map(|index| usages.get_index(scope, index))
        .collect::<Option<Vec<_>>>()?;
    serialize_v8_iter_array(scope, values)
}
