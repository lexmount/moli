use moli_webapi_declare::WebApiValue;

use crate::{native_bridge::throw_dom_exception, webidl};

use super::super::{
    base64url,
    value::{Dictionary, Text},
};
use super::schema::{
    CreationJson, DescriptorJson, ExtensionsJson, LargeBlobJson, Parameters, PrfJson,
    PrfValuesJson, RelyingParty, RequestJson, Selection, UserJson,
};

fn member<'s, T: WebApiValue<'s> + ?Sized>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: &T,
) -> Option<()> {
    let name = v8::String::new(scope, name)?;
    let value = value.to_v8_value(scope)?;
    object
        .create_data_property(scope, name.into(), value)?
        .then_some(())
}

fn optional<'s, T: WebApiValue<'s>>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: &Option<T>,
) -> Option<()> {
    if let Some(value) = value {
        member(scope, object, name, value)?;
    }
    Some(())
}

fn invalid_base64url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::ArrayBuffer>> {
    throw_dom_exception(scope, "EncodingError", 0, "Invalid base64url data.");
    None
}

fn buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    text: &Text,
) -> Option<v8::Local<'s, v8::ArrayBuffer>> {
    let Some(bytes) = base64url::decode(text) else {
        return invalid_base64url(scope);
    };
    let backing_store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    Some(v8::ArrayBuffer::with_backing_store(scope, &backing_store))
}

fn optional_buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: &Option<Text>,
) -> Option<()> {
    if let Some(value) = value {
        let decoded = buffer(scope, value)?;
        member(scope, object, name, &decoded)?;
    }
    Some(())
}

fn sequence<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    values: &webidl::Sequence<Dictionary<T>>,
    encode: fn(&mut v8::PinScope<'s, '_>, &T) -> Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Value>> {
    let mut result = Vec::with_capacity(values.0.len());
    for value in &values.0 {
        result.push(encode(scope, &value.0)?);
    }
    result.to_v8_value(scope)
}

pub(super) fn creation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: &CreationJson,
) -> Option<v8::Local<'s, v8::Object>> {
    let result = v8::Object::new(scope);
    member(scope, result, "attestation", &options.attestation)?;
    member(
        scope,
        result,
        "attestationFormats",
        &options.attestation_formats.0,
    )?;
    if let Some(value) = &options.authenticator_selection {
        let value = selection(scope, &value.0)?;
        member(scope, result, "authenticatorSelection", &value)?;
    }
    let challenge = buffer(scope, &options.challenge)?;
    member(scope, result, "challenge", &challenge)?;
    let excluded = sequence(scope, &options.exclude_credentials, descriptor)?;
    member(scope, result, "excludeCredentials", &excluded)?;
    if let Some(value) = &options.extensions {
        let value = extensions(scope, &value.0)?;
        member(scope, result, "extensions", &value)?;
    }
    member(scope, result, "hints", &options.hints.0)?;
    let parameters = sequence(scope, &options.pub_key_cred_params, parameters)?;
    member(scope, result, "pubKeyCredParams", &parameters)?;
    let rp = relying_party(scope, &options.rp.0)?;
    member(scope, result, "rp", &rp)?;
    optional(scope, result, "timeout", &options.timeout)?;
    let user = user(scope, &options.user.0)?;
    member(scope, result, "user", &user)?;
    Some(result)
}

pub(super) fn request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: &RequestJson,
) -> Option<v8::Local<'s, v8::Object>> {
    let result = v8::Object::new(scope);
    let allowed = sequence(scope, &options.allow_credentials, descriptor)?;
    member(scope, result, "allowCredentials", &allowed)?;
    let challenge = buffer(scope, &options.challenge)?;
    member(scope, result, "challenge", &challenge)?;
    if let Some(value) = &options.extensions {
        let value = extensions(scope, &value.0)?;
        member(scope, result, "extensions", &value)?;
    }
    member(scope, result, "hints", &options.hints.0)?;
    optional(scope, result, "rpId", &options.rp_id)?;
    optional(scope, result, "timeout", &options.timeout)?;
    member(
        scope,
        result,
        "userVerification",
        &options.user_verification,
    )?;
    Some(result)
}

fn relying_party<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &RelyingParty,
) -> Option<v8::Local<'s, v8::Object>> {
    let result = v8::Object::new(scope);
    member(scope, result, "name", &value.base.name)?;
    optional(scope, result, "id", &value.id)?;
    Some(result)
}

