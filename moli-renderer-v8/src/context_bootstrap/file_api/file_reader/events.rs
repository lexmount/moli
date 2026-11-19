use super::*;
use moli_webapi::events::progress::new_file_reader_progress_event;

pub(in crate::context_bootstrap::file_api::file_reader) fn dispatch_file_reader_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
    event_type: &str,
    loaded: f64,
    total: f64,
) {
    let event = new_file_reader_progress_event(scope, reader, event_type, loaded, total);
    let _ = dispatch_simple_event_target_event(scope, reader, event_type, event);
}
