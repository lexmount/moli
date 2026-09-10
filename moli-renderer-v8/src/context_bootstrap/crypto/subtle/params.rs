use super::*;

pub(crate) fn subtle_crypto_receiver_is_valid<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) -> bool {
    if moli_webapi_declare::implements_interface(scope, args.this(), "SubtleCrypto") {
        true
    } else {
        set_rejected_webcrypto_promise(scope, rv, WebCryptoRejection::Type);
        false
    }
}

pub(crate) fn subtle_crypto_key_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<v8::Local<'s, v8::Object>, ()> {
    subtle_crypto_key_value(scope, args.get(index))
}

pub(crate) fn subtle_crypto_key_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Object>, ()> {
    let key = v8::Local::<v8::Object>::try_from(value).map_err(|_| ())?;
    if is_crypto_key_object(scope, key) {
        Ok(key)
    } else {
        Err(())
    }
}

pub(crate) fn crypto_key_can_run_symmetric_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
    algorithm: WebCryptoKeyAlgorithm,
    usage: &str,
) -> bool {
    crypto_key_kind(scope, key).as_deref() == Some("secret")
        && crypto_key_algorithm_name(scope, key).as_deref()
            == Some(crypto_algorithm_name_for_match(algorithm))
        && crypto_key_has_usage(scope, key, usage)
}

pub(crate) fn snapshot_key_material_for_wrap_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
    format: &str,
) -> Result<ExportKeySnapshot, WebCryptoRejection> {
    let format = SubtleKeyFormat::parse(format).ok_or(WebCryptoRejection::Type)?;
    if !crypto_key_extractable(scope, key).unwrap_or(false) {
        return Err(WebCryptoRejection::InvalidAccess);
    }
    let key_type = crypto_key_kind(scope, key).ok_or(WebCryptoRejection::Type)?;
    let Some(algorithm) = crypto_key_algorithm_name(scope, key)
        .and_then(|name| name.parse::<WebCryptoKeyAlgorithm>().ok())
    else {
        return Err(WebCryptoRejection::NotSupported);
    };
    if let Some(rejection) = key_export_format_error(format, algorithm, &key_type) {
        return Err(rejection);
    }
    export_key_snapshot_from_crypto_key(scope, key, format, algorithm, key_type)
}

pub(crate) fn key_export_format_error(
    format: SubtleKeyFormat,
    algorithm: WebCryptoKeyAlgorithm,
    key_type: &str,
) -> Option<WebCryptoRejection> {
    let supported = matches!(
        (format, algorithm, key_type),
        (
            SubtleKeyFormat::Raw | SubtleKeyFormat::RawSecret | SubtleKeyFormat::Jwk,
            WebCryptoKeyAlgorithm::AesCbc
                | WebCryptoKeyAlgorithm::AesCtr
                | WebCryptoKeyAlgorithm::AesGcm
                | WebCryptoKeyAlgorithm::AesKw
                | WebCryptoKeyAlgorithm::Hmac,
            "secret"
        ) | (
            SubtleKeyFormat::RawSecret | SubtleKeyFormat::Jwk,
            WebCryptoKeyAlgorithm::Chacha20Poly1305,
            "secret"
        ) | (
            SubtleKeyFormat::Raw
                | SubtleKeyFormat::RawPublic
                | SubtleKeyFormat::Spki
                | SubtleKeyFormat::Jwk,
            WebCryptoKeyAlgorithm::X25519
                | WebCryptoKeyAlgorithm::X448
                | WebCryptoKeyAlgorithm::Ed25519
                | WebCryptoKeyAlgorithm::Ed448
                | WebCryptoKeyAlgorithm::Ecdh
                | WebCryptoKeyAlgorithm::Ecdsa,
            "public"
        ) | (
            SubtleKeyFormat::Spki | SubtleKeyFormat::Jwk,
            WebCryptoKeyAlgorithm::RsaOaep
                | WebCryptoKeyAlgorithm::RsaPss
                | WebCryptoKeyAlgorithm::RsassaPkcs1V15,
            "public"
        ) | (
            SubtleKeyFormat::Pkcs8 | SubtleKeyFormat::Jwk,
            WebCryptoKeyAlgorithm::X25519
                | WebCryptoKeyAlgorithm::X448
                | WebCryptoKeyAlgorithm::Ed25519
                | WebCryptoKeyAlgorithm::Ed448
                | WebCryptoKeyAlgorithm::Ecdh
                | WebCryptoKeyAlgorithm::Ecdsa
                | WebCryptoKeyAlgorithm::RsaOaep
                | WebCryptoKeyAlgorithm::RsaPss
                | WebCryptoKeyAlgorithm::RsassaPkcs1V15,
            "private"
        )
    );
    if supported {
        return None;
    }
    Some(match (format, algorithm, key_type) {
        // Asymmetric keys implement these formats only for one side of the
        // pair. Chromium reports key-type mismatches as InvalidAccessError;
        // genuinely unsupported format/algorithm pairs remain NotSupportedError.
        (
            SubtleKeyFormat::Raw | SubtleKeyFormat::RawPublic,
            WebCryptoKeyAlgorithm::X25519
            | WebCryptoKeyAlgorithm::X448
            | WebCryptoKeyAlgorithm::Ed25519
            | WebCryptoKeyAlgorithm::Ed448
            | WebCryptoKeyAlgorithm::Ecdh
            | WebCryptoKeyAlgorithm::Ecdsa,
            "private",
        )
        | (
            SubtleKeyFormat::Spki,
            WebCryptoKeyAlgorithm::X25519
            | WebCryptoKeyAlgorithm::X448
            | WebCryptoKeyAlgorithm::Ed25519
            | WebCryptoKeyAlgorithm::Ed448
            | WebCryptoKeyAlgorithm::Ecdh
            | WebCryptoKeyAlgorithm::Ecdsa
            | WebCryptoKeyAlgorithm::RsaOaep
            | WebCryptoKeyAlgorithm::RsaPss
            | WebCryptoKeyAlgorithm::RsassaPkcs1V15,
            "private",
        )
        | (
            SubtleKeyFormat::Pkcs8,
            WebCryptoKeyAlgorithm::X25519
            | WebCryptoKeyAlgorithm::X448
            | WebCryptoKeyAlgorithm::Ed25519
            | WebCryptoKeyAlgorithm::Ed448
            | WebCryptoKeyAlgorithm::Ecdh
            | WebCryptoKeyAlgorithm::Ecdsa
            | WebCryptoKeyAlgorithm::RsaOaep
            | WebCryptoKeyAlgorithm::RsaPss
            | WebCryptoKeyAlgorithm::RsassaPkcs1V15,
            "public",
        ) => WebCryptoRejection::InvalidAccess,
        _ => WebCryptoRejection::NotSupported,
    })
}

