use super::navigation_window::child_browsing_context_handle_for_runtime_owner;
use super::*;

/// A deferred iframe attribute navigation still belongs to the initial empty
/// Document. Reloading that Document must leave the pending attribute load
/// alone; an ordinary initial about:blank iframe has an active history entry
/// and can be reloaded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NavigationReloadAdmission {
    PendingInitialAttributeNavigation,
    Admitted,
}

pub(super) fn navigation_reload_admission<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> NavigationReloadAdmission {
    let Some(handle) = child_browsing_context_handle_for_runtime_owner(scope, owner) else {
        return NavigationReloadAdmission::Admitted;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return NavigationReloadAdmission::Admitted;
    };
    if unsafe { &*host_ptr }.child_initial_empty_has_pending_attribute_navigation(handle) {
        NavigationReloadAdmission::PendingInitialAttributeNavigation
    } else {
        NavigationReloadAdmission::Admitted
    }
}
