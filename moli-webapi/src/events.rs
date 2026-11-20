//! Event dictionaries and payloads independent of DOM dispatch and scheduling.
//!
//! State lives in V8's heap. The embedder supplies its realm clock and wrapper
//! projection, so these bindings do not own Documents, listeners or native hosts.

use std::rc::Rc;

pub mod audio;
pub mod device;
pub mod dispatch_init;
pub mod fallback;
pub mod fetch;
pub mod ice_error_event;
pub mod init;
pub mod kind;
pub mod legacy;
pub mod modifiers;
pub mod progress;
pub mod service_worker;
pub mod speech;
pub mod state;
pub mod subclasses;
pub mod task_priority_change_event;
pub mod ui;
pub mod value;
pub mod webrtc;
pub mod webrtc_payload;

use init::EventInit;
use state::{
    define_event_property, event_private_value, mark_event_trusted, new_event_state,
    set_event_private_value,
};

pub fn event_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        moli_v8_util::throw_type_error(
            scope,
            "Failed to construct 'Event': Please use the 'new' operator.",
        );
        return;
    }
    let Some(event_type) = init::event_type_argument(scope, &args, "Event") else {
        return;
    };
    let Some(init) = init::parse_event_dictionary(scope, &args, "Event") else {
        return;
    };
    if initialize_event_from_init(scope, args.this(), event_type, init).is_some() {
        rv.set(args.this().into());
    }
}

/// Build Event state in the selected realm. The dispatch owner sets trust.
pub fn construct_native_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    init: EventInit,
) -> Option<v8::Local<'s, v8::Object>> {
    let event_type = moli_v8_util::v8_string(scope, event_type)?;
    construct_native_event_with_type(scope, event_type, init)
}

pub fn construct_native_event_with_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: v8::Local<'s, v8::String>,
    init: EventInit,
) -> Option<v8::Local<'s, v8::Object>> {
    let wrapper = moli_v8_util::new_intrinsic_interface_instance(scope, "Event")?;
    initialize_event_from_init(scope, wrapper, event_type, init)?;
    Some(wrapper)
}

fn initialize_event_from_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    event_type: v8::Local<'s, v8::String>,
    init: EventInit,
) -> Option<()> {
    let event = new_event_state(scope);
    initialize_event_object_with_type(scope, event, event_type, init.bubbles, init.cancelable);
    define_event_property(
        scope,
        event,
        "composed",
        v8::Boolean::new(scope, init.composed).into(),
    );
    initialize_event_wrapper(scope, wrapper, event)
}

/// Materialize browser-created event state through the same intrinsic and
/// wrapper capability used by public event constructors.
pub fn new_event_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let state = state::event_backing(scope, state);
    let interface = moli_webapi_declare::web_api_object_type(scope, state)?.name();
    let prototype = moli_v8_util::global_constructor_prototype(scope, interface)?;
    let wrapper = v8::Object::new(scope);
    if wrapper.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    initialize_event_wrapper(scope, wrapper, state)?;
    Some(wrapper)
}

/// Capabilities needed when constructing an Event in a particular realm.
/// Install before exposing event constructors. Callbacks execute in the callee
/// realm and must not retain the supplied local handles.
pub struct EventRuntime {
    /// Returns this realm's DOM high resolution timestamp.
    pub timestamp: for<'s, 'i> fn(&mut v8::PinScope<'s, 'i>) -> f64,
    /// Applies the embedder's native Window predicate, including retained globals.
    pub is_window_receiver:
        for<'s, 'i> fn(&mut v8::PinScope<'s, 'i>, v8::Local<'s, v8::Object>) -> bool,
    /// Binds the author's wrapper to private state and the embedder's world cache.
    pub initialize_wrapper: for<'s, 'i> fn(
        &mut v8::PinScope<'s, 'i>,
        v8::Local<'s, v8::Object>,
        v8::Local<'s, v8::Object>,
    ) -> Option<()>,
}

pub fn install_runtime(scope: &mut v8::PinScope<'_, '_>, runtime: EventRuntime) {
    scope.get_current_context().set_slot(Rc::new(runtime));
}

fn runtime(scope: &mut v8::PinScope<'_, '_>) -> Rc<EventRuntime> {
    scope
        .get_current_context()
        .get_slot::<EventRuntime>()
        .expect("install the Event runtime before invoking event bindings")
        .clone()
}

fn initialize_event_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Object>,
) -> Option<()> {
    (runtime(scope).initialize_wrapper)(scope, wrapper, state)
}

fn initialize_event_object_with_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    event_type: v8::Local<'s, v8::String>,
    bubbles: bool,
    cancelable: bool,
) {
    let timestamp = (runtime(scope).timestamp)(scope);
    state::initialize_event_object_with_type(
        scope, event, event_type, bubbles, cancelable, timestamp,
    );
}

fn initialize_event_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    event_type: &str,
    bubbles: bool,
    cancelable: bool,
) {
    let event_type = moli_v8_util::v8_string(scope, event_type).expect("event type value");
    initialize_event_object_with_type(scope, event, event_type, bubbles, cancelable);
}

pub mod idb_version_change_event;
pub mod media_query_list_event;