pub(crate) struct NormalizedCipherAlgorithm {
    pub(crate) algorithm: WebCryptoKeyAlgorithm,
    pub(crate) params: NormalizedCipherOperationParams,
}

pub(crate) enum NormalizedCipherOperationParams {
    Cbc {
        iv: Vec<u8>,
    },
    Ctr {
        counter: Vec<u8>,
        length_bits: u8,
    },
    Gcm {
        iv: Vec<u8>,
        additional_data: Vec<u8>,
        tag_length_bits: usize,
    },
    Chacha20Poly1305 {
        iv: Vec<u8>,
        additional_data: Vec<u8>,
        tag_length_bits: usize,
    },
    Kw,
}

pub(crate) fn normalize_symmetric_cipher_algorithm_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm_value: v8::Local<'s, v8::Value>,
) -> Result<NormalizedCipherAlgorithm, WebCryptoRejection> {
    let algorithm = normalize_symmetric_algorithm_name_rejection(scope, algorithm_value)?;
    match algorithm {
        WebCryptoKeyAlgorithm::AesCbc
        | WebCryptoKeyAlgorithm::AesCtr
        | WebCryptoKeyAlgorithm::AesGcm
        | WebCryptoKeyAlgorithm::Chacha20Poly1305 => {
            let params =
                normalize_symmetric_operation_params_rejection(scope, algorithm_value, algorithm)?;
            Ok(NormalizedCipherAlgorithm { algorithm, params })
        }
        _ => Err(WebCryptoRejection::NotSupported),
    }
}

pub(crate) fn validate_symmetric_wrapping_algorithm_rejection(
    algorithm: WebCryptoKeyAlgorithm,
) -> Result<WebCryptoKeyAlgorithm, WebCryptoRejection> {
    match algorithm {
        WebCryptoKeyAlgorithm::AesCbc
        | WebCryptoKeyAlgorithm::AesCtr
        | WebCryptoKeyAlgorithm::AesGcm
        | WebCryptoKeyAlgorithm::AesKw
        | WebCryptoKeyAlgorithm::Chacha20Poly1305 => Ok(algorithm),
        _ => Err(WebCryptoRejection::NotSupported),
    }
}

pub(crate) fn normalize_symmetric_wrapping_params_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm_value: v8::Local<'s, v8::Value>,
    algorithm: WebCryptoKeyAlgorithm,
) -> Result<NormalizedCipherOperationParams, WebCryptoRejection> {
    match algorithm {
        WebCryptoKeyAlgorithm::AesCbc
        | WebCryptoKeyAlgorithm::AesCtr
        | WebCryptoKeyAlgorithm::AesGcm
        | WebCryptoKeyAlgorithm::Chacha20Poly1305 => {
            normalize_symmetric_operation_params_rejection(scope, algorithm_value, algorithm)
        }
        WebCryptoKeyAlgorithm::AesKw => Ok(NormalizedCipherOperationParams::Kw),
        _ => Err(WebCryptoRejection::NotSupported),
    }
}

pub(crate) fn normalize_symmetric_algorithm_name_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm_value: v8::Local<'s, v8::Value>,
) -> Result<WebCryptoKeyAlgorithm, WebCryptoRejection> {
    let Some(name) = crypto_algorithm_name(scope, algorithm_value) else {
        return Err(WebCryptoRejection::Type);
    };
    name.parse::<WebCryptoKeyAlgorithm>()
        .map_err(|_| WebCryptoRejection::NotSupported)
}

