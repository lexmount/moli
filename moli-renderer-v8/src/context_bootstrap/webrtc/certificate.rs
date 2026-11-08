//! Certificate generation, immutable native key leases and DTLS fingerprints.

use super::super::{
    crypto::{WebCryptoRejection, WebCryptoTaskResult, register_crypto_resolver_task},
    exposed_interfaces,
};
use crate::{
    native_bridge::WindowOriginKey,
    util::{context_host_ptr_from_global_bridge, get_private_value, set_private_value, v8str},
    web_api_interfaces, webidl,
};
use moli_crypto::{CertificateKeyAlgorithm, SelfSignedCertificate};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const ID: &str = "__moliRtcCertificateNativeHandle";
const CONNECTION_CERTIFICATES: &str = "__moliRtcConnectionCertificates";
const DEFAULT_LIFETIME: u64 = 2_592_000_000;
const MAX_LIFETIME: u64 = 31_536_000_000;

#[derive(Clone, Debug)]
pub(crate) struct CertificatePayload {
    pub(crate) certificate: SelfSignedCertificate,
    origin: WindowOriginKey,
}

#[derive(Default)]
struct Certificates {
    next_id: u64,
    entries: HashMap<u64, (v8::Weak<v8::Object>, CertificatePayload)>,
}
type Store = Rc<RefCell<Certificates>>;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCCertificate)]
struct CertificateSlots<'s> {
    #[webapi(slot = ID)]
    id: v8::Local<'s, v8::BigInt>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCCertificate, enumerable, receiver)]
struct CertificatePrototype {
    #[webapi(accessor_property, getter = expires)]
    expires: (),
    #[webapi(method, length = 0, callback = get_fingerprints)]
    get_fingerprints: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCPeerConnection, enumerable)]
struct CertificateStatics {
    #[webapi(static_method, returns_promise, length = 1, callback = generate_certificate)]
    generate_certificate: (),
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct Fingerprint {
    algorithm: &'static str,
    value: String,
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    CertificatePrototype::initialize_prototype_template(scope, prototype);
}
pub(super) fn install_static<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    CertificateStatics::initialize_template(scope, template);
}

enum AlgorithmInput<'s> {
    Object(v8::Local<'s, v8::Object>),
    Name(String),
}
impl<'s> webidl::WebIdlConverter<'s> for AlgorithmInput<'s> {
    type Options = ();
    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
            Ok(Self::Object(object))
        } else {
            webidl::convert::<webidl::DomString>(scope, value, context)
                .map(|value| Self::Name(value.0))
        }
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCPeerConnection.generateCertificate")]
struct GenerateArgs<'s> {
    #[webidl(required, converter = "raw")]
    algorithm: AlgorithmInput<'s>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCCertificateExpiration")]
struct Expiration {
    #[webidl(converter = "enforce_range_unsigned_long_long")]
    expires: Option<u64>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "Algorithm")]
struct Algorithm {
    #[webidl(required)]
    name: String,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "EcKeyGenParams")]
struct EcParams {
    #[webidl(inherit)]
    _base: Algorithm,
    #[webidl(required)]
    named_curve: String,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RsaKeyGenParams")]
struct RsaParams<'s> {
    #[webidl(inherit)]
    _base: Algorithm,
    #[webidl(required, converter = "enforce_range_unsigned_long")]
    modulus_length: u32,
    #[webidl(required)]
    public_exponent: v8::Local<'s, v8::Uint8Array>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RsaHashedKeyGenParams")]
struct RsaHashedParams<'s> {
    #[webidl(inherit)]
    base: RsaParams<'s>,
    #[webidl(required, converter = "raw")]
    hash: AlgorithmInput<'s>,
}

fn algorithm_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    input: AlgorithmInput<'s>,
) -> v8::Local<'s, v8::Object> {
    match input {
        AlgorithmInput::Object(object) => object,
        AlgorithmInput::Name(name) => {
            let object = crate::util::new_null_prototype_object(scope);
            let name = crate::util::v8_string(scope, &name).expect("algorithm name");
            assert_eq!(
                object.create_data_property(scope, v8str(scope, "name").into(), name.into()),
                Some(true)
            );
            object
        }
    }
}

