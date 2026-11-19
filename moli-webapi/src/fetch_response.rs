use crate::headers::{build_headers_object, headers_entries_from_init};
use crate::interfaces as web_api_interfaces;
use moli_fetch_types::{FetchResponseMetadata, FetchResponseType};
use moli_v8_util::{get_private_value, set_private_value, v8_string};
use moli_webapi_declare::WebApiObject;
use moli_webidl as webidl;

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
pub struct ResponseCloneShellDeclaration<'scope> {
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

pub const RESPONSE_METADATA_SLOT: &str = "__moliResponseMetadata";
const TYPE_SLOT: &str = "__moliResponseMetadataType";
const URL_LIST_SLOT: &str = "__moliResponseMetadataUrls";
const TIMING_SLOT: &str = "__moliResponseMetadataTiming";
const REDIRECTED_SLOT: &str = "__moliResponseMetadataRedirected";

#[derive(WebApiObject)]
#[webapi(plain)]
struct ResponseMetadataDeclaration<'scope> {
    #[webapi(slot = TYPE_SLOT)]
    response_type: &'static str,
    #[webapi(slot = URL_LIST_SLOT)]
    url_list: v8::Local<'scope, v8::Array>,
    #[webapi(slot = TIMING_SLOT)]
    timing_allow_passed: bool,
    #[webapi(slot = REDIRECTED_SLOT)]
    redirected: bool,
}

/// Install one immutable metadata snapshot. Clone can share it; native
/// consumers always materialize the complete typed value through this module.
pub fn set_response_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
    metadata: &FetchResponseMetadata,
) {
    // Defining own elements must not invoke Array.prototype setters: these
    // URLs can be private even when a page owns the surrounding Response.
    let values: Vec<v8::Local<'s, v8::Value>> = metadata
        .url_list
        .iter()
        .map(|url| {
            v8_string(scope, url.as_str())
                .expect("Response URL should fit a V8 string")
                .into()
        })
        .collect();
    let urls = v8::Array::new_with_elements(scope, &values);
    let value = ResponseMetadataDeclaration::new(
        metadata.response_type.as_str(),
        urls,
        metadata.timing_allow_passed,
        metadata.redirected,
    )
    .bind(scope)
    .expect("Response metadata declaration should bind");
    set_private_value(scope, response, RESPONSE_METADATA_SLOT, value.into());
}

pub fn response_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
) -> Option<FetchResponseMetadata> {
    let metadata = get_private_value(scope, response, RESPONSE_METADATA_SLOT)?;
    let metadata = v8::Local::<v8::Object>::try_from(metadata).ok()?;
    let response_type = get_private_value(scope, metadata, TYPE_SLOT)?
        .to_rust_string_lossy(scope)
        .parse()
        .ok()?;
    let urls = get_private_value(scope, metadata, URL_LIST_SLOT)?;
    let urls = v8::Local::<v8::Array>::try_from(urls).ok()?;
    let url_list = (0..urls.length())
        .map(|index| {
            urls.get_index(scope, index)?
                .to_rust_string_lossy(scope)
                .parse()
                .ok()
        })
        .collect::<Option<Vec<_>>>()?;
    Some(FetchResponseMetadata {
        response_type,
        url_list,
        timing_allow_passed: get_private_value(scope, metadata, TIMING_SLOT)?.boolean_value(scope),
        redirected: get_private_value(scope, metadata, REDIRECTED_SLOT)?.boolean_value(scope),
    })
}

/// Public accessors project the same private snapshot without reparsing URLs
/// or constructing another Rust copy of the entire response.
pub fn response_metadata_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
    attribute: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let metadata = get_private_value(scope, response, RESPONSE_METADATA_SLOT)?;
    let metadata = v8::Local::<v8::Object>::try_from(metadata).ok()?;
    let kind = get_private_value(scope, metadata, TYPE_SLOT)?;
    if attribute == RESPONSE_TYPE_SLOT {
        return Some(kind);
    }
    let kind: FetchResponseType = kind.to_rust_string_lossy(scope).parse().ok()?;
    if attribute == RESPONSE_REDIRECTED_SLOT {
        return if kind.is_readable() {
            get_private_value(scope, metadata, REDIRECTED_SLOT)
        } else {
            Some(v8::Boolean::new(scope, false).into())
        };
    }
    if attribute == RESPONSE_URL_SLOT {
        let urls = get_private_value(scope, metadata, URL_LIST_SLOT)?;
        let urls = v8::Local::<v8::Array>::try_from(urls).ok()?;
        let url = if kind == FetchResponseType::Opaque || urls.length() == 0 {
            String::new()
        } else if let Some(url) = get_private_value(scope, response, RESPONSE_URL_SLOT) {
            // Navigation preload's manual redirect surface keeps its request URL.
            url.to_rust_string_lossy(scope)
        } else {
            urls.get_index(scope, urls.length() - 1)?
                .to_rust_string_lossy(scope)
        };
        let url = url.split_once('#').map_or(url.as_str(), |(url, _)| url);
        return v8_string(scope, url).map(Into::into);
    }
    None
}