pub(crate) fn normalize_symmetric_operation_params_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm_value: v8::Local<'s, v8::Value>,
    algorithm: WebCryptoKeyAlgorithm,
) -> Result<NormalizedCipherOperationParams, WebCryptoRejection> {
    let Some(algorithm_object) = algorithm_value.to_object(scope) else {
        return Err(WebCryptoRejection::Type);
    };
    match algorithm {
        WebCryptoKeyAlgorithm::AesCbc => {
            let iv = required_buffer_source_member_with_max_rejection(
                scope,
                algorithm_object,
                "iv",
                MAX_AES_OPERATION_BYTES,
            )?;
            if iv.len() == 16 {
                Ok(NormalizedCipherOperationParams::Cbc { iv })
            } else {
                Err(WebCryptoRejection::Operation)
            }
        }
        WebCryptoKeyAlgorithm::AesCtr => {
            let counter = required_buffer_source_member_with_max_rejection(
                scope,
                algorithm_object,
                "counter",
                MAX_AES_OPERATION_BYTES,
            )?;
            let length =
                required_enforce_range_octet_member_rejection(scope, algorithm_object, "length")?;
            if counter.len() != 16 {
                return Err(WebCryptoRejection::Operation);
            }
            if (1..=128).contains(&length) {
                Ok(NormalizedCipherOperationParams::Ctr {
                    counter,
                    length_bits: length as u8,
                })
            } else {
                Err(WebCryptoRejection::Operation)
            }
        }
        WebCryptoKeyAlgorithm::AesGcm => {
            let iv = required_buffer_source_member_with_max_rejection(
                scope,
                algorithm_object,
                "iv",
                MAX_AES_OPERATION_BYTES,
            )?;
            let additional_data = optional_buffer_source_member_with_max_rejection(
                scope,
                algorithm_object,
                "additionalData",
                MAX_AES_OPERATION_BYTES,
            )?
            .unwrap_or_default();
            let tag_length = optional_enforce_range_octet_member_rejection(
                scope,
                algorithm_object,
                "tagLength",
            )?
            .unwrap_or(128);
            if matches!(tag_length, 32 | 64 | 96 | 104 | 112 | 120 | 128) {
                Ok(NormalizedCipherOperationParams::Gcm {
                    iv,
                    additional_data,
                    tag_length_bits: tag_length as usize,
                })
            } else {
                Err(WebCryptoRejection::Operation)
            }
        }
        WebCryptoKeyAlgorithm::Chacha20Poly1305 => {
            let iv = required_buffer_source_member_with_max_rejection(
                scope,
                algorithm_object,
                "iv",
                MAX_AES_OPERATION_BYTES,
            )?;
            let additional_data = optional_buffer_source_member_with_max_rejection(
                scope,
                algorithm_object,
                "additionalData",
                MAX_AES_OPERATION_BYTES,
            )?
            .unwrap_or_default();
            let tag_length = optional_enforce_range_octet_member_rejection(
                scope,
                algorithm_object,
                "tagLength",
            )?
            .unwrap_or(128);
            if iv.len() == 12 && tag_length == 128 {
                Ok(NormalizedCipherOperationParams::Chacha20Poly1305 {
                    iv,
                    additional_data,
                    tag_length_bits: tag_length as usize,
                })
            } else {
                Err(WebCryptoRejection::Operation)
            }
        }
        _ => Err(WebCryptoRejection::NotSupported),
    }
}

pub(crate) fn run_symmetric_cipher_operation(
    operation: &str,
    params: &NormalizedCipherOperationParams,
    key: &[u8],
    data: &[u8],
) -> Result<Vec<u8>, WebCryptoError> {
    match (operation, params) {
        ("encrypt", NormalizedCipherOperationParams::Cbc { iv }) => aes_cbc_encrypt(key, iv, data),
        ("decrypt", NormalizedCipherOperationParams::Cbc { iv }) => aes_cbc_decrypt(key, iv, data),
        (
            "encrypt" | "decrypt",
            NormalizedCipherOperationParams::Ctr {
                counter,
                length_bits,
            },
        ) => aes_ctr_crypt(key, counter, *length_bits, data),
        (
            "encrypt",
            NormalizedCipherOperationParams::Gcm {
                iv,
                additional_data,
                tag_length_bits,
            },
        ) => aes_gcm_encrypt(key, iv, additional_data, *tag_length_bits, data),
        (
            "decrypt",
            NormalizedCipherOperationParams::Gcm {
                iv,
                additional_data,
                tag_length_bits,
            },
        ) => aes_gcm_decrypt(key, iv, additional_data, *tag_length_bits, data),
        (
            "encrypt",
            NormalizedCipherOperationParams::Chacha20Poly1305 {
                iv,
                additional_data,
                tag_length_bits,
            },
        ) => chacha20_poly1305_encrypt(key, iv, additional_data, *tag_length_bits, data),
        (
            "decrypt",
            NormalizedCipherOperationParams::Chacha20Poly1305 {
                iv,
                additional_data,
                tag_length_bits,
            },
        ) => chacha20_poly1305_decrypt(key, iv, additional_data, *tag_length_bits, data),
        ("wrapKey", NormalizedCipherOperationParams::Kw) => aes_kw_wrap(key, data),
        ("unwrapKey", NormalizedCipherOperationParams::Kw) => aes_kw_unwrap(key, data),
        ("wrapKey", params) => run_symmetric_cipher_operation("encrypt", params, key, data),
        ("unwrapKey", params) => run_symmetric_cipher_operation("decrypt", params, key, data),
        _ => Err(WebCryptoError::Operation),
    }
}

pub(crate) fn crypto_key_hmac_hash_algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
) -> Option<WebCryptoHashAlgorithm> {
    let algorithm = crypto_key_algorithm_object(scope, key)?;
    let hash = algorithm
        .get(scope, v8str(scope, "hash").into())
        .and_then(|value| crypto_algorithm_name(scope, value))?;
    hash.parse::<WebCryptoHashAlgorithm>().ok()
}

pub(crate) fn crypto_key_rsa_hash_algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
) -> Option<WebCryptoHashAlgorithm> {
    let algorithm = crypto_key_algorithm_object(scope, key)?;
    let hash = algorithm
        .get(scope, v8str(scope, "hash").into())
        .and_then(|value| crypto_algorithm_name(scope, value))?;
    hash.parse::<WebCryptoHashAlgorithm>().ok()
}

