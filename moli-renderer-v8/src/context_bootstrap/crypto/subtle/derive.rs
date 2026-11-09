use super::*;

pub(crate) fn x25519_derive_material<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm_value: v8::Local<'s, v8::Value>,
    base_key: v8::Local<'s, v8::Object>,
    required_usage: &str,
) -> Result<([u8; 32], [u8; 32]), WebCryptoRejection> {
    let Some(algorithm_object) = algorithm_value.to_object(scope) else {
        return Err(WebCryptoRejection::Type);
    };
    let Some(name) = crypto_algorithm_name(scope, algorithm_value) else {
        return Err(WebCryptoRejection::Type);
    };
    if name.parse::<WebCryptoKeyAlgorithm>() != Ok(WebCryptoKeyAlgorithm::X25519) {
        return Err(WebCryptoRejection::NotSupported);
    }
    let Some(public_key_value) = algorithm_object.get(scope, v8str(scope, "public").into()) else {
        return Err(WebCryptoRejection::Type);
    };
    let Ok(public_key) = subtle_crypto_key_value(scope, public_key_value) else {
        return Err(WebCryptoRejection::Type);
    };
    let Some(base_key_kind) = crypto_key_kind(scope, base_key) else {
        return Err(WebCryptoRejection::Type);
    };
    let Some(public_key_kind) = crypto_key_kind(scope, public_key) else {
        return Err(WebCryptoRejection::Type);
    };
    let Some(base_key_algorithm) = crypto_key_algorithm_name(scope, base_key) else {
        return Err(WebCryptoRejection::Type);
    };
    let Some(public_key_algorithm) = crypto_key_algorithm_name(scope, public_key) else {
        return Err(WebCryptoRejection::Type);
    };
    if base_key_kind != "private"
        || base_key_algorithm != "x25519"
        || !crypto_key_has_usage(scope, base_key, required_usage)
        || public_key_kind != "public"
        || public_key_algorithm != "x25519"
    {
        return Err(WebCryptoRejection::InvalidAccess);
    }
    let Some(private_bytes) = crypto_key_bytes(scope, base_key) else {
        return Err(WebCryptoRejection::Type);
    };
    let Some(public_bytes) = crypto_key_bytes(scope, public_key) else {
        return Err(WebCryptoRejection::Type);
    };
    let Ok(private_bytes) = <[u8; 32]>::try_from(private_bytes.as_slice()) else {
        return Err(WebCryptoRejection::Type);
    };
    let Ok(public_bytes) = <[u8; 32]>::try_from(public_bytes.as_slice()) else {
        return Err(WebCryptoRejection::Type);
    };
    Ok((private_bytes, public_bytes))
}

pub(crate) fn derive_public_key_parameter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm_value: v8::Local<'s, v8::Value>,
    algorithm: WebCryptoKeyAlgorithm,
) -> Result<v8::Local<'s, v8::Object>, WebCryptoRejection> {
    let Some(algorithm_object) = algorithm_value.to_object(scope) else {
        return Err(WebCryptoRejection::Type);
    };
    let Some(name) = crypto_algorithm_name(scope, algorithm_value) else {
        return Err(WebCryptoRejection::Type);
    };
    if name.parse::<WebCryptoKeyAlgorithm>() != Ok(algorithm) {
        return Err(WebCryptoRejection::NotSupported);
    }
    let Some(public_key_value) = algorithm_object.get(scope, v8str(scope, "public").into()) else {
        return Err(WebCryptoRejection::Type);
    };
    subtle_crypto_key_value(scope, public_key_value).map_err(|_| WebCryptoRejection::Type)
}

