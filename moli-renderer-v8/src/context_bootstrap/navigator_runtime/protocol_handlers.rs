//! Protocol-handler validation. The browser currently declines registration
//! requests; no external application or handler navigation is installed.

use crate::{
    native_bridge::{
        element::parse_url_with_document_query_encoding,
        node_runtime_and_handle_from_object_or_detached,
    },
    util::get_private_value,
    web_api_interfaces, webidl,
};
use moli_webapi_declare::WebApiFunctionTemplate;

pub(super) const DOCUMENT_SLOT: &str = "__moliNavigatorDocument";
pub(super) const ORIGIN_SLOT: &str = "__moliNavigatorOrigin";

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Navigator, receiver)]
struct NavigatorProtocolHandlersDeclaration {
    #[webapi(method, enumerable, length = 2, callback = protocol_handler_callback)]
    register_protocol_handler: (),
    #[webapi(method, enumerable, length = 2, callback = protocol_handler_callback)]
    unregister_protocol_handler: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Navigator protocol handler")]
struct ProtocolHandlerArgs {
    #[webidl(required)]
    scheme: String,
    #[webidl(required, converter = "usv_string")]
    url: String,
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    NavigatorProtocolHandlersDeclaration::initialize_prototype_template(scope, prototype);
}

fn allowed_scheme(scheme: &str) -> bool {
    matches!(
        scheme,
        "bitcoin"
            | "ftp"
            | "ftps"
            | "geo"
            | "im"
            | "irc"
            | "ircs"
            | "magnet"
            | "mailto"
            | "matrix"
            | "mms"
            | "news"
            | "nntp"
            | "openpgp4fpr"
            | "sftp"
            | "sip"
            | "sms"
            | "smsto"
            | "ssh"
            | "tel"
            | "urn"
            | "webcal"
            | "wtai"
            | "xmpp"
    ) || scheme.strip_prefix("web+").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_lowercase())
    })
}

fn protocol_handler_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    // The generated binding checks the native Navigator brand before conversion.
    let Some(parsed) = webidl::parse_args::<ProtocolHandlerArgs>(scope, &args) else {
        return;
    };
    if !allowed_scheme(&parsed.scheme.to_ascii_lowercase()) {
        webidl::throw_dom_exception(scope, "SecurityError", "The protocol cannot be registered.");
        return;
    }
    if !parsed.url.contains("%s") {
        webidl::throw_dom_exception(scope, "SyntaxError", "The handler URL must contain %s.");
        return;
    }
    let Some(document) = get_private_value(scope, args.this(), DOCUMENT_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        webidl::throw_dom_exception(
            scope,
            "SecurityError",
            "The Navigator has no associated Document.",
        );
        return;
    };
    let Ok((host_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, document)
    else {
        webidl::throw_dom_exception(
            scope,
            "SecurityError",
            "The Navigator Document is unavailable.",
        );
        return;
    };
    // Retain the receiver's Document and origin, including when its browsing
    // context navigates. Public baseURI/origin getters and the callee realm do
    // not supply the relevant environment settings.
    let host = unsafe { &*host_ptr };
    let base = host.document_base_url_for_handle(handle);
    let Ok(url) = parse_url_with_document_query_encoding(host, handle, &base, &parsed.url) else {
        webidl::throw_dom_exception(scope, "SyntaxError", "The handler URL cannot be parsed.");
        return;
    };
    let origin = get_private_value(scope, args.this(), ORIGIN_SLOT)
        .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
        .map(|value| value.to_rust_string_lossy(scope));
    if !matches!(url.scheme(), "http" | "https")
        || origin.as_deref() != Some(url.origin().ascii_serialization().as_str())
    {
        webidl::throw_dom_exception(
            scope,
            "SecurityError",
            "The handler URL must have the Navigator's origin.",
        );
        return;
    }
    // HTML permits the user agent to cancel a registration request. With no
    // registration UI, there are no accepted handlers for unregister to remove.
    rv.set_undefined();
}