pub(crate) fn crypto_key_ec_named_curve<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
) -> Option<WebCryptoEcNamedCurve> {
    let algorithm = crypto_key_algorithm_object(scope, key)?;
    let named_curve = algorithm
        .get(scope, v8str(scope, "namedCurve").into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))?;
    parse_ec_named_curve(&named_curve)
}

pub(crate) fn rsa_key_gen_params<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm_object: v8::Local<'s, v8::Object>,
) -> Result<(usize, Vec<u8>, WebCryptoHashAlgorithm), WebCryptoRejection> {
    let Some(modulus_length_value) =
        algorithm_object.get(scope, v8str(scope, "modulusLength").into())
    else {
        return Err(WebCryptoRejection::Type);
    };
    if modulus_length_value.is_undefined() {
        return Err(WebCryptoRejection::Type);
    }
    let modulus_length_bits = webidl::convert::<webidl::EnforceRangeUnsignedLong>(
        scope,
        modulus_length_value,
        webidl::Context::member("RsaHashedKeyGenParams", "modulusLength"),
    )
    .map_err(|_| WebCryptoRejection::Type)?
    .0 as usize;
    if !(MIN_RSA_MODULUS_LENGTH_BITS..=MAX_RSA_MODULUS_LENGTH_BITS).contains(&modulus_length_bits) {
        return Err(WebCryptoRejection::Operation);
    }
    let public_exponent =
        required_buffer_source_member_rejection(scope, algorithm_object, "publicExponent")?;
    if public_exponent.is_empty() || public_exponent.len() > MAX_RSA_PUBLIC_EXPONENT_BYTES {
        return Err(WebCryptoRejection::Operation);
    }
    let public_exponent_without_leading_zeroes: Vec<_> = public_exponent
        .iter()
        .copied()
        .skip_while(|byte| *byte == 0)
        .collect();
    if !matches!(
        public_exponent_without_leading_zeroes.as_slice(),
        [3] | [1, 0, 1]
    ) {
        return Err(WebCryptoRejection::Operation);
    }
    let hash = required_hmac_hash_algorithm(scope, algorithm_object)?;
    Ok((modulus_length_bits, public_exponent, hash))
}

pub(crate) fn rsa_import_hash_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm_value: v8::Local<'s, v8::Value>,
) -> Result<WebCryptoHashAlgorithm, WebCryptoRejection> {
    let Some(algorithm_object) = algorithm_value.to_object(scope) else {
        return Err(WebCryptoRejection::Type);
    };
    required_hmac_hash_algorithm(scope, algorithm_object)
}

pub(crate) fn rsa_pss_salt_length_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm_value: v8::Local<'s, v8::Value>,
) -> Result<usize, WebCryptoRejection> {
    let Some(algorithm_object) = algorithm_value.to_object(scope) else {
        return Err(WebCryptoRejection::Type);
    };
    let Some(value) = algorithm_object.get(scope, v8str(scope, "saltLength").into()) else {
        return Err(WebCryptoRejection::Type);
    };
    if value.is_undefined() {
        return Err(WebCryptoRejection::Type);
    }
    Ok(webidl::convert::<webidl::EnforceRangeUnsignedLong>(
        scope,
        value,
        webidl::Context::member("RsaPssParams", "saltLength"),
    )
    .map_err(|_| WebCryptoRejection::Type)?
    .0 as usize)
}

pub(crate) fn rsa_oaep_operation_label_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm_value: v8::Local<'s, v8::Value>,
) -> Result<Vec<u8>, WebCryptoRejection> {
    let Some(algorithm_object) = algorithm_value.to_object(scope) else {
        return Err(WebCryptoRejection::Type);
    };
    optional_buffer_source_member_with_max_rejection(
        scope,
        algorithm_object,
        "label",
        MAX_RSA_OAEP_LABEL_BYTES,
    )
    .map(|label| label.unwrap_or_default())
}

pub(crate) fn ecdsa_operation_hash_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm_value: v8::Local<'s, v8::Value>,
) -> Result<WebCryptoHashAlgorithm, WebCryptoRejection> {
    let Some(algorithm_object) = algorithm_value.to_object(scope) else {
        return Err(WebCryptoRejection::Type);
    };
    required_hmac_hash_algorithm(scope, algorithm_object)
}

pub(crate) fn ec_named_curve_param<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm_object: v8::Local<'s, v8::Object>,
) -> Result<WebCryptoEcNamedCurve, WebCryptoRejection> {
    let Some(value) = algorithm_object.get(scope, v8str(scope, "namedCurve").into()) else {
        return Err(WebCryptoRejection::Type);
    };
    if value.is_undefined() {
        return Err(WebCryptoRejection::Type);
    }
    let named_curve = webidl::convert::<webidl::DomString>(
        scope,
        value,
        webidl::Context::member("EcKeyGenParams", "namedCurve"),
    )
    .map_err(|_| WebCryptoRejection::Type)?;
    parse_ec_named_curve(&named_curve.0).ok_or(WebCryptoRejection::NotSupported)
}

pub(crate) fn parse_ec_named_curve(value: &str) -> Option<WebCryptoEcNamedCurve> {
    match value {
        "P-256" => Some(WebCryptoEcNamedCurve::P256),
        "P-384" => Some(WebCryptoEcNamedCurve::P384),
        "P-521" => Some(WebCryptoEcNamedCurve::P521),
        _ => None,
    }
}

