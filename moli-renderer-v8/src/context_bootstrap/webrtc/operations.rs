//! The connection's native operations chain. Networking tasks retain their
//! exact owner; promise reactions retain the operation's calling realm.

use crate::{
    native_bridge::throw_dom_exception,
    page_task_queue::RendererPageWebRtcTaskKind,
    util::{
        context_host_ptr_from_global_bridge, get_private_object, get_private_value,
        set_private_value,
    },
};
use moli_webapi_declare::WebApiObject;

const OPERATIONS: &str = "__moliRtcOperations";
const DEFER_NEGOTIATION: &str = "__moliRtcNegotiateOnEmptyChain";
const OWNER: &str = "__moliRtcOperationOwner";
const KIND: &str = "__moliRtcOperationKind";
const PAYLOAD: &str = "__moliRtcOperationPayload";
const INNER: &str = "__moliRtcOperationInner";
const OUTER: &str = "__moliRtcOperationOuter";

#[derive(Clone, Copy)]
pub(super) enum Kind {
    CreateOffer,
    SetLocalDescription,
    ReplaceTrack,
}
impl Kind {
    fn task(self) -> RendererPageWebRtcTaskKind {
        match self {
            Self::CreateOffer => RendererPageWebRtcTaskKind::CreateOffer,
            Self::SetLocalDescription => RendererPageWebRtcTaskKind::SetLocalDescription,
            Self::ReplaceTrack => RendererPageWebRtcTaskKind::ReplaceTrack,
        }
    }
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct Request<'s> {
    #[webapi(slot = OWNER)]
    owner: v8::Local<'s, v8::Object>,
    #[webapi(slot = KIND)]
    kind: v8::Local<'s, v8::Integer>,
    #[webapi(slot = PAYLOAD)]
    payload: v8::Local<'s, v8::Value>,
    #[webapi(slot = INNER)]
    inner: v8::Local<'s, v8::PromiseResolver>,
    #[webapi(slot = OUTER)]
    outer: v8::Local<'s, v8::PromiseResolver>,
}

pub(super) fn initialize<'s>(scope: &mut v8::PinScope<'s, '_>, pc: v8::Local<'s, v8::Object>) {
    let list = v8::Array::new(scope, 0);
    set_private_value(scope, pc, OPERATIONS, list.into());
    set_private_value(
        scope,
        pc,
        DEFER_NEGOTIATION,
        v8::Boolean::new(scope, false).into(),
    );
}
fn list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    v8::Local::try_from(get_private_value(scope, pc, OPERATIONS).expect("operations chain"))
        .expect("operations array")
}
pub(super) fn is_empty<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
) -> bool {
    list(scope, pc).length() == 0
}
pub(super) fn defer_negotiation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        pc,
        DEFER_NEGOTIATION,
        v8::Boolean::new(scope, true).into(),
    );
}
pub(super) fn owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    get_private_object(scope, request, OWNER).expect("operation owner")
}
pub(super) fn payload<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    get_private_value(scope, request, PAYLOAD).expect("operation payload")
}
fn resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    slot: &str,
) -> v8::Local<'s, v8::PromiseResolver> {
    let object = get_private_object(scope, request, slot).expect("operation resolver");
    // Only the native Request declaration can populate these private slots.
    unsafe { v8::Local::cast_unchecked(object) }
}
pub(super) fn resolve<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    resolver(scope, request, INNER).resolve(scope, value);
}
pub(super) fn reject<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    resolver(scope, request, INNER).reject(scope, reason);
}
pub(super) fn queue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    payload: v8::Local<'s, v8::Object>,
    kind: RendererPageWebRtcTaskKind,
) -> bool {
    context_host_ptr_from_global_bridge(scope).is_some_and(|host| {
        unsafe { &mut *host }.queue_webrtc_task_for_owner(scope, owner, payload, kind)
    })
}

pub(super) fn enqueue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: v8::Local<'s, v8::Object>,
    kind: Kind,
    payload: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Promise>> {
    if super::rtp_transceivers::closed(scope, pc) {
        throw_dom_exception(scope, "InvalidStateError", 11, "The connection is closed.");
        return None;
    }
    let inner = v8::PromiseResolver::new(scope)?;
    let outer = v8::PromiseResolver::new(scope)?;
    let request = Request::new(
        pc,
        v8::Integer::new(scope, kind as i32),
        payload,
        inner,
        outer,
    )
    .bind(scope)
    .ok()?;
    let fulfill = v8::Function::builder(fulfilled)
        .data(request.into())
        .build(scope)?;
    let reject = v8::Function::builder(rejected)
        .data(request.into())
        .build(scope)?;
    inner.get_promise(scope).then2(scope, fulfill, reject)?;
    let pending = list(scope, pc);
    pending.set_index(scope, pending.length(), request.into())?;
    if pending.length() == 1 {
        start(scope, request);
    }
    Some(outer.get_promise(scope))
}

