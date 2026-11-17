//! Event dictionaries and payloads independent of DOM dispatch and scheduling.
//!
//! State lives in V8's heap. The embedder supplies its realm clock and wrapper
//! projection, so these bindings do not own Documents, listeners or native hosts.

use std::rc::Rc;

pub mod audio;
pub mod device;
pub mod ice_error_event;
pub mod init;
pub mod kind;
pub mod speech;
pub mod state;
pub mod subclasses;
pub mod value;
pub mod webrtc;

use init::EventInit;
use state::{
    define_event_property, event_private_value, mark_event_trusted, new_event_state,
    set_event_private_value,
};

/// Capabilities needed when constructing an Event in a particular realm.
/// Install before exposing event constructors. Callbacks execute in the callee
/// realm and must not retain the supplied local handles.
pub struct EventRuntime {
    /// Returns this realm's DOM high resolution timestamp.
    pub timestamp: for<'s, 'i> fn(&mut v8::PinScope<'s, 'i>) -> f64,
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