pub(crate) fn okp_curve_for_algorithm(
    algorithm: WebCryptoKeyAlgorithm,
) -> Option<WebCryptoOkpCurve> {
    match algorithm {
        WebCryptoKeyAlgorithm::Ed25519 => Some(WebCryptoOkpCurve::Ed25519),
        WebCryptoKeyAlgorithm::Ed448 => Some(WebCryptoOkpCurve::Ed448),
        WebCryptoKeyAlgorithm::X448 => Some(WebCryptoOkpCurve::X448),
        _ => None,
    }
}

pub(crate) fn required_okp_curve_for_algorithm_rejection(
    algorithm: WebCryptoKeyAlgorithm,
) -> Result<WebCryptoOkpCurve, WebCryptoRejection> {
    okp_curve_for_algorithm(algorithm).ok_or(WebCryptoRejection::NotSupported)
}

pub(crate) fn signing_rsa_key_material<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
    algorithm: WebCryptoKeyAlgorithm,
) -> Result<(Vec<u8>, WebCryptoHashAlgorithm), WebCryptoRejection> {
    if crypto_key_kind(scope, key).as_deref() != Some("private")
        || crypto_key_algorithm_name(scope, key).as_deref()
            != Some(crypto_algorithm_name_for_match(algorithm))
        || !crypto_key_has_usage(scope, key, "sign")
    {
        return Err(WebCryptoRejection::InvalidAccess);
    }
    let hash = crypto_key_rsa_hash_algorithm(scope, key).ok_or(WebCryptoRejection::Type)?;
    let bytes = crypto_key_bytes(scope, key).ok_or(WebCryptoRejection::Type)?;
    Ok((bytes, hash))
}

pub(crate) fn verifying_rsa_key_material<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
    algorithm: WebCryptoKeyAlgorithm,
) -> Result<(Vec<u8>, WebCryptoHashAlgorithm), WebCryptoRejection> {
    if crypto_key_kind(scope, key).as_deref() != Some("public")
        || crypto_key_algorithm_name(scope, key).as_deref()
            != Some(crypto_algorithm_name_for_match(algorithm))
        || !crypto_key_has_usage(scope, key, "verify")
    {
        return Err(WebCryptoRejection::InvalidAccess);
    }
    let hash = crypto_key_rsa_hash_algorithm(scope, key).ok_or(WebCryptoRejection::Type)?;
    let bytes = crypto_key_bytes(scope, key).ok_or(WebCryptoRejection::Type)?;
    Ok((bytes, hash))
}

pub(crate) fn rsa_oaep_key_material_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
    usage: &str,
) -> Result<(Vec<u8>, WebCryptoHashAlgorithm), WebCryptoRejection> {
    let expected_type = match usage {
        "encrypt" | "wrapKey" => "public",
        "decrypt" | "unwrapKey" => "private",
        _ => return Err(WebCryptoRejection::InvalidAccess),
    };
    if crypto_key_kind(scope, key).as_deref() != Some(expected_type)
        || crypto_key_algorithm_name(scope, key).as_deref() != Some("rsa-oaep")
        || !crypto_key_has_usage(scope, key, usage)
    {
        return Err(WebCryptoRejection::InvalidAccess);
    }
    let hash = crypto_key_rsa_hash_algorithm(scope, key).ok_or(WebCryptoRejection::Type)?;
    let bytes = crypto_key_bytes(scope, key).ok_or(WebCryptoRejection::Type)?;
    Ok((bytes, hash))
}

pub(crate) fn signing_ec_key_material<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
) -> Result<(Vec<u8>, WebCryptoEcNamedCurve), WebCryptoRejection> {
    if crypto_key_kind(scope, key).as_deref() != Some("private")
        || crypto_key_algorithm_name(scope, key).as_deref() != Some("ecdsa")
        || !crypto_key_has_usage(scope, key, "sign")
    {
        return Err(WebCryptoRejection::InvalidAccess);
    }
    let curve = crypto_key_ec_named_curve(scope, key).ok_or(WebCryptoRejection::Type)?;
    let bytes = crypto_key_bytes(scope, key).ok_or(WebCryptoRejection::Type)?;
    Ok((bytes, curve))
}

pub(crate) fn verifying_ec_key_material<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
) -> Result<(Vec<u8>, WebCryptoEcNamedCurve), WebCryptoRejection> {
    if crypto_key_kind(scope, key).as_deref() != Some("public")
        || crypto_key_algorithm_name(scope, key).as_deref() != Some("ecdsa")
        || !crypto_key_has_usage(scope, key, "verify")
    {
        return Err(WebCryptoRejection::InvalidAccess);
    }
    let curve = crypto_key_ec_named_curve(scope, key).ok_or(WebCryptoRejection::Type)?;
    let bytes = crypto_key_bytes(scope, key).ok_or(WebCryptoRejection::Type)?;
    Ok((bytes, curve))
}

pub(crate) fn signing_okp_key_material<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
    algorithm: WebCryptoKeyAlgorithm,
) -> Result<Vec<u8>, WebCryptoRejection> {
    if crypto_key_kind(scope, key).as_deref() != Some("private")
        || crypto_key_algorithm_name(scope, key).as_deref()
            != Some(crypto_algorithm_name_for_match(algorithm))
        || !crypto_key_has_usage(scope, key, "sign")
    {
        return Err(WebCryptoRejection::InvalidAccess);
    }
    crypto_key_bytes(scope, key).ok_or(WebCryptoRejection::Type)
}

