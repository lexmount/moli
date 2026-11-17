use super::super::super::events;
use super::*;
pub(in crate::context_bootstrap) use moli_webapi::events::idb_version_change_event::idb_version_change_event_constructor_callback;
pub(in crate::context_bootstrap::indexed_db) use moli_webapi::events::idb_version_change_event::install_version_change_event_template_bindings;
pub(in crate::context_bootstrap::indexed_db) fn dispatch_version_change_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event_type: &str,
    old_version: u64,
    new_version: Option<u64>,
) -> bool {
    dispatch_version_change_event_with_result(scope, target, event_type, old_version, new_version)
        .uncanceled
}

pub(in crate::context_bootstrap::indexed_db) fn dispatch_version_change_event_with_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    event_type: &str,
    old_version: u64,
    new_version: Option<u64>,
) -> IdbEventDispatchResult {
    let state = moli_webapi::events::idb_version_change_event::new_state(
        scope,
        event_type,
        old_version,
        new_version,
    );
    let Some(event) = events::new_event_wrapper(scope, state) else {
        return IdbEventDispatchResult {
            uncanceled: true,
            did_listeners_throw: false,
        };
    };
    events::mark_event_trusted(scope, event);
    dispatch_idb_event_object_with_result(scope, target, event)
}