fn user<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &UserJson,
) -> Option<v8::Local<'s, v8::Object>> {
    let result = v8::Object::new(scope);
    // The output is PublicKeyCredentialUserEntity, whose name is inherited.
    member(scope, result, "name", &value.name)?;
    member(scope, result, "displayName", &value.display_name)?;
    let id = buffer(scope, &value.id)?;
    member(scope, result, "id", &id)?;
    Some(result)
}

fn parameters<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &Parameters,
) -> Option<v8::Local<'s, v8::Object>> {
    let result = v8::Object::new(scope);
    member(scope, result, "alg", &value.alg)?;
    member(scope, result, "type", &value.credential_type)?;
    Some(result)
}

fn descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &DescriptorJson,
) -> Option<v8::Local<'s, v8::Object>> {
    let result = v8::Object::new(scope);
    let id = buffer(scope, &value.id)?;
    member(scope, result, "id", &id)?;
    if let Some(transports) = &value.transports {
        member(scope, result, "transports", &transports.0)?;
    }
    member(scope, result, "type", &value.credential_type)?;
    Some(result)
}

fn selection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &Selection,
) -> Option<v8::Local<'s, v8::Object>> {
    let result = v8::Object::new(scope);
    optional(
        scope,
        result,
        "authenticatorAttachment",
        &value.authenticator_attachment,
    )?;
    member(
        scope,
        result,
        "requireResidentKey",
        &value.require_resident_key,
    )?;
    optional(scope, result, "residentKey", &value.resident_key)?;
    member(scope, result, "userVerification", &value.user_verification)?;
    Some(result)
}

fn extensions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &ExtensionsJson,
) -> Option<v8::Local<'s, v8::Object>> {
    let result = v8::Object::new(scope);
    optional(scope, result, "appid", &value.appid)?;
    optional(scope, result, "appidExclude", &value.appid_exclude)?;
    optional_buffer(scope, result, "credBlob", &value.cred_blob)?;
    optional(scope, result, "credProps", &value.cred_props)?;
    optional(
        scope,
        result,
        "credentialProtectionPolicy",
        &value.credential_protection_policy,
    )?;
    member(
        scope,
        result,
        "enforceCredentialProtectionPolicy",
        &value.enforce_credential_protection_policy,
    )?;
    optional(scope, result, "getCredBlob", &value.get_cred_blob)?;
    optional(scope, result, "hmacCreateSecret", &value.hmac_create_secret)?;
    if let Some(value) = &value.large_blob {
        let value = large_blob(scope, &value.0)?;
        member(scope, result, "largeBlob", &value)?;
    }
    optional(scope, result, "minPinLength", &value.min_pin_length)?;
    if let Some(value) = &value.prf {
        let value = prf(scope, &value.0)?;
        member(scope, result, "prf", &value)?;
    }
    optional(
        scope,
        result,
        "remoteClientDataJSON",
        &value.remote_client_data_json,
    )?;
    Some(result)
}

fn large_blob<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &LargeBlobJson,
) -> Option<v8::Local<'s, v8::Object>> {
    let result = v8::Object::new(scope);
    optional(scope, result, "read", &value.read)?;
    optional(scope, result, "support", &value.support)?;
    optional_buffer(scope, result, "write", &value.write)?;
    Some(result)
}

fn prf<'s>(scope: &mut v8::PinScope<'s, '_>, value: &PrfJson) -> Option<v8::Local<'s, v8::Object>> {
    let result = v8::Object::new(scope);
    if let Some(value) = &value.eval {
        let value = prf_values(scope, &value.0)?;
        member(scope, result, "eval", &value)?;
    }
    if let Some(record) = &value.eval_by_credential {
        let entries = v8::Object::new(scope);
        for (key, value) in &record.0 {
            let key = v8::Local::<v8::Name>::try_from(key.to_v8_value(scope)?).ok()?;
            let value = prf_values(scope, &value.0)?;
            if entries.create_data_property(scope, key, value.into()) != Some(true) {
                return None;
            }
        }
        member(scope, result, "evalByCredential", &entries)?;
    }
    Some(result)
}

fn prf_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &PrfValuesJson,
) -> Option<v8::Local<'s, v8::Object>> {
    let result = v8::Object::new(scope);
    let first = buffer(scope, &value.first)?;
    member(scope, result, "first", &first)?;
    optional_buffer(scope, result, "second", &value.second)?;
    Some(result)
}