pub(crate) fn verifying_okp_key_material<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
    algorithm: WebCryptoKeyAlgorithm,
) -> Result<Vec<u8>, WebCryptoRejection> {
    if crypto_key_kind(scope, key).as_deref() != Some("public")
        || crypto_key_algorithm_name(scope, key).as_deref()
            != Some(crypto_algorithm_name_for_match(algorithm))
        || !crypto_key_has_usage(scope, key, "verify")
    {
        return Err(WebCryptoRejection::InvalidAccess);
    }
    crypto_key_bytes(scope, key).ok_or(WebCryptoRejection::Type)
}

pub(crate) fn crypto_key_algorithm_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let algorithm = crypto_key_algorithm_object(scope, key)?;
    crypto_algorithm_name(scope, algorithm.into())
}

pub(crate) fn key_usages_are_valid(
    algorithm: WebCryptoKeyAlgorithm,
    key_type: &str,
    usages: &[String],
) -> bool {
    if key_type == "public" {
        // Public asymmetric keys are allowed to carry no usages. This is common
        // for ECDH/X* public keys, but RSA/ECDSA/EdDSA public imports also use
        // the same WebCrypto key-creation rule.
        return !key_usages_contain_invalid(algorithm, key_type, usages);
    }
    !usages.is_empty() && !key_usages_contain_invalid(algorithm, key_type, usages)
}

pub(crate) fn key_usages_contain_invalid(
    algorithm: WebCryptoKeyAlgorithm,
    key_type: &str,
    usages: &[String],
) -> bool {
    usages
        .iter()
        .any(|usage| !key_usage_is_valid(algorithm, key_type, usage))
}

pub(crate) fn key_pair_usages_contain_invalid(
    algorithm: WebCryptoKeyAlgorithm,
    usages: &[String],
) -> bool {
    usages.iter().any(|usage| {
        !key_usage_is_valid(algorithm, "private", usage)
            && !key_usage_is_valid(algorithm, "public", usage)
    })
}

pub(crate) fn key_pair_usages_for_key_type(
    algorithm: WebCryptoKeyAlgorithm,
    key_type: &str,
    usages: &[String],
) -> Vec<String> {
    usages
        .iter()
        .filter(|usage| key_usage_is_valid(algorithm, key_type, usage))
        .cloned()
        .collect()
}

pub(crate) fn key_usages_contain_unrecognized(usages: &[String]) -> bool {
    usages.iter().any(|usage| !key_usage_is_recognized(usage))
}

pub(crate) fn key_usage_is_recognized(usage: &str) -> bool {
    matches!(
        usage,
        "encrypt"
            | "decrypt"
            | "sign"
            | "verify"
            | "deriveKey"
            | "deriveBits"
            | "wrapKey"
            | "unwrapKey"
    )
}

pub(crate) fn key_usage_is_valid(
    algorithm: WebCryptoKeyAlgorithm,
    key_type: &str,
    usage: &str,
) -> bool {
    matches!(
        (algorithm, key_type, usage),
        (
            WebCryptoKeyAlgorithm::AesCbc
                | WebCryptoKeyAlgorithm::AesCtr
                | WebCryptoKeyAlgorithm::AesGcm
                | WebCryptoKeyAlgorithm::Chacha20Poly1305,
            "secret",
            "encrypt" | "decrypt" | "wrapKey" | "unwrapKey"
        ) | (
            WebCryptoKeyAlgorithm::AesKw,
            "secret",
            "wrapKey" | "unwrapKey"
        ) | (
            WebCryptoKeyAlgorithm::Hkdf | WebCryptoKeyAlgorithm::Pbkdf2,
            "secret",
            "deriveBits" | "deriveKey"
        ) | (WebCryptoKeyAlgorithm::Hmac, "secret", "sign" | "verify")
            | (
                WebCryptoKeyAlgorithm::RsaOaep,
                "public",
                "encrypt" | "wrapKey"
            )
            | (
                WebCryptoKeyAlgorithm::RsaOaep,
                "private",
                "decrypt" | "unwrapKey"
            )
            | (
                WebCryptoKeyAlgorithm::RsaPss | WebCryptoKeyAlgorithm::RsassaPkcs1V15,
                "private",
                "sign"
            )
            | (
                WebCryptoKeyAlgorithm::RsaPss | WebCryptoKeyAlgorithm::RsassaPkcs1V15,
                "public",
                "verify"
            )
            | (WebCryptoKeyAlgorithm::Ecdsa, "private", "sign")
            | (WebCryptoKeyAlgorithm::Ecdsa, "public", "verify")
            | (
                WebCryptoKeyAlgorithm::Ecdh,
                "private",
                "deriveBits" | "deriveKey"
            )
            | (
                WebCryptoKeyAlgorithm::Ed25519 | WebCryptoKeyAlgorithm::Ed448,
                "private",
                "sign"
            )
            | (
                WebCryptoKeyAlgorithm::Ed25519 | WebCryptoKeyAlgorithm::Ed448,
                "public",
                "verify"
            )
            | (
                WebCryptoKeyAlgorithm::X25519,
                "private",
                "deriveBits" | "deriveKey"
            )
            | (
                WebCryptoKeyAlgorithm::X448,
                "private",
                "deriveBits" | "deriveKey"
            )
    )
}

