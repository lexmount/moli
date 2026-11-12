use super::*;
use crate::network_host::fetch_surface::{response_slot_value, set_response_slot_value};
use crate::util::get_private_value;
use moli_fetch::RequestMode;
use moli_fetch_types::{FetchResponseMetadata, FetchResponseType};
use moli_url::WebOrigin;
use moli_webapi_declare::WebApiObject;

pub(in crate::network_host) const RESPONSE_METADATA_SLOT: &str = "__moliResponseMetadata";
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
pub(crate) fn set_response_metadata<'s>(
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
    set_response_slot_value(scope, response, RESPONSE_METADATA_SLOT, value.into());
}

pub(crate) fn response_metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
) -> Option<FetchResponseMetadata> {
    let metadata = response_slot_value(scope, response, RESPONSE_METADATA_SLOT)?;
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
pub(in crate::network_host) fn response_metadata_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    response: v8::Local<'s, v8::Object>,
    attribute: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let metadata = response_slot_value(scope, response, RESPONSE_METADATA_SLOT)?;
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
        } else if let Some(url) = response_slot_value(scope, response, RESPONSE_URL_SLOT) {
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

pub(crate) fn fetch_response_metadata(
    origin: &WebOrigin,
    head: &moli_fetch::ResponseHead,
    mode: RequestMode,
    delivery: Option<&crate::types::AsyncSubresourceFetchResponseFilter>,
) -> FetchResponseMetadata {
    if let Some(metadata) = delivery.and_then(|delivery| delivery.service_worker_metadata()) {
        let mut metadata = metadata.clone();
        if metadata.response_type == FetchResponseType::Default {
            metadata.response_type = FetchResponseType::Basic;
        }
        if metadata.url_list.is_empty() {
            metadata.url_list = head.url_list().to_vec();
        }
        metadata.redirected |= head.redirected;
        return metadata;
    }
    let mut metadata = head.fetch_metadata(origin, mode);
    if let Some(delivery) = delivery {
        metadata.response_type = delivery.response_type();
    }
    metadata
}