pub(crate) fn x448_derive_material<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm_value: v8::Local<'s, v8::Value>,
    base_key: v8::Local<'s, v8::Object>,
    required_usage: &str,
) -> Result<(Vec<u8>, Vec<u8>), WebCryptoRejection> {
    let public_key =
        derive_public_key_parameter(scope, algorithm_value, WebCryptoKeyAlgorithm::X448)?;
    if crypto_key_kind(scope, base_key).as_deref() != Some("private")
        || crypto_key_algorithm_name(scope, base_key).as_deref() != Some("x448")
        || !crypto_key_has_usage(scope, base_key, required_usage)
        || crypto_key_kind(scope, public_key).as_deref() != Some("public")
        || crypto_key_algorithm_name(scope, public_key).as_deref() != Some("x448")
    {
        return Err(WebCryptoRejection::InvalidAccess);
    }
    let private_bytes = crypto_key_bytes(scope, base_key).ok_or(WebCryptoRejection::Type)?;
    let public_bytes = crypto_key_bytes(scope, public_key).ok_or(WebCryptoRejection::Type)?;
    if private_bytes.len() != 56 || public_bytes.len() != 56 {
        return Err(WebCryptoRejection::Type);
    }
    Ok((private_bytes, public_bytes))
}

pub(crate) fn ecdh_derive_material<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm_value: v8::Local<'s, v8::Value>,
    base_key: v8::Local<'s, v8::Object>,
    required_usage: &str,
) -> Result<(Vec<u8>, Vec<u8>, WebCryptoEcNamedCurve), WebCryptoRejection> {
    let public_key =
        derive_public_key_parameter(scope, algorithm_value, WebCryptoKeyAlgorithm::Ecdh)?;
    if crypto_key_kind(scope, base_key).as_deref() != Some("private")
        || crypto_key_algorithm_name(scope, base_key).as_deref() != Some("ecdh")
        || !crypto_key_has_usage(scope, base_key, required_usage)
        || crypto_key_kind(scope, public_key).as_deref() != Some("public")
        || crypto_key_algorithm_name(scope, public_key).as_deref() != Some("ecdh")
    {
        return Err(WebCryptoRejection::InvalidAccess);
    }
    let base_curve = crypto_key_ec_named_curve(scope, base_key).ok_or(WebCryptoRejection::Type)?;
    let public_curve =
        crypto_key_ec_named_curve(scope, public_key).ok_or(WebCryptoRejection::Type)?;
    if base_curve != public_curve {
        return Err(WebCryptoRejection::InvalidAccess);
    }
    let private_bytes = crypto_key_bytes(scope, base_key).ok_or(WebCryptoRejection::Type)?;
    let public_bytes = crypto_key_bytes(scope, public_key).ok_or(WebCryptoRejection::Type)?;
    Ok((private_bytes, public_bytes, base_curve))
}

pub(crate) struct KdfDeriveParams {
    pub(crate) hash: WebCryptoHashAlgorithm,
    pub(crate) salt: Vec<u8>,
    pub(crate) info: Vec<u8>,
    pub(crate) iterations: u32,
}

pub(crate) fn kdf_base_key_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    base_key: v8::Local<'s, v8::Object>,
    algorithm: WebCryptoKeyAlgorithm,
    required_usage: &str,
) -> Result<Vec<u8>, WebCryptoRejection> {
    let Some(base_key_kind) = crypto_key_kind(scope, base_key) else {
        return Err(WebCryptoRejection::Type);
    };
    let Some(base_key_algorithm) = crypto_key_algorithm_name(scope, base_key) else {
        return Err(WebCryptoRejection::Type);
    };
    if base_key_kind != "secret"
        || base_key_algorithm != crypto_algorithm_name_for_match(algorithm)
        || !crypto_key_has_usage(scope, base_key, required_usage)
    {
        return Err(WebCryptoRejection::InvalidAccess);
    }
    crypto_key_bytes(scope, base_key).ok_or(WebCryptoRejection::Type)
}

