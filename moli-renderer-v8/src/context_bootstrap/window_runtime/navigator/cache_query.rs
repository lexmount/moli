//! Shared Cache WebIDL conversion. Conversion precedes request construction;
//! failures reject the binding's Promise with the original exception value.
use super::*;
use moli_storage_service::StorageBucketCacheName;

pub(super) enum RequestInfo<'s> {
    Request(v8::Local<'s, v8::Object>),
    Url(v8::Local<'s, v8::String>),
}

impl<'s> RequestInfo<'s> {
    pub(super) fn into_value(self) -> v8::Local<'s, v8::Value> {
        match self {
            Self::Request(value) => value.into(),
            Self::Url(value) => value.into(),
        }
    }
}

impl<'s> webidl::WebIdlConverter<'s> for RequestInfo<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        if let Ok(object) = webidl::convert_with_options::<webidl::InterfaceObject>(
            scope,
            value,
            context,
            &webidl::InterfaceOptions {
                name: web_api_interfaces::Request::NAME,
                brand_check: web_api_interfaces::Request::is_instance,
            },
        ) {
            let target = moli_webapi_declare::web_api_object_target(scope, object.0)
                .expect("branded Request has native identity");
            return Ok(Self::Request(target));
        }
        let text = webidl::convert::<webidl::UsvString>(scope, value, context)?.0;
        Ok(Self::Url(v8_string(scope, &text).ok_or_else(|| {
            webidl::WebIdlError::pending_exception(context)
        })?))
    }
}

#[derive(Debug, Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "CacheQueryOptions")]
pub(super) struct CacheQueryOptions {
    #[webidl(default = false)]
    pub(super) ignore_method: bool,
    #[webidl(default = false)]
    pub(super) ignore_search: bool,
    #[webidl(default = false)]
    pub(super) ignore_vary: bool,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MultiCacheQueryOptions")]
struct CacheNameOption {
    #[webidl(converter = "dom_string16")]
    cache_name: Option<Vec<u16>>,
}

struct MultiCacheQueryOptions {
    base: CacheQueryOptions,
    cache_name: Option<StorageBucketCacheName>,
}

impl<'s> webidl::WebIdlDictionary<'s> for MultiCacheQueryOptions {
    fn parse_dictionary(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Result<Self, webidl::WebIdlError> {
        // WebIDL visits the ancestor dictionary before its derived members.
        let base =
            <CacheQueryOptions as webidl::WebIdlDictionary>::parse_dictionary(scope, object)?;
        let cache_name =
            <CacheNameOption as webidl::WebIdlDictionary>::parse_dictionary(scope, object)?
                .cache_name
                .map(StorageBucketCacheName::from_utf16);
        Ok(Self { base, cache_name })
    }
}

pub(super) struct QueryArguments {
    pub(super) request: Option<CacheRequestInfo>,
    pub(super) options: CacheQueryOptions,
    pub(super) cache_name: Option<StorageBucketCacheName>,
}

pub(super) fn rejecting_conversion<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    convert: impl FnOnce(&mut v8::PinScope<'s, '_>) -> Result<T, webidl::WebIdlError>,
) -> Option<T> {
    v8::tc_scope!(let scope, scope);
    match convert(scope) {
        Ok(value) => Some(value),
        Err(error) => {
            webidl::throw_error(scope, &error);
            let exception = scope
                .exception()
                .unwrap_or_else(|| v8::undefined(scope).into());
            scope.reset();
            let _ = resolver.reject(scope, exception);
            None
        }
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Cache")]
struct QueryArgs<'s> {
    #[webidl(required, with = required_request)]
    request: RequestInfo<'s>,
    #[webidl(dictionary)]
    options: CacheQueryOptions,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Cache")]
struct OptionalQueryArgs<'s> {
    #[webidl(with = optional_request)]
    request: Option<RequestInfo<'s>>,
    #[webidl(dictionary)]
    options: CacheQueryOptions,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CacheStorage.match")]
struct StorageQueryArgs<'s> {
    #[webidl(required, with = required_request)]
    request: RequestInfo<'s>,
    #[webidl(dictionary)]
    options: MultiCacheQueryOptions,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Cache.put")]
pub(super) struct PutArgs<'s> {
    #[webidl(required, with = required_request)]
    pub(super) request: RequestInfo<'s>,
    #[webidl(required, interface = web_api_interfaces::Response)]
    pub(super) response: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CacheStorage")]
struct NameArgs {
    #[webidl(required, converter = "dom_string16")]
    name: Vec<u16>,
}

fn required_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<RequestInfo<'s>, webidl::WebIdlError> {
    webidl::argument(scope, args, index, webidl::Context::argument("Cache", 1))
}

fn optional_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<Option<RequestInfo<'s>>, webidl::WebIdlError> {
    if args.length() <= index || args.get(index).is_undefined() {
        return Ok(None);
    }
    required_request(scope, args, index).map(Some)
}

pub(super) fn materialize_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    input: RequestInfo<'s>,
) -> Result<CacheRequestInfo, webidl::WebIdlError> {
    let context = webidl::Context::argument("Cache", 1);
    let request = match input {
        RequestInfo::Request(request) => request,
        RequestInfo::Url(url) => {
            let constructor =
                crate::context_bootstrap::ensure_intrinsic_interface_constructor(scope, "Request")
                    .expect("Cache Request intrinsic should materialize");
            crate::script_execution::construct(scope, constructor, &[url.into()])
                .ok_or_else(|| webidl::WebIdlError::pending_exception(context))?
        }
    };
    Ok(CacheRequestInfo {
        url: crate::network_host::request_slot_string(
            scope,
            request,
            crate::network_host::REQUEST_URL_SLOT,
        )
        .expect("native Request has a URL"),
        method: crate::network_host::request_method(scope, request),
        headers: crate::network_host::request_headers_entries(scope, request),
    })
}

pub(super) enum QueryKind {
    Cache,
    OptionalRequest,
    Storage,
}

pub(super) fn query_arguments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    kind: QueryKind,
) -> Option<QueryArguments> {
    rejecting_conversion(scope, resolver, |scope| {
        let (input, options, cache_name) = match kind {
            QueryKind::Storage => {
                let parsed = webidl::try_parse_args::<StorageQueryArgs>(scope, args)?;
                (
                    Some(parsed.request),
                    parsed.options.base,
                    parsed.options.cache_name,
                )
            }
            QueryKind::OptionalRequest => {
                let parsed = webidl::try_parse_args::<OptionalQueryArgs>(scope, args)?;
                (parsed.request, parsed.options, None)
            }
            QueryKind::Cache => {
                let parsed = webidl::try_parse_args::<QueryArgs>(scope, args)?;
                (Some(parsed.request), parsed.options, None)
            }
        };
        let request = input
            .map(|input| materialize_request(scope, input))
            .transpose()?;
        Ok(QueryArguments {
            request,
            options,
            cache_name,
        })
    })
}

pub(super) fn name_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
) -> Option<StorageBucketCacheName> {
    rejecting_conversion(scope, resolver, |scope| {
        webidl::try_parse_args::<NameArgs>(scope, args)
            .map(|parsed| StorageBucketCacheName::from_utf16(parsed.name))
    })
}

pub(super) fn name_from_private_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<StorageBucketCacheName> {
    let text = v8::Local::<v8::String>::try_from(get_private_value(scope, object, slot)?).ok()?;
    Some(StorageBucketCacheName::from_utf16(
        crate::util::v8_string_to_u16_string(scope, text).into_vec(),
    ))
}
