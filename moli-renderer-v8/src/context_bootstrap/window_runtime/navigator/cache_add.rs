//! Native Cache fetch batches. V8 owns promise/callback edges; the batch's
//! native state contains only materialized data and never roots JavaScript.

use super::*;
use crate::network_host::{MaterializedResponseBody, MaterializedResponseHead};
use moli_storage_service::StorageBucketCachePut;
use std::cell::RefCell;

const STATE: &str = "__moliCacheAddState";
const RESOLVER: &str = "__moliCacheAddResolver";
const CACHE: &str = "__moliCacheAddCache";
const CANCEL: &str = "__moliCacheAddCancel";
const BATCH: &str = "__moliCacheAddBatch";
const INDEX: &str = "__moliCacheAddIndex";

#[derive(WebApiObject)]
#[webapi(plain)]
struct BatchDeclaration<'s> {
    #[webapi(slot = STATE)]
    state: v8::Local<'s, v8::External>,
    #[webapi(slot = RESOLVER)]
    resolver: v8::Local<'s, v8::PromiseResolver>,
    #[webapi(slot = CACHE)]
    cache: v8::Local<'s, v8::Object>,
    #[webapi(slot = CANCEL)]
    cancel: v8::Local<'s, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct ReactionDeclaration<'s> {
    #[webapi(slot = BATCH)]
    batch: v8::Local<'s, v8::Object>,
    #[webapi(slot = INDEX)]
    index: u32,
}

struct Entry {
    request: CacheRequestInfo,
    head: Option<MaterializedResponseHead>,
    response: Option<StorageBucketCachedResponse>,
}

struct BatchState {
    handle: StorageBucketCacheHandle,
    entries: Vec<Entry>,
    remaining: usize,
    settled: bool,
}

struct RequestInfo<'s>(v8::Local<'s, v8::Value>);

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Cache.add")]
struct AddArgs<'s> {
    #[webidl(required, with = request_info_arg)]
    request: RequestInfo<'s>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Cache.addAll")]
struct AddAllArgs<'s> {
    #[webidl(required, with = request_info_sequence_arg)]
    requests: webidl::Sequence<RequestInfo<'s>>,
}

fn request_info_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<RequestInfo<'s>, webidl::WebIdlError> {
    webidl::argument(
        scope,
        args,
        index,
        webidl::Context::argument("Cache.add", 1),
    )
}

fn request_info_sequence_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<webidl::Sequence<RequestInfo<'s>>, webidl::WebIdlError> {
    webidl::argument(
        scope,
        args,
        index,
        webidl::Context::argument("Cache.addAll", 1),
    )
}

impl<'s> webidl::WebIdlConverter<'s> for RequestInfo<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
            && web_api_interfaces::Request::is_instance(scope, object)
        {
            let target = moli_webapi_declare::web_api_object_target(scope, object)
                .expect("branded Request has native identity");
            return Ok(Self(target.into()));
        }
        let text = webidl::convert::<webidl::UsvString>(scope, value, context)?.0;
        Ok(Self(
            v8_string(scope, &text)
                .ok_or_else(|| webidl::WebIdlError::pending_exception(context))?
                .into(),
        ))
    }
}

pub(super) fn add_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    start(scope, args, &mut rv, false);
}

pub(super) fn add_all_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    start(scope, args, &mut rv, true);
}

