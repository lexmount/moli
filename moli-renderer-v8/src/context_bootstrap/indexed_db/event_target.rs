use super::*;
use crate::context_bootstrap::initialize_simple_event_target;
use crate::util::set_private_value;

mod dispatch;
mod target;
mod version_change;

pub(super) use self::dispatch::*;
pub(crate) use self::target::dispatch_indexed_db_event_callback;
pub(in crate::context_bootstrap) use self::version_change::*;

const EVENT_PARENT_SLOT: &str = "moli.IndexedDb.EventParent";

pub(in crate::context_bootstrap::indexed_db) fn initialize_indexed_db_event_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    parent: Option<v8::Local<'s, v8::Object>>,
) {
    initialize_simple_event_target(scope, target);
    if let Some(parent) = parent {
        // The event parent survives transaction completion but is traced only
        // while its JS wrapper remains reachable, independently of public db.
        set_private_value(scope, target, EVENT_PARENT_SLOT, parent.into());
    }
}
