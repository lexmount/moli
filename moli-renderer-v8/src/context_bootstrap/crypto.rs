use super::*;

mod helpers;
mod install;
use moli_webapi::crypto_key as keys;
mod random;
mod subtle;

pub(crate) use helpers::WebCryptoRejection;
pub(in crate::context_bootstrap) use install::{
    build_window_crypto_for_receiver, ensure_worker_crypto_for_global,
    finalize_crypto_realm_bindings, install_crypto_template_bindings,
    install_window_crypto_runtime_state, install_worker_crypto_runtime_state,
};
pub(crate) use keys::{
    CryptoKeyAlgorithmClonePayload, CryptoKeyClonePayload, crypto_key_clone_payload_from_object,
};
pub(in crate::context_bootstrap) use subtle::register_crypto_resolver_task;

/// Owner-neutral result of one blocking WebCrypto operation.
///
/// Page and Worker event loops wrap this payload in their own exact-owner task
/// envelopes; the payload itself does not authorize either executor.
#[derive(Debug)]
pub(crate) enum WebCryptoTaskResult {
    RtcCertificate(Box<super::rtc_certificate::CertificatePayload>),
    Bytes(Vec<u8>),
    Bool(bool),
    JsonWebKey(serde_json::Value),
    CryptoKey(Box<CryptoKeyClonePayload>),
    CryptoKeyPair {
        private_key: Box<CryptoKeyClonePayload>,
        public_key: Box<CryptoKeyClonePayload>,
    },
}

pub(crate) fn crypto_key_object_from_clone_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: CryptoKeyClonePayload,
) -> Option<v8::Local<'s, v8::Object>> {
    if !super::exposed_interfaces::is_realm_interface_exposed(scope, "CryptoKey") {
        return None;
    }
    super::ensure_intrinsic_interface_constructor(scope, "CryptoKey").ok()?;
    keys::crypto_key_object_from_clone_payload(scope, payload)
}