fn normalized_params<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    input: AlgorithmInput<'s>,
) -> Option<(CertificateKeyAlgorithm, u64)> {
    let lifetime = if let AlgorithmInput::Object(object) = &input {
        dictionary::<Expiration>(scope, *object)?
            .expires
            .unwrap_or(DEFAULT_LIFETIME)
            .min(MAX_LIFETIME)
    } else {
        DEFAULT_LIFETIME
    };
    let object = algorithm_object(scope, input);
    let name = dictionary::<Algorithm>(scope, object)?.name;
    let algorithm = match name.to_ascii_lowercase().as_str() {
        "ecdsa" => {
            let params = dictionary::<EcParams>(scope, object)?;
            if params.named_curve != "P-256" {
                return not_supported(scope);
            }
            CertificateKeyAlgorithm::EcdsaP256
        }
        "rsassa-pkcs1-v1_5" => {
            let params = dictionary::<RsaHashedParams>(scope, object)?;
            let hash = algorithm_object(scope, params.hash);
            let name = dictionary::<Algorithm>(scope, hash)?.name;
            if !name.eq_ignore_ascii_case("SHA-256") {
                return not_supported(scope);
            }
            // Inspect the native view after dictionary conversion. Leading
            // zeroes are valid; no author-sized allocation is necessary.
            let exponent = params.base.public_exponent;
            let Some(backing) = exponent.get_backing_store() else {
                return not_supported(scope);
            };
            let Some(bytes) = backing
                .get(exponent.byte_offset()..exponent.byte_offset() + exponent.byte_length())
            else {
                return not_supported(scope);
            };
            let Some(public_exponent) = bytes.iter().try_fold(0_u32, |value, byte| {
                value.checked_mul(256)?.checked_add(u32::from(byte.get()))
            }) else {
                return not_supported(scope);
            };
            CertificateKeyAlgorithm::Rsa {
                modulus_bits: params.base.modulus_length,
                public_exponent,
            }
        }
        _ => return not_supported(scope),
    };
    if !algorithm.is_supported() {
        return not_supported(scope);
    }
    Some((algorithm, lifetime))
}

fn not_supported<T>(scope: &mut v8::PinScope<'_, '_>) -> Option<T> {
    webidl::throw_dom_exception(
        scope,
        "NotSupportedError",
        "The certificate algorithm is not supported.",
    );
    None
}

fn dictionary<'s, T: webidl::WebIdlDictionary<'s>>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<T> {
    match webidl::parse_dictionary_object::<T>(scope, object) {
        Ok(value) => Some(value),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn current_origin(scope: &mut v8::PinScope<'_, '_>) -> Option<WindowOriginKey> {
    let origin = context_host_ptr_from_global_bridge(scope).and_then(|host| {
        // SAFETY: the live callback bridge owns this host throughout the read.
        unsafe { &mut *host }.window_origin_key_for_current_realm(scope)
    });
    if origin.is_none() {
        webidl::throw_dom_exception(
            scope,
            "OperationError",
            "The certificate origin is unavailable.",
        );
    }
    origin
}

fn generate_certificate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<GenerateArgs>(scope, &args) else {
        return;
    };
    let Some((algorithm, lifetime)) = normalized_params(scope, parsed.algorithm) else {
        return;
    };
    let Some(origin) = current_origin(scope) else {
        return;
    };
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    rv.set(resolver.get_promise(scope).into());
    if let Some((handle, completion)) = register_crypto_resolver_task(scope, resolver) {
        handle.spawn_blocking(move || {
            let result =
                SelfSignedCertificate::generate(algorithm, Duration::from_millis(lifetime))
                    .map(|certificate| {
                        WebCryptoTaskResult::RtcCertificate(Box::new(CertificatePayload {
                            certificate,
                            origin,
                        }))
                    })
                    .map_err(|_| WebCryptoRejection::Operation);
            completion.send(result);
        });
    } else if tokio::runtime::Handle::try_current().is_err() {
        // Standalone ScriptVm tests have no task executor. Production realms
        // always use the captured owner queue and never block on key generation.
        match SelfSignedCertificate::generate(algorithm, Duration::from_millis(lifetime)) {
            Ok(certificate) => {
                if let Some(object) = from_payload(
                    scope,
                    CertificatePayload {
                        certificate,
                        origin,
                    },
                ) {
                    let _ = resolver.resolve(scope, object.into());
                } else {
                    crate::script_vm::webcrypto_tasks::reject_webcrypto_task(
                        scope,
                        resolver,
                        WebCryptoRejection::Operation,
                    );
                }
            }
            Err(_) => crate::script_vm::webcrypto_tasks::reject_webcrypto_task(
                scope,
                resolver,
                WebCryptoRejection::Operation,
            ),
        }
    }
}

pub(crate) fn from_payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: CertificatePayload,
) -> Option<v8::Local<'s, v8::Object>> {
    if !exposed_interfaces::is_realm_interface_exposed(scope, "RTCCertificate") {
        return None;
    }
    let object =
        exposed_interfaces::build_intrinsic_interface_instance(scope, "RTCCertificate").ok()?;
    let prototype =
        exposed_interfaces::ensure_intrinsic_interface_prototype(scope, "RTCCertificate").ok()?;
    if object.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    let store = scope.get_slot::<Store>().cloned().unwrap_or_else(|| {
        let store = Store::default();
        scope.set_slot(store.clone());
        store
    });
    let id = {
        let mut store = store.borrow_mut();
        store.next_id = store
            .next_id
            .checked_add(1)
            .expect("RTCCertificate identity exhausted");
        store.next_id
    };
    CertificateSlots::new(v8::BigInt::new_from_u64(scope, id))
        .initialize(scope, object)
        .ok()?;
    let weak_store = Rc::downgrade(&store);
    let weak = v8::Weak::with_finalizer(
        scope,
        object,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                store.borrow_mut().entries.remove(&id);
            }
        }),
    );
    store.borrow_mut().entries.insert(id, (weak, payload));
    Some(object)
}

