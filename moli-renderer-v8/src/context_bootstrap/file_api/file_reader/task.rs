use super::*;
use crate::web_api_interfaces;

pub(crate) fn flush_one_pending_file_reader(scope: &mut v8::PinScope<'_, '_>) -> bool {
    let Some(reader) = pop_first_object_from_global_queue(scope, FILE_READER_QUEUE_SLOT) else {
        return false;
    };
    set_file_reader_scheduled(scope, reader, false);
    let read_id = file_reader_read_id(scope, reader);
    let phase = file_reader_task_phase(scope, reader);
    if phase < 3.0 && file_reader_ready_state(scope, reader) != 1.0 {
        return true;
    }
    match phase as u32 {
        0 => {
            let total = file_reader_pending_total(scope, reader);
            dispatch_file_reader_event(scope, reader, "loadstart", 0.0, total);
            schedule_next_file_reader_phase(scope, reader, read_id, 1.0);
        }
        1 => {
            if file_reader_pending_result(scope, reader).is_none_or(|value| value.is_undefined()) {
                // The native source will resume this phase when bytes arrive.
                return true;
            }
            let total = file_reader_pending_total(scope, reader);
            if total > 0.0 && file_reader_pending_error(scope, reader).is_none() {
                dispatch_file_reader_event(scope, reader, "progress", total, total);
            }
            schedule_next_file_reader_phase(scope, reader, read_id, 2.0);
        }
        2 => {
            let total = file_reader_pending_total(scope, reader);
            let pending =
                file_reader_pending_result(scope, reader).unwrap_or_else(|| v8::null(scope).into());
            set_file_reader_ready_state(scope, reader, 2.0);
            if let Some(error) = file_reader_pending_error(scope, reader) {
                set_file_reader_error(scope, reader, error);
                set_file_reader_result(scope, reader, v8::null(scope).into());
                dispatch_file_reader_event(scope, reader, "error", 0.0, total);
            } else {
                set_file_reader_result(scope, reader, pending);
                dispatch_file_reader_event(scope, reader, "load", total, total);
            }
            schedule_next_file_reader_phase(scope, reader, read_id, 3.0);
        }
        3 => {
            let total = file_reader_pending_total(scope, reader);
            let loaded = if file_reader_pending_error(scope, reader).is_some() {
                0.0
            } else {
                total
            };
            dispatch_file_reader_event(scope, reader, "loadend", loaded, total);
        }
        _ => {}
    }
    true
}

fn file_reader_pending_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let value = file_reader_pending_result(scope, reader)?;
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    web_api_interfaces::DOMException::is_instance(scope, object).then_some(value)
}

pub(super) fn schedule_next_file_reader_phase<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
    read_id: f64,
    phase: f64,
) {
    if file_reader_read_id(scope, reader) != read_id {
        return;
    }
    set_file_reader_task_phase(scope, reader, phase);
    if file_reader_scheduled(scope, reader) {
        return;
    }
    set_file_reader_scheduled(scope, reader, true);
    push_object_to_global_queue(scope, FILE_READER_QUEUE_SLOT, reader);
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &mut *host_ptr };
        schedule_host_callback(scope, host, file_reader_flush_callback);
    }
}

pub(in crate::context_bootstrap::file_api::file_reader) fn file_reader_flush_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let _ = flush_one_pending_file_reader(scope);
}