fn start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    multiple: bool,
) {
    let method = if multiple { "addAll" } else { "add" };
    let Some(resolver) = storage_bucket_cache_resolver(scope, &args, rv, method) else {
        return;
    };
    let values = {
        v8::tc_scope!(let scope, scope);
        let converted = if multiple {
            <AddAllArgs<'s> as webidl::WebIdlArguments<'s>>::parse_arguments(scope, &args)
                .map(|values| values.requests.0)
        } else {
            <AddArgs<'s> as webidl::WebIdlArguments<'s>>::parse_arguments(scope, &args)
                .map(|value| vec![value.request])
        };
        match converted {
            Ok(values) => values,
            Err(error) => {
                webidl::throw_error(scope, &error);
                let exception = scope
                    .exception()
                    .unwrap_or_else(|| v8::undefined(scope).into());
                scope.reset();
                let _ = resolver.reject(scope, exception);
                return;
            }
        }
    };
    let cache = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("branded Cache has native identity");
    if !storage_bucket_receiver_execution_context_is_live(scope, cache) {
        reject_storage_bucket_invalid_state(scope, resolver, method);
        return;
    }
    let Some(handle) = storage_bucket_cache_live_handle(scope, cache, resolver, method) else {
        return;
    };
    // WebIDL converts the whole sequence first. The algorithm then validates
    // all existing Request inputs before constructing or fetching string inputs.
    for value in &values {
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value.0)
            && web_api_interfaces::Request::is_instance(scope, object)
            && !request_is_cacheable(scope, object)
        {
            reject_type_error(
                scope,
                resolver,
                "Cache.addAll requires HTTP(S) GET requests.",
            );
            return;
        }
    }
    let callee_context = scope.get_current_context();
    let owner_context = cache
        .get_creation_context(scope)
        .expect("native Cache has a realm");
    let scope = &mut v8::ContextScope::new(scope, owner_context);
    if values.is_empty() {
        let resolver = publish_result_resolver(scope, callee_context, resolver, rv);
        let _ = resolver.resolve(scope, v8::undefined(scope).into());
        return;
    }
    let cancel =
        new_cancel_signal(scope).expect("Cache fetch cancellation signal should initialize");
    let mut state = Box::new(RefCell::new(BatchState {
        handle,
        remaining: values.len(),
        entries: Vec::with_capacity(values.len()),
        settled: false,
    }));
    let pointer = (&mut *state as *mut RefCell<BatchState>).cast();
    let external = v8::External::new(scope, pointer);
    let batch = BatchDeclaration::new(external, resolver, cache, cancel)
        .bind(scope)
        .expect("Cache fetch batch should bind");
    crate::v8_finalizer::track_context_owned_v8_finalizer(scope, batch, move || drop(state));
    let constructor = {
        let scope = &mut v8::ContextScope::new(scope, callee_context);
        crate::context_bootstrap::ensure_intrinsic_interface_constructor(scope, "Request")
            .expect("Cache Request intrinsic should materialize")
    };
    for (index, input) in values.into_iter().enumerate() {
        let request = {
            // Construction failures are WebIDL rejections in the method's
            // realm. Fetch/body callbacks and the cache job use the owner realm.
            let scope = &mut v8::ContextScope::new(scope, callee_context);
            v8::tc_scope!(let scope, scope);
            match crate::script_execution::construct(scope, constructor, &[input.0]) {
                Some(request) if request_is_cacheable(scope, request) => Ok(request),
                Some(_) => {
                    let message = v8str(scope, "Cache.addAll requires HTTP(S) GET requests.");
                    Err(v8::Exception::type_error(scope, message))
                }
                None => {
                    let exception = scope
                        .exception()
                        .unwrap_or_else(|| v8::undefined(scope).into());
                    scope.reset();
                    Err(exception)
                }
            }
        };
        let request = match request {
            Ok(request) => request,
            Err(exception) => {
                reject(scope, batch, exception);
                return;
            }
        };
        let snapshot = crate::network_host::request_input_snapshot(scope, request.into())
            .expect("intrinsic Request has readable native slots")
            .expect("intrinsic Request is branded");
        let mut signals = vec![cancel];
        if let Some(signal) = snapshot.signal {
            signals.push(v8::Local::new(scope, signal));
        }
        let signal = if crate::worker::get_worker_state(scope).is_some() {
            crate::worker::abort::new_worker_dependent_abort_signal(scope, &signals)
        } else {
            crate::native_bridge::abort::new_dependent_abort_signal(scope, &signals)
        }
        .expect("Cache request cancellation dependency should initialize");
        let init = crate::util::new_null_prototype_object(scope);
        let key = v8str(scope, "signal");
        init.create_data_property(scope, key.into(), signal.into());
        let request =
            crate::script_execution::construct(scope, constructor, &[request.into(), init.into()])
                .expect("native GET Request clone with internal signal should construct");
        batch_state(scope, batch).borrow_mut().entries.push(Entry {
            request: CacheRequestInfo {
                url: snapshot.url,
                method: snapshot.method,
                headers: snapshot.headers,
            },
            head: None,
            response: None,
        });
        let data = ReactionDeclaration::new(batch, index as u32)
            .bind(scope)
            .expect("Cache reaction data should bind");
        let promise = crate::network_host::fetch_native_request(scope, request)
            .expect("native Cache Fetch should return a promise");
        if !attach(scope, batch, promise, data, response_fulfilled) {
            return;
        }
    }
    if !batch_state(scope, batch).borrow().settled {
        let resolver = publish_result_resolver(scope, callee_context, resolver, rv);
        set_private_value(scope, batch, RESOLVER, resolver.into());
    }
}

fn publish_result_resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callee_context: v8::Local<'s, v8::Context>,
    callee_resolver: v8::Local<'s, v8::PromiseResolver>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) -> v8::Local<'s, v8::PromiseResolver> {
    if scope.get_current_context() == callee_context {
        return callee_resolver;
    }
    let resolver = v8::PromiseResolver::new(scope).expect("Cache job promise should initialize");
    rv.set(resolver.get_promise(scope).into());
    resolver
}