fn start<'s>(scope: &mut v8::PinScope<'s, '_>, request: v8::Local<'s, v8::Object>) {
    let context = request
        .get_creation_context(scope)
        .expect("operation realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    let pc = owner(scope, request);
    if super::rtp_transceivers::closed(scope, pc) {
        return;
    }
    let kind = match get_private_value(scope, request, KIND)
        .expect("operation kind")
        .int32_value(scope)
        .expect("operation tag")
    {
        0 => Kind::CreateOffer,
        1 => Kind::SetLocalDescription,
        2 => Kind::ReplaceTrack,
        _ => unreachable!("native operation tag"),
    };
    if matches!(kind, Kind::ReplaceTrack) && !super::rtp_sender::start_replace_track(scope, request)
    {
        return;
    }
    queue(scope, pc, request, kind.task());
}

fn fulfilled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    finish(scope, &args, false);
}
fn rejected<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    finish(scope, &args, true);
}
fn finish<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    rejected: bool,
) {
    let request =
        v8::Local::<v8::Object>::try_from(args.data()).expect("native operation reaction");
    let pc = owner(scope, request);
    if super::rtp_transceivers::closed(scope, pc) {
        return;
    }
    let outer = resolver(scope, request, OUTER);
    let settled = v8::Function::builder(advance)
        .data(request.into())
        .build(scope)
        .expect("operation completion reaction");
    outer.get_promise(scope).then2(scope, settled, settled);
    if rejected {
        outer.reject(scope, args.get(0));
    } else {
        outer.resolve(scope, args.get(0));
    }
}
fn advance<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let request =
        v8::Local::<v8::Object>::try_from(args.data()).expect("native operation reaction");
    let pc = owner(scope, request);
    if super::rtp_transceivers::closed(scope, pc) {
        return;
    }
    let pending = list(scope, pc);
    let rest: Vec<_> = (1..pending.length())
        .map(|i| pending.get_index(scope, i).expect("queued operation"))
        .collect();
    let next = v8::Array::new_with_elements(scope, &rest);
    set_private_value(scope, pc, OPERATIONS, next.into());
    if let Some(first) = rest.first() {
        start(
            scope,
            v8::Local::try_from(*first).expect("operation object"),
        );
    } else if get_private_value(scope, pc, DEFER_NEGOTIATION)
        .expect("deferred negotiation")
        .boolean_value(scope)
    {
        set_private_value(
            scope,
            pc,
            DEFER_NEGOTIATION,
            v8::Boolean::new(scope, false).into(),
        );
        super::rtp_transceivers::update_negotiation_needed(scope, pc);
    }
}

pub(super) fn close<'s>(scope: &mut v8::PinScope<'s, '_>, pc: v8::Local<'s, v8::Object>) {
    // Retired/closed operations deliberately never complete their promises.
    initialize(scope, pc);
}

pub(super) fn apply<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request: v8::Local<'s, v8::Object>,
    kind: RendererPageWebRtcTaskKind,
) -> bool {
    let pc = owner(scope, request);
    if super::rtp_transceivers::closed(scope, pc) {
        return false;
    }
    let context = request
        .get_creation_context(scope)
        .expect("operation realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    v8::tc_scope!(let caught, scope);
    let completed = match kind {
        RendererPageWebRtcTaskKind::CreateOffer => {
            super::complete_create_offer(caught, request, pc)
        }
        RendererPageWebRtcTaskKind::SetLocalDescription => {
            super::complete_set_local_description(caught, request, pc)
        }
        RendererPageWebRtcTaskKind::ReplaceTrack => {
            super::rtp_sender::apply_replace_track(caught, request, pc)
        }
        RendererPageWebRtcTaskKind::CompleteReplaceTrack => {
            let value = v8::undefined(caught);
            resolve(caught, request, value.into());
            true
        }
        _ => unreachable!("connection operation task"),
    };
    // Fallible native planning runs asynchronously, after the promise was
    // returned. Reject that operation so its chain can advance. Termination
    // remains owned by the task boundary and must not become a JS rejection.
    if !completed
        && !caught.has_terminated()
        && let Some(reason) = caught.exception()
    {
        caught.reset();
        reject(caught, request, reason);
        true
    } else {
        completed
    }
}
