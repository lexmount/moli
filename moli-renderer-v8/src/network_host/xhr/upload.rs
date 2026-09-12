use super::events::xhr_dispatch_upload_progress_events;
use super::*;

pub(crate) fn apply_xhr_upload_event(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    internal_id: u64,
    event: moli_fetch::UploadEvent,
) -> bool {
    let is_current = |scope: &mut v8::PinScope<'_, '_>| {
        xhr_state_number_property(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT)
            == Some(internal_id as f64)
            && xhr_state_bool_property(scope, xhr, XHR_SEND_FLAG_SLOT).unwrap_or(false)
            && !events::xhr_is_aborted(scope, xhr)
    };
    if !is_current(scope) {
        return false;
    }
    if !xhr_state_bool_property(scope, xhr, XHR_UPLOAD_IN_PROGRESS_SLOT).unwrap_or(false) {
        return true;
    }
    match event {
        moli_fetch::UploadEvent::Progress { loaded, total } => {
            let previous =
                xhr_state_number_property(scope, xhr, XHR_UPLOAD_LOADED_SLOT).unwrap_or(0.0);
            if loaded as f64 > previous {
                set_xhr_state_number(scope, xhr, XHR_UPLOAD_LOADED_SLOT, loaded as f64);
                xhr_dispatch_upload_progress_events(
                    scope,
                    xhr,
                    &["progress"],
                    loaded as f64,
                    total as f64,
                );
            }
        }
        moli_fetch::UploadEvent::Complete { loaded, total } => {
            // The end-of-body algorithm marks the upload complete before
            // firing events. No state writes follow the reentrant callbacks.
            set_xhr_state_bool(scope, xhr, XHR_UPLOAD_IN_PROGRESS_SLOT, false);
            xhr_dispatch_upload_progress_events(
                scope,
                xhr,
                &["progress", "load", "loadend"],
                loaded as f64,
                total as f64,
            );
        }
    }
    is_current(scope)
}

pub(super) fn dispatch_xhr_upload_error_if_in_progress(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    event: &str,
) {
    if !xhr_state_bool_property(scope, xhr, XHR_UPLOAD_IN_PROGRESS_SLOT).unwrap_or(false) {
        return;
    }
    set_xhr_state_bool(scope, xhr, XHR_UPLOAD_IN_PROGRESS_SLOT, false);
    xhr_dispatch_upload_progress_events(scope, xhr, &[event, "loadend"], 0.0, 0.0);
}