pub(crate) fn crypto_algorithm_name_for_match(algorithm: WebCryptoKeyAlgorithm) -> &'static str {
    match algorithm {
        WebCryptoKeyAlgorithm::AesCbc => "aes-cbc",
        WebCryptoKeyAlgorithm::AesCtr => "aes-ctr",
        WebCryptoKeyAlgorithm::AesGcm => "aes-gcm",
        WebCryptoKeyAlgorithm::AesKw => "aes-kw",
        WebCryptoKeyAlgorithm::Chacha20Poly1305 => "chacha20-poly1305",
        WebCryptoKeyAlgorithm::Hkdf => "hkdf",
        WebCryptoKeyAlgorithm::Hmac => "hmac",
        WebCryptoKeyAlgorithm::Pbkdf2 => "pbkdf2",
        WebCryptoKeyAlgorithm::RsaOaep => "rsa-oaep",
        WebCryptoKeyAlgorithm::RsaPss => "rsa-pss",
        WebCryptoKeyAlgorithm::RsassaPkcs1V15 => "rsassa-pkcs1-v1_5",
        WebCryptoKeyAlgorithm::Ecdh => "ecdh",
        WebCryptoKeyAlgorithm::Ecdsa => "ecdsa",
        WebCryptoKeyAlgorithm::Ed25519 => "ed25519",
        WebCryptoKeyAlgorithm::Ed448 => "ed448",
        WebCryptoKeyAlgorithm::X25519 => "x25519",
        WebCryptoKeyAlgorithm::X448 => "x448",
    }
}

pub(crate) fn kdf_derive_params<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm_object: v8::Local<'s, v8::Object>,
    algorithm: WebCryptoKeyAlgorithm,
) -> Result<KdfDeriveParams, WebCryptoRejection> {
    match algorithm {
        WebCryptoKeyAlgorithm::Hkdf => {
            // Blink's HKDF dictionary parser reads members in WebCrypto order:
            // hash, salt, then info. Keep the observable getter/error order in
            // sync with NormalizeAlgorithm().
            let hash = required_kdf_hash_algorithm(scope, algorithm_object)?;
            let salt = required_buffer_source_member_with_max_rejection(
                scope,
                algorithm_object,
                "salt",
                MAX_KDF_PARAMETER_BYTES,
            )?;
            let info = required_buffer_source_member_with_max_rejection(
                scope,
                algorithm_object,
                "info",
                MAX_KDF_PARAMETER_BYTES,
            )?;
            Ok(KdfDeriveParams {
                hash,
                salt,
                info,
                iterations: 0,
            })
        }
        WebCryptoKeyAlgorithm::Pbkdf2 => {
            // Blink's PBKDF2 parser reads salt and iterations before hash. An
            // invalid hash must not hide a missing salt or bad iteration value.
            let salt = required_buffer_source_member_with_max_rejection(
                scope,
                algorithm_object,
                "salt",
                MAX_KDF_PARAMETER_BYTES,
            )?;
            let iterations = required_pbkdf2_iterations(scope, algorithm_object)?;
            let hash = required_kdf_hash_algorithm(scope, algorithm_object)?;
            Ok(KdfDeriveParams {
                hash,
                salt,
                info: Vec::new(),
                iterations,
            })
        }
        _ => Err(WebCryptoRejection::NotSupported),
    }
}

pub(crate) use super::params::required_webcrypto_hash_parameter as required_kdf_hash_algorithm;

pub(crate) fn required_pbkdf2_iterations(
    scope: &mut v8::PinScope<'_, '_>,
    algorithm_object: v8::Local<'_, v8::Object>,
) -> Result<u32, WebCryptoRejection> {
    let Some(value) = algorithm_object.get(scope, v8str(scope, "iterations").into()) else {
        return Err(WebCryptoRejection::Type);
    };
    if value.is_undefined() {
        return Err(WebCryptoRejection::Type);
    }
    let iterations = webidl::convert::<webidl::EnforceRangeUnsignedLong>(
        scope,
        value,
        webidl::Context::member("Pbkdf2Params", "iterations"),
    )
    .map_err(|_| WebCryptoRejection::Type)?
    .0;
    if iterations == 0 || iterations > MAX_PBKDF2_ITERATIONS {
        return Err(WebCryptoRejection::Operation);
    }
    Ok(iterations)
}

