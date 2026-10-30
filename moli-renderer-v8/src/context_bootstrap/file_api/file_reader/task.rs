use super::*;
use crate::page_task_queue::RendererPageFileReaderPhase;
use crate::web_api_interfaces;

pub(crate) struct FileReaderTask {
    reader: v8::Global<v8::Object>,
    read_id: f64,
    phase: RendererPageFileReaderPhase,
}

#[derive(Clone, Copy)]
pub(crate) enum FileReaderTaskEffect {
    Advanced,
    Stale,
}

impl FileReaderTask {
    pub(in crate::context_bootstrap::file_api) fn phase(&self) -> RendererPageFileReaderPhase {
        self.phase
    }

    pub(crate) fn new(
        scope: &v8::PinScope<'_, '_>,
        reader: v8::Local<'_, v8::Object>,
        read_id: f64,
        phase: RendererPageFileReaderPhase,
    ) -> Self {
        Self {
            reader: v8::Global::new(scope, reader),
            read_id,
            phase,
        }
    }

    pub(crate) fn invoke(self, scope: &mut v8::PinScope<'_, '_>) -> FileReaderTaskEffect {
        let reader = v8::Local::new(scope, &self.reader);
        if file_reader_read_id(scope, reader) != self.read_id
            || file_reader_ready_state(scope, reader) != 1.0
        {
            return FileReaderTaskEffect::Stale;
        }
        let total = file_reader_pending_total(scope, reader);
        let next = match self.phase {
            RendererPageFileReaderPhase::LoadStart => {
                set_file_reader_load_start_dispatched(scope, reader, true);
                dispatch_file_reader_event(scope, reader, "loadstart", 0.0, total);
                file_reader_pending_result(scope, reader)
                    .is_some_and(|value| !value.is_undefined())
                    .then_some(RendererPageFileReaderPhase::Progress)
            }
            RendererPageFileReaderPhase::Progress => {
                if total > 0.0 && file_reader_pending_error(scope, reader).is_none() {
                    dispatch_file_reader_event(scope, reader, "progress", total, total);
                }
                Some(RendererPageFileReaderPhase::Complete)
            }
            RendererPageFileReaderPhase::Complete => {
                let error = file_reader_pending_error(scope, reader);
                let result = file_reader_pending_result(scope, reader)
                    .unwrap_or_else(|| v8::null(scope).into());
                set_file_reader_pending_result(scope, reader, v8::null(scope).into());
                set_file_reader_ready_state(scope, reader, 2.0);
                let loaded = if let Some(error) = error {
                    set_file_reader_error(scope, reader, error);
                    set_file_reader_result(scope, reader, v8::null(scope).into());
                    dispatch_file_reader_event(scope, reader, "error", 0.0, total);
                    0.0
                } else {
                    set_file_reader_result(scope, reader, result);
                    dispatch_file_reader_event(scope, reader, "load", total, total);
                    total
                };
                // Both events belong to the completion task. A load handler
                // can start a new read, in which case this loadend is omitted.
                if file_reader_ready_state(scope, reader) != 1.0 {
                    dispatch_file_reader_event(scope, reader, "loadend", loaded, total);
                }
                None
            }
        };
        if let Some(next) = next
            && file_reader_read_id(scope, reader) == self.read_id
            && file_reader_ready_state(scope, reader) == 1.0
        {
            queue_file_reader_phase(scope, reader, self.read_id, next);
        }
        FileReaderTaskEffect::Advanced
    }
}

pub(in crate::context_bootstrap::file_api::file_reader) fn queue_file_reader_phase<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
    read_id: f64,
    phase: RendererPageFileReaderPhase,
) {
    let Some(context) = reader.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let task = FileReaderTask::new(scope, reader, read_id, phase);
    let _ = crate::context_bootstrap::queue_file_reading_task(
        scope,
        crate::context_bootstrap::FileReadingTask::Reader(task),
    );
}

fn file_reader_pending_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let value = file_reader_pending_result(scope, reader)?;
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    web_api_interfaces::DOMException::is_instance(scope, object).then_some(value)
}
