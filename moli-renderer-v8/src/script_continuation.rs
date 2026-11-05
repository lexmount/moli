//! Data V8 carries across Promise reactions and async continuations, including
//! the Window generation and origin captured before a subsequent navigation.
use crate::{
    native_bridge::{
        RuntimeObservableContextToken, WindowExecutionContextIdentity, WindowOriginKey,
        WindowSecurityOrigin,
    },
    util::{get_private_value, set_private_value},
};
use std::rc::{Rc, Weak};

const BASE_URL_SLOT: &str = "moli.scriptContinuationBaseURL";
const WINDOW_SLOT: &str = "moli.scriptContinuationWindow";
const WINDOW_STATE_SLOT: &str = "moli.scriptContinuationWindowState";
const RESOURCE_INITIATOR_SLOT: &str = "moli.scriptContinuationResourceInitiator";
const RESOURCE_INITIATOR_STATE_SLOT: &str = "moli.scriptContinuationResourceInitiatorState";

/// Attribution is separate from the script's resolution base and from Window
/// authority. A redacted script still establishes an explicit unknown source.
#[derive(Clone, Debug)]
pub(crate) struct ResourceInitiator {
    pub(crate) source_url: Option<url::Url>,
    pub(crate) origin: Option<ResourceInitiatorOrigin>,
}

/// Attribution identifies the originating environment without rooting its
/// callback graph or granting permission to execute a callback.
#[derive(Clone, Debug)]
pub(crate) enum ResourceInitiatorOrigin {
    Window(WindowOriginKey),
    Worker {
        runtime: Weak<crate::common_runtime::CommonRuntimeState>,
        origin: moli_url::WebOrigin,
    },
}

impl ResourceInitiatorOrigin {
    pub(crate) fn for_worker(scope: &mut v8::PinScope<'_, '_>) -> Option<Self> {
        let runtime = crate::common_runtime::CommonRuntimeState::from_scope(scope)?;
        let origin = crate::worker::worker_global_origin(scope)?;
        Some(Self::Worker {
            runtime: Rc::downgrade(&runtime),
            origin,
        })
    }

    pub(crate) fn matches_worker(&self, scope: &mut v8::PinScope<'_, '_>) -> bool {
        let Self::Worker { runtime, origin } = self else {
            return false;
        };
        let Some(current) = crate::common_runtime::CommonRuntimeState::from_scope(scope) else {
            return false;
        };
        runtime.ptr_eq(&Rc::downgrade(&current))
            && crate::worker::worker_global_origin(scope).as_ref() == Some(origin)
    }
}

/// Immutable attribution captured for one callback registration. Its V8
/// anchor carries no callback or scheduling authority.
#[derive(Clone)]
pub(crate) struct CallbackResourceInitiator {
    anchor: Rc<Option<v8::Global<v8::Object>>>,
}

impl CallbackResourceInitiator {
    pub(crate) fn capture(scope: &mut v8::PinScope<'_, '_>) -> Self {
        let anchor = resource_initiator_anchor(scope).or_else(|| unknown_resource_initiator(scope));
        Self {
            anchor: Rc::new(anchor.map(|anchor| v8::Global::new(scope, anchor))),
        }
    }

    pub(crate) fn from_value<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
    ) -> Self {
        Self {
            anchor: Rc::new(
                v8::Local::<v8::Object>::try_from(value)
                    .ok()
                    .map(|anchor| v8::Global::new(scope, anchor)),
            ),
        }
    }

    pub(crate) fn value<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        self.anchor
            .as_ref()
            .as_ref()
            .map(|anchor| v8::Local::new(scope, anchor).into())
            .unwrap_or_else(|| v8::undefined(scope).into())
    }
}

/// Select before entering a callback Realm: a synchronous caller, including an
/// unknown one, takes precedence over the callback's registration source.
pub(crate) fn callback_resource_initiator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration: Option<&CallbackResourceInitiator>,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(anchor) = resource_initiator_anchor(scope) {
        return Some(anchor);
    }
    let registration = registration?;
    if v8::StackTrace::current_stack_trace(scope, 1)
        .is_none_or(|stack| stack.get_frame_count() != 0)
    {
        return unknown_resource_initiator(scope);
    }
    registration
        .anchor
        .as_ref()
        .as_ref()
        .map(|anchor| v8::Local::new(scope, anchor))
}

pub(crate) fn with_callback_resource_initiator<'s, R>(
    scope: &mut v8::PinScope<'s, '_>,
    anchor: Option<v8::Local<'s, v8::Object>>,
    invoke: impl FnOnce(&mut v8::PinScope<'s, '_>) -> R,
) -> R {
    let Some(anchor) = anchor else {
        return invoke(scope);
    };
    let previous = scope.get_continuation_preserved_embedder_data();
    let base_url = base_url_value(scope);
    let window = window_anchor(scope);
    let data = data_with_parts(scope, base_url, window, Some(anchor));
    scope.set_continuation_preserved_embedder_data(data);
    let result = invoke(scope);
    scope.set_continuation_preserved_embedder_data(previous);
    result
}

fn unknown_resource_initiator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    // Bare callback-layer test contexts have no renderer-owned finalizers.
    crate::common_runtime::CommonRuntimeState::from_scope(scope)?;
    Some(new_resource_initiator_anchor(
        scope,
        ResourceInitiator {
            source_url: None,
            origin: None,
        },
    ))
}

