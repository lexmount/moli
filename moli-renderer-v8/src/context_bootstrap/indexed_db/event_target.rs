use super::*;
use crate::context_bootstrap::{
    install_simple_event_target_ordered_handlers, mark_simple_event_target_slot,
};
use crate::util::set_private_value;

mod dispatch;
mod handlers;
mod version_change;

pub(crate) use self::dispatch::dispatch_indexed_db_script_event;
pub(super) use self::dispatch::*;
pub(super) use self::handlers::*;
pub(in crate::context_bootstrap) use self::version_change::*;

const EVENT_PARENT_SLOT: &str = "moli.IndexedDb.EventParent";

pub(in crate::context_bootstrap::indexed_db) fn initialize_indexed_db_event_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    parent: Option<v8::Local<'s, v8::Object>>,
) {
    mark_simple_event_target_slot(scope, target, INDEXED_DB_EVENT_LISTENERS_SLOT);
    install_simple_event_target_ordered_handlers(scope, target);
    if let Some(parent) = parent {
        // The event parent survives transaction completion but is traced only
        // while its JS wrapper remains reachable, independently of public db.
        set_private_value(scope, target, EVENT_PARENT_SLOT, parent.into());
    }
}
