mod bindings;
mod error;
mod init;
mod input;

use super::headers::{
    build_headers_object_with_state, filter_headers_for_guard, headers_entries_from_init,
};
use super::*;
use crate::util::{get_private_value, set_private_value};

const STATIC_ROUTER_RACE_SLOT: &str = "__moliStaticRouterRaceRequest";

pub(crate) fn request_static_router_race_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> Option<crate::service_worker_runtime::ServiceWorkerEventId> {
    if !is_branded_request_object(scope, request) {
        return None;
    }
    let value = get_private_value(scope, request, STATIC_ROUTER_RACE_SLOT)?;
    let value = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (id, lossless) = value.u64_value();
    lossless.then(|| crate::service_worker_runtime::ServiceWorkerEventId::from_u64_for_worker(id))
}

pub(crate) fn set_request_static_router_race_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    event_id: crate::service_worker_runtime::ServiceWorkerEventId,
) {
    let value = v8::BigInt::new_from_u64(scope, event_id.as_u64());
    set_private_value(scope, request, STATIC_ROUTER_RACE_SLOT, value.into());
}

pub(crate) use self::bindings::{
    build_cached_request_object, build_service_worker_request_object, cached_request_from_native,
    request_constructor_callback,
};
pub(crate) use self::error::{FetchArgumentError, RequestUrlError};
pub(in crate::network_host) use self::init::request_credentials_mode_label;
pub(crate) use self::init::{FetchArgs, RequestInitValidation, convert_fetch_arguments};
pub(crate) use self::init::{
    parse_fetch_init, request_object_credentials_mode, validate_fetch_body,
};
pub(crate) use self::init::{parse_request_redirect_mode_label, request_redirect_mode_label};
pub(crate) use self::input::{
    RequestInfo, RequestInputUrl, mark_request_input_body_used_for_fetch,
    request_body_already_used_error, request_headers_guard_for_mode,
    try_resolve_request_constructor_url_for_base, try_resolve_request_constructor_url_for_child,
    validate_request_method_body,
};
pub(in crate::network_host) use self::input::{RequestMethodError, normalize_request_method};
