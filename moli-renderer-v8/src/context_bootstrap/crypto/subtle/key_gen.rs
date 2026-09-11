use super::*;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(record)]
struct CryptoKeyPairDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    private_key: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, enumerable)]
    public_key: v8::Local<'scope, v8::Object>,
}

pub(crate) fn crypto_subtle_digest_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !subtle_crypto_receiver_is_valid(scope, &args, &mut rv) {
        return;
    }
    let Some(promise) = PendingCryptoPromise::new(scope, &mut rv) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let data_preflight = buffer_source_value_can_be_detached_to_empty(args.get(1));
    let Some(algorithm) = crypto_algorithm_name(scope, args.get(0)) else {
        promise.reject_webcrypto(scope, WebCryptoRejection::Type);
        return;
    };
    let Ok(algorithm) = algorithm.parse::<WebCryptoHashAlgorithm>() else {
        promise.reject_webcrypto(scope, WebCryptoRejection::NotSupported);
        return;
    };
    // WebCrypto normalizes the algorithm before converting operation
    // BufferSource arguments. Getter side effects during normalization are
    // therefore visible to the BufferSource conversion, while changes after the
    // method returns are not.
    let data = match subtle_buffer_source_arg_with_max_rejection(
        scope,
        &args,
        1,
        "SubtleCrypto.digest",
        MAX_DIGEST_OPERATION_BYTES,
        data_preflight,
    ) {
        Ok(data) => data,
        Err(rejection) => {
            promise.reject_webcrypto(scope, rejection);
            return;
        }
    };
    if let Some((handle, completion_tx)) = register_webcrypto_task(scope, &promise) {
        spawn_webcrypto_bytes_task(handle, completion_tx, move || {
            algorithm.digest_with_limit(data)
        });
        return;
    }

    let bytes = match algorithm.digest_with_limit(data) {
        Ok(bytes) => bytes,
        Err(error) => {
            promise.reject_webcrypto(scope, WebCryptoRejection::from(error));
            return;
        }
    };
    match blob::array_buffer_from_bytes(scope, bytes) {
        Some(buffer) => promise.resolve(scope, buffer.into()),
        None => promise.reject_webcrypto(scope, WebCryptoRejection::Type),
    }
}

pub(crate) fn resolve_crypto_key_pair_payloads<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    promise: &PendingCryptoPromise<'s>,
    private_payload: CryptoKeyClonePayload,
    public_payload: CryptoKeyClonePayload,
) {
    let private_key = crypto_key_object_from_clone_payload(scope, private_payload);
    let public_key = crypto_key_object_from_clone_payload(scope, public_payload);
    match (private_key, public_key) {
        (Some(private_key), Some(public_key)) => {
            let pair = CryptoKeyPairDeclaration {
                private_key,
                public_key,
            }
            .bind(scope)
            .expect("CryptoKeyPair declaration should bind");
            promise.resolve(scope, pair.into());
        }
        _ => promise.reject_webcrypto(scope, WebCryptoRejection::Type),
    }
}

pub(crate) fn resolve_crypto_key_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    promise: &PendingCryptoPromise<'s>,
    payload: CryptoKeyClonePayload,
) {
    match crypto_key_object_from_clone_payload(scope, payload) {
        Some(key) => promise.resolve(scope, key.into()),
        None => promise.reject_webcrypto(scope, WebCryptoRejection::Type),
    }
}

pub(crate) fn generated_rsa_key_pair_payloads(
    algorithm: WebCryptoKeyAlgorithm,
    hash: WebCryptoHashAlgorithm,
    private_extractable: bool,
    requested_usages: &[String],
    key_pair: RsaKeyPair,
) -> (CryptoKeyClonePayload, CryptoKeyClonePayload) {
    let algorithm_payload = CryptoKeyAlgorithmClonePayload {
        name: webcrypto_algorithm_display_name(algorithm).to_owned(),
        hash_name: Some(hash.as_ref().to_ascii_uppercase()),
        length_bits: None,
        named_curve: None,
        modulus_length_bits: Some(key_pair.modulus_length_bits),
        public_exponent: Some(key_pair.public_exponent.clone()),
    };
    let private_key = CryptoKeyClonePayload {
        key_type: "private".to_owned(),
        algorithm: algorithm_payload.clone(),
        extractable: private_extractable,
        usages: key_pair_usages_for_key_type(algorithm, "private", requested_usages),
        key_bytes: key_pair.private_key.as_slice().to_vec(),
    };
    let public_key = CryptoKeyClonePayload {
        key_type: "public".to_owned(),
        algorithm: algorithm_payload,
        extractable: true,
        usages: key_pair_usages_for_key_type(algorithm, "public", requested_usages),
        key_bytes: key_pair.public_key,
    };
    (private_key, public_key)
}