pub(crate) fn required_buffer_source_member_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Result<Vec<u8>, WebCryptoRejection> {
    let Some(value) = object.get(scope, v8str(scope, member).into()) else {
        return Err(WebCryptoRejection::Type);
    };
    if value.is_undefined() {
        return Err(WebCryptoRejection::Type);
    }
    subtle_buffer_source_value_after_preflight(
        scope,
        value,
        webidl::Context::member("SubtleCrypto", member),
        buffer_source_value_can_be_detached_to_empty(value),
    )
    .map_err(|_| WebCryptoRejection::Type)
}

pub(crate) fn required_buffer_source_member_with_max_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    member: &'static str,
    max_bytes: usize,
) -> Result<Vec<u8>, WebCryptoRejection> {
    let Some(value) = object.get(scope, v8str(scope, member).into()) else {
        return Err(WebCryptoRejection::Type);
    };
    if value.is_undefined() {
        return Err(WebCryptoRejection::Type);
    }
    subtle_buffer_source_value_with_max_rejection(
        scope,
        value,
        webidl::Context::member("SubtleCrypto", member),
        max_bytes,
        false,
    )
}

pub(crate) fn optional_buffer_source_member_with_max_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    member: &'static str,
    max_bytes: usize,
) -> Result<Option<Vec<u8>>, WebCryptoRejection> {
    let key = v8str(scope, member);
    if !object.has(scope, key.into()).unwrap_or(false) {
        return Ok(None);
    }
    let Some(value) = object.get(scope, key.into()) else {
        return Err(WebCryptoRejection::Type);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    subtle_buffer_source_value_with_max_rejection(
        scope,
        value,
        webidl::Context::member("SubtleCrypto", member),
        max_bytes,
        false,
    )
    .map(Some)
}

pub(crate) fn required_enforce_range_octet_member_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Result<u32, WebCryptoRejection> {
    let Some(value) = object.get(scope, v8str(scope, member).into()) else {
        return Err(WebCryptoRejection::Type);
    };
    if value.is_undefined() {
        return Err(WebCryptoRejection::Type);
    }
    enforce_range_octet_member_rejection(scope, value, member)
}

pub(crate) fn optional_enforce_range_octet_member_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> Result<Option<u32>, WebCryptoRejection> {
    let key = v8str(scope, member);
    if !object.has(scope, key.into()).unwrap_or(false) {
        return Ok(None);
    }
    let Some(value) = object.get(scope, key.into()) else {
        return Err(WebCryptoRejection::Type);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    enforce_range_octet_member_rejection(scope, value, member).map(Some)
}

pub(crate) fn enforce_range_octet_member_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    member: &'static str,
) -> Result<u32, WebCryptoRejection> {
    let value = webidl::convert::<webidl::EnforceRangeUnsignedLong>(
        scope,
        value,
        webidl::Context::member("SubtleCrypto", member),
    )
    .map_err(|_| WebCryptoRejection::Type)?
    .0;
    if value <= u32::from(u8::MAX) {
        Ok(value)
    } else {
        Err(WebCryptoRejection::Type)
    }
}

pub(crate) fn derive_kdf_bytes(
    algorithm: WebCryptoKeyAlgorithm,
    params: KdfDeriveParams,
    base_key: &[u8],
    length_bits: usize,
) -> Result<Vec<u8>, moli_webcrypto::WebCryptoError> {
    match algorithm {
        WebCryptoKeyAlgorithm::Hkdf => derive_hkdf_bits(
            params.hash,
            base_key,
            &params.salt,
            &params.info,
            length_bits,
        ),
        WebCryptoKeyAlgorithm::Pbkdf2 => derive_pbkdf2_bits(
            params.hash,
            base_key,
            &params.salt,
            params.iterations,
            length_bits,
        ),
        _ => Err(moli_webcrypto::WebCryptoError::Operation),
    }
}

pub(crate) fn optional_derive_bits_length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<usize>, WebCryptoRejection> {
    if args.length() <= 2 || args.get(2).is_null_or_undefined() {
        return Ok(None);
    }
    let length = webidl::convert::<webidl::UnsignedLong>(
        scope,
        args.get(2),
        webidl::Context::argument("SubtleCrypto.deriveBits", 3),
    )
    .map_err(|_| WebCryptoRejection::Type)?;
    Ok(Some(length.0 as usize))
}

pub(crate) fn crypto_subtle_derive_bits_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !subtle_crypto_receiver_is_valid(scope, &args, &mut rv) {
        return;
    }
    // Chromium's generated WebIDL binding converts baseKey to CryptoKey before
    // the implementation normalizes algorithm.name. Preserve that observable
    // ordering so invalid base keys cannot trigger algorithm getter effects.
    let Ok(base_key) = subtle_crypto_key_arg(scope, &args, 1) else {
        set_rejected_webcrypto_promise(scope, &mut rv, WebCryptoRejection::Type);
        return;
    };
    // The nullable length is a WebIDL argument, so Chromium converts it before
    // SubtleCrypto::deriveBits enters NormalizeAlgorithm. This matters when a
    // length valueOf/toString hook and algorithm.name getter both have side
    // effects.
    let length_bits = match optional_derive_bits_length(scope, &args) {
        Ok(length_bits) => length_bits,
        Err(rejection) => {
            set_rejected_webcrypto_promise(scope, &mut rv, rejection);
            return;
        }
    };
    let Some(promise) = PendingCryptoPromise::new(scope, &mut rv) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let Some(name) = crypto_algorithm_name(scope, args.get(0)) else {
        promise.reject_webcrypto(scope, WebCryptoRejection::Type);
        return;
    };
    let Ok(algorithm) = name.parse::<WebCryptoKeyAlgorithm>() else {
        promise.reject_webcrypto(scope, WebCryptoRejection::NotSupported);
        return;
    };

    match algorithm {
        WebCryptoKeyAlgorithm::X25519 => {
            let length_bits = length_bits.unwrap_or(256);
            let (private_bytes, public_bytes) =
                match x25519_derive_material(scope, args.get(0), base_key, "deriveBits") {
                    Ok(material) => material,
                    Err(rejection) => {
                        promise.reject_webcrypto(scope, rejection);
                        return;
                    }
                };
            if length_bits > 256 {
                promise.reject_webcrypto(scope, WebCryptoRejection::Operation);
                return;
            }
            if let Some((handle, completion_tx)) = register_webcrypto_task(scope, &promise) {
                spawn_webcrypto_bytes_task(handle, completion_tx, move || {
                    derive_x25519_bits(&private_bytes, public_bytes, length_bits)
                });
                return;
            }
            let bytes = match derive_x25519_bits(&private_bytes, public_bytes, length_bits) {
                Ok(bytes) => bytes,
                Err(error) => {
                    promise.reject_webcrypto(scope, WebCryptoRejection::from(error));
                    return;
                }
            };
            resolve_derived_bits(scope, &promise, bytes);
        }
        WebCryptoKeyAlgorithm::X448 => {
            let length_bits = length_bits.unwrap_or(448);
            let (private_bytes, public_bytes) =
                match x448_derive_material(scope, args.get(0), base_key, "deriveBits") {
                    Ok(material) => material,
                    Err(rejection) => {
                        promise.reject_webcrypto(scope, rejection);
                        return;
                    }
                };
            if length_bits > 448 {
                promise.reject_webcrypto(scope, WebCryptoRejection::Operation);
                return;
            }
            if let Some((handle, completion_tx)) = register_webcrypto_task(scope, &promise) {
                spawn_webcrypto_bytes_task(handle, completion_tx, move || {
                    derive_x448_bits(&private_bytes, &public_bytes, length_bits)
                });
                return;
            }
            let bytes = match derive_x448_bits(&private_bytes, &public_bytes, length_bits) {
                Ok(bytes) => bytes,
                Err(error) => {
                    promise.reject_webcrypto(scope, WebCryptoRejection::from(error));
                    return;
                }
            };
            resolve_derived_bits(scope, &promise, bytes);
        }
        WebCryptoKeyAlgorithm::Ecdh => {
            let (private_bytes, public_bytes, curve) =
                match ecdh_derive_material(scope, args.get(0), base_key, "deriveBits") {
                    Ok(material) => material,
                    Err(rejection) => {
                        promise.reject_webcrypto(scope, rejection);
                        return;
                    }
                };
            let max_bits = curve.coordinate_len_bytes() * 8;
            let length_bits = length_bits.unwrap_or(max_bits);
            if length_bits > max_bits {
                promise.reject_webcrypto(scope, WebCryptoRejection::Operation);
                return;
            }
            if let Some((handle, completion_tx)) = register_webcrypto_task(scope, &promise) {
                spawn_webcrypto_bytes_task(handle, completion_tx, move || {
                    derive_ecdh_bits(&private_bytes, &public_bytes, curve, length_bits)
                });
                return;
            }
            let bytes = match derive_ecdh_bits(&private_bytes, &public_bytes, curve, length_bits) {
                Ok(bytes) => bytes,
                Err(error) => {
                    promise.reject_webcrypto(scope, WebCryptoRejection::from(error));
                    return;
                }
            };
            resolve_derived_bits(scope, &promise, bytes);
        }
        WebCryptoKeyAlgorithm::Hkdf | WebCryptoKeyAlgorithm::Pbkdf2 => {
            let Some(algorithm_object) = args.get(0).to_object(scope) else {
                promise.reject_webcrypto(scope, WebCryptoRejection::Type);
                return;
            };
            let Some(length_bits) = length_bits else {
                promise.reject_webcrypto(scope, WebCryptoRejection::Operation);
                return;
            };
            if !length_bits.is_multiple_of(8) {
                promise.reject_webcrypto(scope, WebCryptoRejection::Operation);
                return;
            }
            let params = match kdf_derive_params(scope, algorithm_object, algorithm) {
                Ok(params) => params,
                Err(rejection) => {
                    promise.reject_webcrypto(scope, rejection);
                    return;
                }
            };
            let base_key = match kdf_base_key_bytes(scope, base_key, algorithm, "deriveBits") {
                Ok(bytes) => bytes,
                Err(rejection) => {
                    promise.reject_webcrypto(scope, rejection);
                    return;
                }
            };
            if length_bits > MAX_KDF_DERIVED_BITS {
                promise.reject_webcrypto(scope, WebCryptoRejection::Operation);
                return;
            }
            if let Some((handle, completion_tx)) = register_webcrypto_task(scope, &promise) {
                spawn_webcrypto_bytes_task(handle, completion_tx, move || {
                    derive_kdf_bytes(algorithm, params, &base_key, length_bits)
                });
                return;
            }
            let bytes = match derive_kdf_bytes(algorithm, params, &base_key, length_bits) {
                Ok(bytes) => bytes,
                Err(error) => {
                    promise.reject_webcrypto(scope, WebCryptoRejection::from(error));
                    return;
                }
            };
            resolve_derived_bits(scope, &promise, bytes);
        }
        _ => promise.reject_webcrypto(scope, WebCryptoRejection::NotSupported),
    }
}

pub(crate) fn resolve_derived_bits<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    promise: &PendingCryptoPromise<'s>,
    bytes: Vec<u8>,
) {
    match blob::array_buffer_from_bytes(scope, bytes) {
        Some(buffer) => promise.resolve(scope, buffer.into()),
        None => promise.reject_webcrypto(scope, WebCryptoRejection::Type),
    }
}