pub(crate) fn webcrypto_algorithm_display_name(algorithm: WebCryptoKeyAlgorithm) -> &'static str {
    match algorithm {
        WebCryptoKeyAlgorithm::AesCbc => "AES-CBC",
        WebCryptoKeyAlgorithm::AesCtr => "AES-CTR",
        WebCryptoKeyAlgorithm::AesGcm => "AES-GCM",
        WebCryptoKeyAlgorithm::AesKw => "AES-KW",
        WebCryptoKeyAlgorithm::Chacha20Poly1305 => "ChaCha20-Poly1305",
        WebCryptoKeyAlgorithm::Hkdf => "HKDF",
        WebCryptoKeyAlgorithm::Hmac => "HMAC",
        WebCryptoKeyAlgorithm::Pbkdf2 => "PBKDF2",
        WebCryptoKeyAlgorithm::RsaOaep => "RSA-OAEP",
        WebCryptoKeyAlgorithm::RsaPss => "RSA-PSS",
        WebCryptoKeyAlgorithm::RsassaPkcs1V15 => "RSASSA-PKCS1-v1_5",
        WebCryptoKeyAlgorithm::Ecdh => "ECDH",
        WebCryptoKeyAlgorithm::Ecdsa => "ECDSA",
        WebCryptoKeyAlgorithm::Ed25519 => "Ed25519",
        WebCryptoKeyAlgorithm::Ed448 => "Ed448",
        WebCryptoKeyAlgorithm::X25519 => "X25519",
        WebCryptoKeyAlgorithm::X448 => "X448",
    }
}

pub(crate) fn subtle_buffer_source_arg_with_max_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    prefix: &'static str,
    max_bytes: usize,
    detached_empty_if_preflighted: bool,
) -> Result<Vec<u8>, WebCryptoRejection> {
    subtle_buffer_source_value_with_max_rejection(
        scope,
        args.get(index),
        webidl::Context::argument(prefix, (index + 1) as usize),
        max_bytes,
        detached_empty_if_preflighted,
    )
}

pub(crate) fn subtle_buffer_source_value_after_preflight<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
    detached_empty_if_preflighted: bool,
) -> Result<Vec<u8>, webidl::WebIdlError> {
    if !detached_empty_if_preflighted && buffer_source_value_is_shared(value) {
        return Err(webidl::WebIdlError::custom_message(
            "SharedArrayBuffer-backed BufferSource values are not accepted",
        ));
    }
    match webidl::convert::<webidl::BufferSource>(scope, value, context) {
        Ok(value) => Ok(value.into_bytes()),
        Err(_) if detached_empty_if_preflighted || buffer_source_value_is_detached_view(value) => {
            Ok(Vec::new())
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn subtle_buffer_source_value_with_max_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
    max_bytes: usize,
    detached_empty_if_preflighted: bool,
) -> Result<Vec<u8>, WebCryptoRejection> {
    if !detached_empty_if_preflighted && buffer_source_value_is_shared(value) {
        return Err(WebCryptoRejection::Type);
    }
    let byte_length = match buffer_source_value_byte_length(value) {
        Some(byte_length) => byte_length,
        None if detached_empty_if_preflighted => 0,
        None => return Err(WebCryptoRejection::Type),
    };
    if byte_length > max_bytes {
        return Err(WebCryptoRejection::Operation);
    }
    match webidl::convert::<webidl::BufferSource>(scope, value, context) {
        Ok(value) => Ok(value.into_bytes()),
        Err(_) if detached_empty_if_preflighted || buffer_source_value_is_detached_view(value) => {
            Ok(Vec::new())
        }
        Err(_) => Err(WebCryptoRejection::Type),
    }
}

pub(crate) fn buffer_source_value_is_acceptable(value: v8::Local<'_, v8::Value>) -> bool {
    buffer_source_value_can_be_detached_to_empty(value)
}

pub(crate) fn buffer_source_value_can_be_detached_to_empty(
    value: v8::Local<'_, v8::Value>,
) -> bool {
    !buffer_source_value_is_shared(value)
        && (v8::Local::<v8::ArrayBufferView>::try_from(value).is_ok()
            || v8::Local::<v8::ArrayBuffer>::try_from(value).is_ok()
            || buffer_source_value_has_array_buffer_view_tag(value))
}

pub(crate) fn buffer_source_value_byte_length(value: v8::Local<'_, v8::Value>) -> Option<usize> {
    if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
        return Some(view.byte_length());
    }
    if buffer_source_value_has_array_buffer_view_tag(value) {
        return Some(0);
    }
    v8::Local::<v8::ArrayBuffer>::try_from(value)
        .ok()
        .map(|buffer| buffer.byte_length())
}

pub(crate) fn buffer_source_value_is_detached_view(value: v8::Local<'_, v8::Value>) -> bool {
    buffer_source_value_has_array_buffer_view_tag(value)
        && v8::Local::<v8::ArrayBufferView>::try_from(value).is_err()
}

pub(crate) fn buffer_source_value_has_array_buffer_view_tag(
    value: v8::Local<'_, v8::Value>,
) -> bool {
    value.is_int8_array()
        || value.is_uint8_array()
        || value.is_uint8_clamped_array()
        || value.is_int16_array()
        || value.is_uint16_array()
        || value.is_int32_array()
        || value.is_uint32_array()
        || value.is_big_int64_array()
        || value.is_big_uint64_array()
        || value.is_float32_array()
        || value.is_float64_array()
        || value.is_data_view()
}

pub(crate) fn buffer_source_value_is_shared(value: v8::Local<'_, v8::Value>) -> bool {
    if value.is_shared_array_buffer() {
        return true;
    }
    v8::Local::<v8::ArrayBufferView>::try_from(value)
        .ok()
        .and_then(|view| view.get_backing_store())
        .is_some_and(|backing_store| backing_store.is_shared())
}

pub(crate) fn subtle_required_arg<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    prefix: &'static str,
) -> Result<T, webidl::WebIdlError>
where
    T: webidl::WebIdlConverter<'s>,
{
    let context = webidl::Context::argument(prefix, (index + 1) as usize);
    if args.length() <= index {
        return Err(webidl::WebIdlError::missing_required(context));
    }
    webidl::argument::<T>(scope, args, index, context)
}