fn request_is_cacheable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> bool {
    crate::network_host::request_method(scope, request) == "GET"
        && crate::network_host::request_slot_string(
            scope,
            request,
            crate::network_host::REQUEST_URL_SLOT,
        )
        .is_some_and(|url| url.starts_with("http://") || url.starts_with("https://"))
}

fn new_cancel_signal<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Object>> {
    if crate::worker::get_worker_state(scope).is_some() {
        crate::worker::abort::new_worker_abort_signal(scope)
    } else {
        crate::native_bridge::abort::new_dependent_abort_signal(scope, &[])
    }
}

fn batch_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    batch: v8::Local<'s, v8::Object>,
) -> &'s RefCell<BatchState> {
    let external = v8::Local::<v8::External>::try_from(
        get_private_value(scope, batch, STATE).expect("native batch state"),
    )
    .expect("native batch state pointer");
    // The hidden batch object owns the Box through its context finalizer.
    // Local callback data keeps it alive, and disconnected contexts are checked
    // before entering a reaction. No borrow spans V8 calls or author callbacks.
    unsafe { &*external.value().cast::<RefCell<BatchState>>() }
}

fn resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    batch: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::PromiseResolver> {
    let object = v8::Local::<v8::Object>::try_from(
        get_private_value(scope, batch, RESOLVER).expect("native batch resolver"),
    )
    .expect("native batch resolver object");
    unsafe { v8::Local::cast_unchecked(object) }
}

fn reaction<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<(v8::Local<'s, v8::Object>, usize)> {
    let data = v8::Local::<v8::Object>::try_from(data).ok()?;
    if !storage_bucket_receiver_execution_context_is_live(scope, data) {
        return None;
    }
    let batch = v8::Local::<v8::Object>::try_from(get_private_value(scope, data, BATCH)?).ok()?;
    let index = get_private_value(scope, data, INDEX)?.uint32_value(scope)? as usize;
    if batch_state(scope, batch).borrow().settled {
        return None;
    }
    Some((batch, index))
}

fn attach<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    batch: v8::Local<'s, v8::Object>,
    promise: v8::Local<'s, v8::Promise>,
    data: v8::Local<'s, v8::Object>,
    fulfilled: impl v8::MapFnTo<v8::FunctionCallback>,
) -> bool {
    let on_fulfilled = v8::Function::builder(fulfilled)
        .data(data.into())
        .build(scope)
        .expect("Cache fulfillment callback should materialize");
    let on_rejected = v8::Function::builder(rejected)
        .data(data.into())
        .build(scope)
        .expect("Cache rejection callback should materialize");
    if promise.then2(scope, on_fulfilled, on_rejected).is_none() {
        reject_type_error_batch(scope, batch, "Cache failed to attach fetch reactions.");
        return false;
    }
    true
}

fn response_fulfilled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((batch, index)) = reaction(scope, args.data()) else {
        return;
    };
    let (head, response) =
        match crate::network_host::materialize_cache_response_object_head(scope, args.get(0)) {
            Ok(response) => response,
            Err(error) => {
                reject_type_error_batch(scope, batch, &error);
                return;
            }
        };
    if !(200..300).contains(&head.status)
        || head.status == 206
        || matches!(
            head.response_type.as_str(),
            "opaque" | "opaqueredirect" | "error"
        )
    {
        reject_type_error_batch(
            scope,
            batch,
            "Cache.addAll fetch did not return an OK response.",
        );
        return;
    }
    match crate::network_host::materialize_response_object_body_preserving_error(
        scope,
        response,
        "Cache.addAll",
    ) {
        Ok(MaterializedResponseBody::Ready(body)) => complete_entry(
            scope,
            batch,
            index,
            storage_bucket_cached_response_from_head_body(head, body),
        ),
        Ok(MaterializedResponseBody::Pending(promise)) => {
            batch_state(scope, batch).borrow_mut().entries[index].head = Some(head);
            let data =
                v8::Local::<v8::Object>::try_from(args.data()).expect("Cache body reaction data");
            attach(scope, batch, promise, data, body_fulfilled);
        }
        Ok(MaterializedResponseBody::Failure(error)) => {
            reject_type_error_batch(scope, batch, &error)
        }
        Err(reason) => reject(scope, batch, reason),
    }
}

fn body_fulfilled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((batch, index)) = reaction(scope, args.data()) else {
        return;
    };
    let body = match crate::network_host::materialized_body_bytes_from_value(scope, args.get(0)) {
        Ok(body) => body,
        Err(error) => {
            reject_type_error_batch(scope, batch, &error);
            return;
        }
    };
    let head = batch_state(scope, batch).borrow_mut().entries[index]
        .head
        .take()
        .expect("pending Cache body head");
    complete_entry(
        scope,
        batch,
        index,
        storage_bucket_cached_response_from_head_body(head, body),
    );
}