pub(crate) fn payload_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<CertificatePayload> {
    if !web_api_interfaces::RTCCertificate::is_instance(scope, object) {
        return None;
    }
    let object = moli_webapi_declare::web_api_object_target(scope, object)?;
    let id = v8::Local::<v8::BigInt>::try_from(get_private_value(scope, object, ID)?)
        .ok()?
        .u64_value()
        .0;
    scope
        .get_slot::<Store>()?
        .borrow()
        .entries
        .get(&id)
        .map(|(_, payload)| payload.clone())
}

fn expires<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let payload = payload_from_object(scope, args.this()).expect("native RTCCertificate receiver");
    rv.set_double(payload.certificate.expires_millis() as f64);
}

fn get_fingerprints<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let payload = payload_from_object(scope, args.this()).expect("native RTCCertificate receiver");
    let fingerprint = Fingerprint {
        algorithm: "sha-256",
        value: payload.certificate.sha256_fingerprint().to_owned(),
    }
    .bind(scope)
    .expect("DTLS fingerprint");
    rv.set(v8::Array::new_with_elements(scope, &[fingerprint.into()]).into());
}

pub(super) fn validate_configuration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    certificates: &[v8::Local<'s, v8::Object>],
) -> Option<()> {
    if certificates.is_empty() {
        return Some(());
    }
    let origin = current_origin(scope)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    for certificate in certificates {
        let payload = payload_from_object(scope, *certificate).expect("converted RTCCertificate");
        if payload.origin != origin
            || Duration::from_millis(payload.certificate.expires_millis()) < now
        {
            webidl::throw_dom_exception(
                scope,
                "InvalidAccessError",
                "The certificate is expired or belongs to another origin.",
            );
            return None;
        }
    }
    Some(())
}

pub(super) fn initialize_connection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    config: v8::Local<'s, v8::Object>,
    explicit: &[v8::Local<'s, v8::Object>],
) -> Option<()> {
    let certificates = if explicit.is_empty() {
        let origin = current_origin(scope)?;
        let certificate = match SelfSignedCertificate::generate(
            CertificateKeyAlgorithm::EcdsaP256,
            Duration::from_millis(DEFAULT_LIFETIME),
        ) {
            Ok(value) => value,
            Err(_) => {
                webidl::throw_dom_exception(
                    scope,
                    "OperationError",
                    "Certificate generation failed.",
                );
                return None;
            }
        };
        vec![
            from_payload(
                scope,
                CertificatePayload {
                    certificate,
                    origin,
                },
            )?
            .into(),
        ]
    } else {
        explicit.iter().map(|object| (*object).into()).collect()
    };
    let certificates = v8::Array::new_with_elements(scope, &certificates);
    set_private_value(scope, config, CONNECTION_CERTIFICATES, certificates.into());
    Some(())
}

pub(super) fn preserve_connection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    old: v8::Local<'s, v8::Object>,
    next: v8::Local<'s, v8::Object>,
) {
    let certificates = get_private_value(scope, old, CONNECTION_CERTIFICATES)
        .expect("native connection certificates");
    set_private_value(scope, next, CONNECTION_CERTIFICATES, certificates);
}

pub(super) fn connection_fingerprints<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    config: v8::Local<'s, v8::Object>,
    sdp: &mut String,
) {
    let certificates = get_private_value(scope, config, CONNECTION_CERTIFICATES)
        .and_then(|value| v8::Local::try_from(value).ok())
        .expect("native connection certificate array");
    append_fingerprints(scope, certificates, sdp);
}

pub(super) fn append_fingerprints<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    certificates: v8::Local<'s, v8::Array>,
    sdp: &mut String,
) {
    for index in 0..certificates.length() {
        let certificate = certificates
            .get_index(scope, index)
            .and_then(|value| v8::Local::try_from(value).ok())
            .and_then(|object| payload_from_object(scope, object))
            .expect("private certificate snapshot");
        sdp.push_str("a=fingerprint:sha-256 ");
        sdp.push_str(
            &certificate
                .certificate
                .sha256_fingerprint()
                .to_ascii_uppercase(),
        );
        sdp.push_str("\r\n");
    }
}
