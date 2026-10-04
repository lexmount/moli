//! WebAuthn JSON option conversion is independent of authenticator availability.
//!
//! Finish WebIDL conversion (including nested getters and iterable/record
//! conversion) before decoding any base64url values. Conversion errors retain
//! their TypeError/original JavaScript exception; decoding errors are EncodingError.

mod encode;
mod schema;

use crate::webidl;

use super::value::Dictionary;
use schema::{CreationJson, RequestJson};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PublicKeyCredential.parseCreationOptionsFromJSON")]
struct CreationArgs {
    #[webidl(required, converter = "raw")]
    options: Dictionary<CreationJson>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PublicKeyCredential.parseRequestOptionsFromJSON")]
struct RequestArgs {
    #[webidl(required, converter = "raw")]
    options: Dictionary<RequestJson>,
}

pub(super) fn parse_creation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<CreationArgs>(scope, &args) else {
        return;
    };
    if let Some(result) = encode::creation(scope, &parsed.options.0) {
        rv.set(result.into());
    }
}

pub(super) fn parse_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<RequestArgs>(scope, &args) else {
        return;
    };
    if let Some(result) = encode::request(scope, &parsed.options.0) {
        rv.set(result.into());
    }
}