fn complete_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    batch: v8::Local<'s, v8::Object>,
    index: usize,
    response: StorageBucketCachedResponse,
) {
    let ready = {
        let mut state = batch_state(scope, batch).borrow_mut();
        state.entries[index].response = Some(response);
        state.remaining -= 1;
        if state.remaining != 0 {
            return;
        }
        state.settled = true;
        let operations = std::mem::take(&mut state.entries)
            .into_iter()
            .map(|entry| {
                let response = entry.response.expect("completed Cache batch response");
                let usage_bytes = cache_entry_usage_bytes(&entry.request, &response);
                StorageBucketCachePut {
                    request_url: entry.request.url,
                    request: StorageBucketCachedRequest {
                        method: entry.request.method,
                        headers: entry.request.headers,
                    },
                    response,
                    usage_bytes,
                }
            })
            .collect();
        (state.handle.clone(), operations)
    };
    let resolver = resolver(scope, batch);
    store_batch(scope, resolver, ready.0, ready.1);
    cancel(scope, batch);
}

fn rejected<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((batch, _)) = reaction(scope, args.data()) else {
        return;
    };
    reject(scope, batch, args.get(0));
}

fn reject_type_error_batch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    batch: v8::Local<'s, v8::Object>,
    message: &str,
) {
    let message = v8_string(scope, message).expect("Cache error string should materialize");
    let exception = v8::Exception::type_error(scope, message);
    reject(scope, batch, exception);
}

fn reject<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    batch: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    {
        let mut state = batch_state(scope, batch).borrow_mut();
        if state.settled {
            return;
        }
        state.settled = true;
        state.entries.clear();
    }
    let _ = resolver(scope, batch).reject(scope, reason);
    cancel(scope, batch);
}

fn cancel<'s>(scope: &mut v8::PinScope<'s, '_>, batch: v8::Local<'s, v8::Object>) {
    let signal = v8::Local::<v8::Object>::try_from(
        get_private_value(scope, batch, CANCEL).expect("native batch cancel signal"),
    )
    .expect("native batch cancel signal object");
    let reason = crate::native_bridge::abort::abort_error_value(scope);
    if crate::worker::get_worker_state(scope).is_some() {
        let id = crate::worker::abort::worker_abort_signal_id(scope, signal)
            .expect("native worker signal identity");
        crate::worker::abort::abort_worker_signal_by_id(scope, id, reason);
    } else {
        crate::native_bridge::abort::abort_signal(scope, signal, reason);
    }
}

fn store_batch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    handle: StorageBucketCacheHandle,
    operations: Vec<StorageBucketCachePut>,
) {
    let locator = with_storage_bucket_store_entry(scope, |store| {
        store.bucket_locator_for_identity(&handle.bucket.identity)
    })
    .flatten();
    let Some(owner) = locator
        .as_ref()
        .and_then(|locator| storage_bucket_quota_owner_for_locator(scope, locator))
    else {
        reject_storage_bucket_unknown_error(scope, resolver, "cache.addAll");
        return;
    };
    let _reservation = owner.reserve_commit();
    let non_cache_usage = match owner.quota_and_non_cache_usage() {
        Ok((_, usage)) => usage,
        Err(error) => {
            reject_type_error(scope, resolver, &error.to_string());
            return;
        }
    };
    let outcome = with_storage_bucket_store_entry(scope, |store| {
        store.put_cache_batch_for_handle_and_identity(
            &handle.bucket.identity,
            &handle.cache_name,
            handle.cache_id,
            operations,
            non_cache_usage,
        )
    });
    match outcome {
        Some(Ok(StorageBucketCachePutOutcome::Stored)) => {
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
        }
        Some(Ok(StorageBucketCachePutOutcome::Duplicate)) => {
            let exception = new_dom_exception_value(
                scope,
                "Cache batch contains duplicate requests.",
                "InvalidStateError",
            );
            let _ = resolver.reject(scope, exception);
        }
        Some(Ok(StorageBucketCachePutOutcome::QuotaExceeded { quota, requested })) => {
            reject_storage_bucket_quota_exceeded(scope, resolver, quota, requested)
        }
        Some(Ok(StorageBucketCachePutOutcome::Stale)) => {
            reject_storage_bucket_unknown_error(scope, resolver, "cache.addAll")
        }
        Some(Err(error)) => reject_type_error(scope, resolver, &error.to_string()),
        None => reject_type_error(
            scope,
            resolver,
            "Cache.addAll storage bucket store is unavailable.",
        ),
    }
}