pub(crate) fn subtle_key_usages_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    prefix: &'static str,
) -> Result<Vec<String>, webidl::WebIdlError> {
    let usages =
        subtle_required_arg::<webidl::Sequence<webidl::DomString>>(scope, args, index, prefix)?;
    let usages: Vec<_> = usages.0.into_iter().map(|value| value.0).collect();
    if key_usages_contain_unrecognized(&usages) {
        return Err(webidl::WebIdlError::custom_message("invalid KeyUsage"));
    }
    Ok(usages)
}

pub(crate) fn required_hmac_hash_algorithm(
    scope: &mut v8::PinScope<'_, '_>,
    algorithm_object: v8::Local<'_, v8::Object>,
) -> Result<WebCryptoHashAlgorithm, WebCryptoRejection> {
    let Some(hash_value) = algorithm_object.get(scope, v8str(scope, "hash").into()) else {
        return Err(WebCryptoRejection::Type);
    };
    if hash_value.is_undefined() {
        return Err(WebCryptoRejection::Type);
    }
    let Some(hash_name) = crypto_algorithm_name(scope, hash_value) else {
        return Err(WebCryptoRejection::Type);
    };
    hash_name
        .parse::<WebCryptoHashAlgorithm>()
        .map_err(|_| WebCryptoRejection::NotSupported)
}

pub(crate) fn optional_hmac_length_bits(
    scope: &mut v8::PinScope<'_, '_>,
    algorithm_object: v8::Local<'_, v8::Object>,
    context: &'static str,
) -> Result<Option<usize>, WebCryptoRejection> {
    let length_key = v8str(scope, "length");
    if !algorithm_object
        .has(scope, length_key.into())
        .unwrap_or(false)
    {
        return Ok(None);
    }
    let Some(length_value) = algorithm_object.get(scope, length_key.into()) else {
        return Err(WebCryptoRejection::Type);
    };
    let Ok(length_bits) = webidl::convert::<webidl::EnforceRangeUnsignedLong>(
        scope,
        length_value,
        webidl::Context::member(context, "length"),
    ) else {
        return Err(WebCryptoRejection::Type);
    };
    Ok(Some(length_bits.0 as usize))
}

pub(crate) fn hmac_generate_key_length_bits(
    scope: &mut v8::PinScope<'_, '_>,
    algorithm_object: v8::Local<'_, v8::Object>,
    hash: WebCryptoHashAlgorithm,
) -> Result<usize, WebCryptoRejection> {
    let Some(length_bits) = optional_hmac_length_bits(scope, algorithm_object, "HmacKeyGenParams")?
    else {
        return Ok(hash.default_hmac_key_len_bytes() * 8);
    };
    if length_bits == 0 || length_bits > MAX_HMAC_KEY_LENGTH_BITS {
        return Err(WebCryptoRejection::Operation);
    }
    Ok(length_bits)
}

pub(crate) fn hmac_derived_key_length_bits(
    scope: &mut v8::PinScope<'_, '_>,
    algorithm_object: v8::Local<'_, v8::Object>,
    hash: WebCryptoHashAlgorithm,
) -> Result<usize, WebCryptoRejection> {
    let Some(length_bits) = optional_hmac_length_bits(scope, algorithm_object, "HmacKeyGenParams")?
    else {
        return Ok(hash.default_hmac_key_len_bytes() * 8);
    };
    if length_bits == 0 || length_bits > MAX_HMAC_KEY_LENGTH_BITS {
        return Err(WebCryptoRejection::Type);
    }
    // Chromium's HMAC GetKeyLength accepts non-byte-aligned lengths. The
    // source algorithm decides whether it can derive that many bits.
    Ok(length_bits)
}

pub(crate) struct HmacImportParams {
    pub(crate) hash: WebCryptoHashAlgorithm,
    pub(crate) length_bits: Option<usize>,
}

pub(crate) fn hmac_import_params_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    algorithm: v8::Local<'s, v8::Value>,
) -> Result<HmacImportParams, WebCryptoRejection> {
    let Some(algorithm_object) = algorithm.to_object(scope) else {
        return Err(WebCryptoRejection::Type);
    };
    let hash = required_hmac_hash_algorithm(scope, algorithm_object)?;
    let length_bits = optional_hmac_length_bits(scope, algorithm_object, "HmacImportParams")?;
    Ok(HmacImportParams { hash, length_bits })
}

pub(crate) fn aes_key_length_bits(
    scope: &mut v8::PinScope<'_, '_>,
    algorithm_object: v8::Local<'_, v8::Object>,
    context: &'static str,
) -> Result<usize, WebCryptoRejection> {
    let Some(length_value) = algorithm_object.get(scope, v8str(scope, "length").into()) else {
        return Err(WebCryptoRejection::Type);
    };
    let length_bits = webidl::convert::<webidl::EnforceRangeUnsignedLong>(
        scope,
        length_value,
        webidl::Context::member(context, "length"),
    )
    .map_err(|_| WebCryptoRejection::Type)?
    .0;
    if length_bits > u16::MAX.into() {
        return Err(WebCryptoRejection::Type);
    }
    if matches!(length_bits, 128 | 192 | 256) {
        return Ok(length_bits as usize);
    }
    Err(WebCryptoRejection::Operation)
}
