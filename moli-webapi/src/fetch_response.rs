use crate::headers::{build_headers_object, clone_headers_object, headers_entries_from_init};
use crate::interfaces as web_api_interfaces;
use moli_v8_util::{get_private_object, get_private_value, set_private_value};
use moli_webapi_declare::WebApiObject;
use moli_webidl as webidl;

mod metadata;
pub use metadata::{response_metadata, set_response_metadata};

pub const RESPONSE_TYPE_SLOT: &str = "__lmResponseType";
pub const RESPONSE_URL_SLOT: &str = "__lmResponseUrl";
pub const RESPONSE_INTERNAL_STATUS_SLOT: &str = "__lmResponseInternalStatus";
pub const RESPONSE_INTERNAL_STATUS_TEXT_SLOT: &str = "__lmResponseInternalStatusText";
pub const RESPONSE_INTERNAL_HEADERS_SLOT: &str = "__lmResponseInternalHeadersObject";
pub const RESPONSE_REDIRECTED_SLOT: &str = "__lmResponseRedirected";
pub const RESPONSE_STATUS_SLOT: &str = "__lmResponseStatus";
pub const RESPONSE_OK_SLOT: &str = "__lmResponseOk";
pub const RESPONSE_STATUS_TEXT_SLOT: &str = "__lmResponseStatusText";
pub const RESPONSE_HEADERS_SLOT: &str = "__lmResponseHeadersObject";
pub const RESPONSE_BODY_SLOT: &str = "__lmResponseBody";
pub const RESPONSE_BODY_USED_SLOT: &str = "__lmResponseBodyUsed";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Response)]
struct ResponseCloneShellDeclaration<'scope> {
    #[webapi(slot = RESPONSE_BODY_USED_SLOT, init = false)]
    body_used: (),
    #[webapi(slot = RESPONSE_BODY_SLOT)]
    body: Option<v8::Local<'scope, v8::Value>>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Response)]
pub struct ResponseInstanceDeclaration {
    #[webapi(slot = RESPONSE_STATUS_SLOT)]
    status: f64,
    #[webapi(slot = RESPONSE_STATUS_TEXT_SLOT)]
    status_text: String,
    #[webapi(slot = RESPONSE_OK_SLOT)]
    ok: bool,
    #[webapi(slot = RESPONSE_BODY_USED_SLOT, init = false)]
    body_used: (),
}

#[derive(WebApiObject)]
#[webapi(plain, prototype = "Response")]
pub struct FetchResponseHeadDeclaration {
    #[webapi(slot = RESPONSE_STATUS_SLOT)]
    status: f64,
    #[webapi(slot = RESPONSE_OK_SLOT)]
    ok: bool,
}

#[derive(WebApiObject)]
#[webapi(plain)]
pub struct FetchResponseBodyDeclaration<'scope> {
    #[webapi(slot = RESPONSE_STATUS_TEXT_SLOT)]
    status_text: String,
    #[webapi(slot = RESPONSE_BODY_SLOT)]
    body: v8::Local<'scope, v8::Value>,
    #[webapi(slot = RESPONSE_BODY_USED_SLOT, init = false)]
    body_used: (),
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct ResponseInitObjectDeclaration<'scope> {
    status: Option<f64>,
    status_text: Option<String>,
    headers: Option<v8::Local<'scope, v8::Value>>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ResponseInit")]
pub struct ParsedResponseInit {
    #[webidl(default = 200)]
    pub status: u16,
    #[webidl(converter = "byte_string", default = "")]
    pub status_text: String,
    #[webidl(with = response_init_headers_member)]
    pub headers: Vec<(String, String)>,
}

pub fn parse_response_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init_arg: v8::Local<'s, v8::Value>,
) -> Result<ParsedResponseInit, webidl::WebIdlError> {
    webidl::parse_dictionary::<ParsedResponseInit>(
        scope,
        init_arg,
        webidl::Context::argument("Response", 2),
    )
    .map(Option::unwrap_or_default)
}

fn response_init_headers_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    _key: &str,
) -> Result<Vec<(String, String)>, webidl::WebIdlError> {
    let context = webidl::Context::member("ResponseInit", "headers");
    let Some(headers) = webidl::property_result(scope, object, "headers", context)? else {
        return Ok(Vec::new());
    };
    if headers.is_null_or_undefined() {
        return Ok(Vec::new());
    }
    headers_entries_from_init(scope, headers)
}

impl Default for ParsedResponseInit {
    fn default() -> Self {
        Self {
            status: 200,
            status_text: String::new(),
            headers: Vec::new(),
        }
    }
}

pub fn empty_response_init_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    ResponseInitObjectDeclaration::new(None, None, None)
        .bind(scope)
        .expect("Response error init declaration should bind")
}

pub fn build_response_init_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mut parsed: ParsedResponseInit,
    extra_headers: &[(&str, &str)],
) -> v8::Local<'s, v8::Object> {
    for (name, value) in extra_headers {
        if !parsed
            .headers
            .iter()
            .any(|(entry_name, _): &(String, String)| entry_name.eq_ignore_ascii_case(name))
        {
            parsed
                .headers
                .push((name.to_ascii_lowercase(), (*value).to_owned()));
        }
    }

    ResponseInitObjectDeclaration::new(
        Some(parsed.status as f64),
        (!parsed.status_text.is_empty()).then_some(parsed.status_text),
        if parsed.headers.is_empty() {
            None
        } else {
            Some(build_headers_object(scope, &parsed.headers).into())
        },
    )
    .bind(scope)
    .expect("Response init declaration should bind")
}

/// Project the Response state that does not require inspection of its live body.
pub fn response_value_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
    attribute: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    match attribute {
        RESPONSE_TYPE_SLOT | RESPONSE_URL_SLOT | RESPONSE_REDIRECTED_SLOT => {
            metadata::response_metadata_attribute(scope, response, attribute)
        }
        _ => get_private_value(scope, response, attribute),
    }
}

/// Clone response values in the response realm. The embedder installs any
/// cloned or teed body before exposing the resulting object.
pub fn clone_response_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let clone = ResponseCloneShellDeclaration::new(Some(v8::null(scope).into()))
        .bind(scope)
        .expect("Response clone shell declaration should bind");
    for slot in [
        RESPONSE_STATUS_SLOT,
        RESPONSE_STATUS_TEXT_SLOT,
        RESPONSE_OK_SLOT,
        RESPONSE_URL_SLOT,
        RESPONSE_INTERNAL_STATUS_SLOT,
        RESPONSE_INTERNAL_STATUS_TEXT_SLOT,
        metadata::RESPONSE_METADATA_SLOT,
    ] {
        if let Some(value) = get_private_value(scope, response, slot) {
            set_private_value(scope, clone, slot, value);
        }
    }
    let headers = get_private_object(scope, response, RESPONSE_HEADERS_SLOT);
    let cloned_headers = headers.map(|headers| clone_headers_object(scope, headers));
    if let Some(cloned_headers) = cloned_headers {
        set_private_value(scope, clone, RESPONSE_HEADERS_SLOT, cloned_headers.into());
    }
    if let Some(internal_headers) =
        get_private_object(scope, response, RESPONSE_INTERNAL_HEADERS_SLOT)
    {
        let cloned_internal = if Some(internal_headers) == headers {
            cloned_headers.expect("public response headers were cloned")
        } else {
            clone_headers_object(scope, internal_headers)
        };
        set_private_value(
            scope,
            clone,
            RESPONSE_INTERNAL_HEADERS_SLOT,
            cloned_internal.into(),
        );
    }
    clone
}
