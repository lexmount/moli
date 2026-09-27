//! Deliver public Body conversions on the receiver global's networking task
//! source. The private carrier keeps bytes out of promise/thenable assimilation.

use super::*;

const RESOLVER: &str = "__moliBodyCompletionResolver";
const KIND: &str = "__moliBodyCompletionKind";
const RESULT: &str = "__moliBodyCompletionResult";
const HEADERS: &str = "__moliBodyCompletionHeaders";
const REALM: &str = "__moliBodyCompletionRealm";
const SUCCEEDED: &str = "__moliBodyCompletionSucceeded";

#[derive(WebApiObject)]
#[webapi(plain)]
struct BodyCompletionDeclaration<'scope> {
    #[webapi(slot = RESOLVER)]
    resolver: v8::Local<'scope, v8::Object>,
    #[webapi(slot = KIND)]
    kind: &'static str,
    #[webapi(slot = RESULT)]
    result: v8::Local<'scope, v8::Value>,
    #[webapi(slot = HEADERS)]
    headers: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(slot = REALM)]
    realm: Option<v8::Local<'scope, v8::Object>>,
    #[webapi(slot = SUCCEEDED)]
    succeeded: bool,
}

fn realm_marker<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Global<v8::Context>,
) -> v8::Local<'s, v8::Object> {
    let context = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, context);
    // Unlike a WindowProxy, this object's creation realm cannot be retargeted
    // by navigation while the task is pending.
    v8::Object::new(scope)
}

pub(super) fn queue_body_completion<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    result: Result<Vec<u8>, v8::Local<'s, v8::Value>>,
    kind: PendingBodyMaterializationKind,
    destination: v8::Global<v8::Context>,
) {
    let (kind, headers, realm) = match kind {
        PendingBodyMaterializationKind::Text => ("text", None, None),
        PendingBodyMaterializationKind::Json => ("json", None, None),
        PendingBodyMaterializationKind::ArrayBuffer(realm) => {
            ("arrayBuffer", None, Some(realm_marker(scope, realm)))
        }
        PendingBodyMaterializationKind::Bytes(realm) => {
            ("bytes", None, Some(realm_marker(scope, realm)))
        }
        PendingBodyMaterializationKind::Blob { headers } => (
            "blob",
            headers.map(|headers| v8::Local::new(scope, headers)),
            None,
        ),
        PendingBodyMaterializationKind::FormData { headers } => (
            "formData",
            headers.map(|headers| v8::Local::new(scope, headers)),
            None,
        ),
    };
    let succeeded = result.is_ok();
    let result = match result {
        Ok(bytes) => blob::array_buffer_from_bytes(scope, bytes)
            .expect("body completion backing store must allocate")
            .into(),
        Err(error) => error,
    };
    let destination = v8::Local::new(scope, destination);
    let scope = &mut v8::ContextScope::new(scope, destination);
    let data =
        BodyCompletionDeclaration::new(resolver.into(), kind, result, headers, realm, succeeded)
            .bind(scope)
            .expect("body completion carrier must bind");
    let callback = v8::Function::builder(complete)
        .data(data.into())
        .build(scope)
        .expect("body completion callback must allocate");
    queue_fetch_task(scope, callback);
}

/// The caller enters the task destination before creating its browser callback.
/// Window ownership checks and worker shutdown retire these tasks with that
/// exact global; a retired destination must not fall back to a microtask.
pub(super) fn queue_fetch_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: v8::Local<'s, v8::Function>,
) {
    if let Some(host) = context_host_mut(scope) {
        host.queue_networking_task(scope, callback);
    } else {
        crate::worker::queue_worker_networking_task(scope, callback);
    }
}

fn complete<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let data = v8::Local::<v8::Object>::try_from(args.data()).expect("body completion carrier");
    let resolver = v8::Local::<v8::Object>::try_from(
        get_private_value(scope, data, RESOLVER).expect("body completion resolver"),
    )
    .expect("body completion resolver object");
    // SAFETY: only queue_body_completion initializes this private slot, with a
    // PromiseResolver. The task consumes and clears the carrier exactly once.
    let resolver = unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(resolver) };
    let context = resolver
        .get_creation_context(scope)
        .expect("Body promise realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let result = get_private_value(scope, data, RESULT).expect("body completion result");
    let succeeded = get_private_value(scope, data, SUCCEEDED).is_some_and(|value| value.is_true());
    let kind = get_private_value(scope, data, KIND)
        .expect("body completion kind")
        .to_rust_string_lossy(scope);
    let headers = get_private_value(scope, data, HEADERS)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|value| v8::Global::new(scope, value));
    let realm = get_private_value(scope, data, REALM)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|value| value.get_creation_context(scope))
        .map(|value| v8::Global::new(scope, value));
    let null = v8::null(scope).into();
    for slot in [RESOLVER, RESULT, HEADERS, REALM] {
        set_private_value(scope, data, slot, null);
    }
    if !succeeded {
        let _ = resolver.reject(scope, result);
        return;
    }
    let kind = match kind.as_str() {
        "text" => PendingBodyMaterializationKind::Text,
        "json" => PendingBodyMaterializationKind::Json,
        "arrayBuffer" => PendingBodyMaterializationKind::ArrayBuffer(realm.expect("binary realm")),
        "bytes" => PendingBodyMaterializationKind::Bytes(realm.expect("binary realm")),
        "blob" => PendingBodyMaterializationKind::Blob { headers },
        "formData" => PendingBodyMaterializationKind::FormData { headers },
        _ => unreachable!("body completion kind is private"),
    };
    let bytes = blob::buffer_source_bytes_from_value(scope, result).expect("body completion bytes");
    resolve_body_materialization(scope, resolver, bytes, kind, None);
}