pub(crate) fn generated_ec_key_pair_payloads(
    algorithm: WebCryptoKeyAlgorithm,
    curve: WebCryptoEcNamedCurve,
    private_extractable: bool,
    requested_usages: &[String],
    key_pair: EcKeyPair,
) -> (CryptoKeyClonePayload, CryptoKeyClonePayload) {
    let algorithm_payload = CryptoKeyAlgorithmClonePayload {
        name: webcrypto_algorithm_display_name(algorithm).to_owned(),
        hash_name: None,
        length_bits: None,
        named_curve: Some(curve.name().to_owned()),
        modulus_length_bits: None,
        public_exponent: None,
    };
    let private_key = CryptoKeyClonePayload {
        key_type: "private".to_owned(),
        algorithm: algorithm_payload.clone(),
        extractable: private_extractable,
        usages: key_pair_usages_for_key_type(algorithm, "private", requested_usages),
        key_bytes: key_pair.private_key.as_slice().to_vec(),
    };
    let public_key = CryptoKeyClonePayload {
        key_type: "public".to_owned(),
        algorithm: algorithm_payload,
        extractable: true,
        usages: key_pair_usages_for_key_type(algorithm, "public", requested_usages),
        key_bytes: key_pair.public_key,
    };
    (private_key, public_key)
}

pub(crate) fn generated_okp_key_pair_payloads(
    algorithm: WebCryptoKeyAlgorithm,
    private_extractable: bool,
    requested_usages: &[String],
    key_pair: OkpKeyPair,
) -> (CryptoKeyClonePayload, CryptoKeyClonePayload) {
    let algorithm_payload = CryptoKeyAlgorithmClonePayload {
        name: webcrypto_algorithm_display_name(algorithm).to_owned(),
        hash_name: None,
        length_bits: None,
        named_curve: None,
        modulus_length_bits: None,
        public_exponent: None,
    };
    let private_key = CryptoKeyClonePayload {
        key_type: "private".to_owned(),
        algorithm: algorithm_payload.clone(),
        extractable: private_extractable,
        usages: key_pair_usages_for_key_type(algorithm, "private", requested_usages),
        key_bytes: key_pair.private_key.as_slice().to_vec(),
    };
    let public_key = CryptoKeyClonePayload {
        key_type: "public".to_owned(),
        algorithm: algorithm_payload,
        extractable: true,
        usages: key_pair_usages_for_key_type(algorithm, "public", requested_usages),
        key_bytes: key_pair.public_key,
    };
    (private_key, public_key)
}

pub(crate) fn generated_x25519_key_pair_payloads(
    private_extractable: bool,
    requested_usages: &[String],
    key_pair: X25519KeyPair,
) -> (CryptoKeyClonePayload, CryptoKeyClonePayload) {
    let algorithm_payload = CryptoKeyAlgorithmClonePayload {
        name: "X25519".to_owned(),
        hash_name: None,
        length_bits: None,
        named_curve: None,
        modulus_length_bits: None,
        public_exponent: None,
    };
    let private_key = CryptoKeyClonePayload {
        key_type: "private".to_owned(),
        algorithm: algorithm_payload.clone(),
        extractable: private_extractable,
        usages: requested_usages.to_vec(),
        key_bytes: key_pair.private_key.as_slice().to_vec(),
    };
    let public_key = CryptoKeyClonePayload {
        key_type: "public".to_owned(),
        algorithm: algorithm_payload,
        extractable: true,
        usages: Vec::new(),
        key_bytes: key_pair.public_key.to_vec(),
    };
    (private_key, public_key)
}