#[derive(Clone)]
pub(crate) struct WindowContinuation {
    pub(crate) identity: WindowExecutionContextIdentity,
    pub(crate) origin: WindowSecurityOrigin,
}

pub(crate) fn base_url_value<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
    let data = scope.get_continuation_preserved_embedder_data();
    if let Ok(object) = v8::Local::<v8::Object>::try_from(data)
        && (get_private_value(scope, object, WINDOW_SLOT).is_some()
            || get_private_value(scope, object, RESOURCE_INITIATOR_SLOT).is_some())
    {
        // Undefined is a valid base URL payload, not the absence of the
        // surrounding continuation record.
        return get_private_value(scope, object, BASE_URL_SLOT)
            .unwrap_or_else(|| v8::undefined(scope).into());
    }
    data
}

fn resource_initiator_anchor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let data =
        v8::Local::<v8::Object>::try_from(scope.get_continuation_preserved_embedder_data()).ok()?;
    v8::Local::<v8::Object>::try_from(get_private_value(scope, data, RESOURCE_INITIATOR_SLOT)?).ok()
}

pub(crate) fn resource_initiator(scope: &mut v8::PinScope<'_, '_>) -> Option<ResourceInitiator> {
    let anchor = resource_initiator_anchor(scope)?;
    let value = get_private_value(scope, anchor, RESOURCE_INITIATOR_STATE_SLOT)?;
    let external = v8::Local::<v8::External>::try_from(value).ok()?;
    // The private anchor owns this allocation until its V8 finalizer runs.
    Some(unsafe { &*external.value().cast::<ResourceInitiator>() }.clone())
}

pub(crate) fn with_resource_initiator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    initiator: ResourceInitiator,
) -> v8::Local<'s, v8::Value> {
    let base_url = base_url_value(scope);
    let window = window_anchor(scope);
    let anchor = new_resource_initiator_anchor(scope, initiator);
    data_with_parts(scope, base_url, window, Some(anchor))
}

fn new_resource_initiator_anchor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    initiator: ResourceInitiator,
) -> v8::Local<'s, v8::Object> {
    let anchor = v8::Object::new(scope);
    let mut initiator = Box::new(initiator);
    let pointer = (&mut *initiator as *mut ResourceInitiator).cast();
    let external = v8::External::new(scope, pointer);
    set_private_value(
        scope,
        anchor,
        RESOURCE_INITIATOR_STATE_SLOT,
        external.into(),
    );
    crate::v8_finalizer::track_context_owned_v8_finalizer(scope, anchor, move || drop(initiator));
    anchor
}

fn window_anchor<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Object>> {
    let data =
        v8::Local::<v8::Object>::try_from(scope.get_continuation_preserved_embedder_data()).ok()?;
    v8::Local::<v8::Object>::try_from(get_private_value(scope, data, WINDOW_SLOT)?).ok()
}

fn window(scope: &mut v8::PinScope<'_, '_>) -> Option<WindowContinuation> {
    let anchor = window_anchor(scope)?;
    let value = get_private_value(scope, anchor, WINDOW_STATE_SLOT)?;
    let external = v8::Local::<v8::External>::try_from(value).ok()?;
    // The private native anchor owns this allocation through its finalizer.
    Some(unsafe { &*external.value().cast::<WindowContinuation>() }.clone())
}

pub(crate) fn running_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Context>,
) -> Option<WindowContinuation> {
    if context != scope.get_current_context()
        || !context
            .get_microtask_queue()
            .is_some_and(v8::MicrotaskQueue::is_running_microtasks)
    {
        return None;
    }
    let owner = window(scope)?;
    let token = context.get_slot::<RuntimeObservableContextToken>()?;
    (owner.identity.realm_token() == *token).then_some(owner)
}

fn data_with_parts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    base_url: v8::Local<'s, v8::Value>,
    window: Option<v8::Local<'s, v8::Object>>,
    resource_initiator: Option<v8::Local<'s, v8::Object>>,
) -> v8::Local<'s, v8::Value> {
    if window.is_none() && resource_initiator.is_none() {
        return base_url;
    }
    let data = v8::Object::new(scope);
    set_private_value(scope, data, BASE_URL_SLOT, base_url);
    if let Some(anchor) = window {
        set_private_value(scope, data, WINDOW_SLOT, anchor.into());
    }
    if let Some(anchor) = resource_initiator {
        set_private_value(scope, data, RESOURCE_INITIATOR_SLOT, anchor.into());
    }
    data.into()
}

pub(crate) fn with_base_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    base_url: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let window = window_anchor(scope);
    let initiator = resource_initiator_anchor(scope);
    data_with_parts(scope, base_url, window, initiator)
}

pub(crate) fn with_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: WindowContinuation,
) -> v8::Local<'s, v8::Value> {
    if window(scope).is_some_and(|current| current.identity == owner.identity) {
        return scope.get_continuation_preserved_embedder_data();
    }
    let base_url = base_url_value(scope);
    let initiator = resource_initiator_anchor(scope);
    let anchor = v8::Object::new(scope);
    let mut owner = Box::new(owner);
    let pointer = (&mut *owner as *mut WindowContinuation).cast();
    let external = v8::External::new(scope, pointer);
    set_private_value(scope, anchor, WINDOW_STATE_SLOT, external.into());
    crate::v8_finalizer::track_context_owned_v8_finalizer(scope, anchor, move || drop(owner));
    data_with_parts(scope, base_url, Some(anchor), initiator)
}
