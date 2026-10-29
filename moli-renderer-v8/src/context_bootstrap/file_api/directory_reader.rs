//! Typed `FileSystemDirectoryReader.readEntries()` state and callback task.
//!
//! The reader wrapper owns enumeration state across calls. One admitted
//! request owns its exact callback pair and result until the HTML file-reading
//! source selects it. Offset/done mutation therefore occurs at selected-task
//! execution, not while the binding merely queues work.

use crate::{
    native_bridge::JsContextHost,
    page_task_queue::{RendererPageFileReadingTaskId, RendererPageFileReadingTaskKind},
    util::{context_host_ptr_from_global_bridge, get_private_value, set_private_value},
    webidl,
    window_webidl_callback::WindowWebIdlCallbackFunction,
};
use moli_webidl_callback::WebIdlCallbackFunction;

use super::data_transfer::{
    FILE_SYSTEM_DIRECTORY_READER_ACTIVE_REQUEST_SLOT, FILE_SYSTEM_DIRECTORY_READER_DONE_SLOT,
    FILE_SYSTEM_DIRECTORY_READER_ENTRIES_SLOT, FILE_SYSTEM_DIRECTORY_READER_ERROR_SLOT,
    FILE_SYSTEM_DIRECTORY_READER_OFFSET_SLOT,
};

const FILE_SYSTEM_DIRECTORY_READER_BATCH_SIZE: u32 = 100;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FileSystemDirectoryReader.readEntries")]
struct FileSystemDirectoryReaderReadEntriesArgs {
    #[webidl(required, converter = "callback_function")]
    success_callback: webidl::WebIdlCallbackFunction,
    #[webidl(converter = "callback_function")]
    error_callback: Option<webidl::WebIdlCallbackFunction>,
}

/// Result of admission after both Web IDL callback conversions have
/// completed. `NoErrorCallback` matches Chromium's observable behavior for an
/// overlapping read without the optional callback: keep the active request
/// untouched and publish no artificial empty task.
pub(crate) enum DirectoryReaderCallbackAdmission {
    Task {
        kind: RendererPageFileReadingTaskKind,
        task: DirectoryReaderCallbackTask,
    },
    NoErrorCallback,
}

/// State effect produced only after an exact reader task reaches its body.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DirectoryReaderCallbackTaskEffect {
    CallbackInvoked,
    CallbackNotInvoked,
    StaleReaderRequest,
}

enum DirectoryReaderCallback {
    Success(WindowWebIdlCallbackFunction),
    Error(WindowWebIdlCallbackFunction),
}

/// Result owned by one admitted reader request.
///
/// A nonempty final batch does not mark the reader done. As required by the
/// Entries algorithm, the following read produces the first empty batch and
/// only that matching task transitions `done` to true.
enum DirectoryReaderRequestOutcome {
    Batch {
        entries: Vec<v8::Global<v8::Value>>,
        next_offset: u32,
        marks_done: bool,
    },
    TerminalEmpty,
    OverlappingInvalidState,
    TerminalError(v8::Global<v8::Value>),
}

/// One-shot callback and exact reader transition retained until FileReading
/// selected execution.
pub(crate) struct DirectoryReaderCallbackTask {
    reader: v8::Global<v8::Object>,
    callback: DirectoryReaderCallback,
    outcome: DirectoryReaderRequestOutcome,
    request_id: RendererPageFileReadingTaskId,
}

impl DirectoryReaderCallbackTask {
    pub(crate) fn admit<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        host: &JsContextHost,
        reader: v8::Local<'s, v8::Object>,
        success_callback: WebIdlCallbackFunction,
        error_callback: Option<WebIdlCallbackFunction>,
        request_id: RendererPageFileReadingTaskId,
    ) -> DirectoryReaderCallbackAdmission {
        if reader_active_request(scope, reader).is_some() {
            let Some(error_callback) = error_callback else {
                return DirectoryReaderCallbackAdmission::NoErrorCallback;
            };
            return DirectoryReaderCallbackAdmission::Task {
                kind: RendererPageFileReadingTaskKind::DirectoryOverlappingReadError,
                task: Self {
                    reader: v8::Global::new(scope, reader),
                    callback: DirectoryReaderCallback::Error(WindowWebIdlCallbackFunction::new(
                        scope,
                        host,
                        error_callback,
                    )),
                    outcome: DirectoryReaderRequestOutcome::OverlappingInvalidState,
                    request_id,
                },
            };
        }

        if let Some(error) = reader_terminal_error(scope, reader) {
            let Some(error_callback) = error_callback else {
                return DirectoryReaderCallbackAdmission::NoErrorCallback;
            };
            return DirectoryReaderCallbackAdmission::Task {
                kind: RendererPageFileReadingTaskKind::DirectoryTerminalError,
                task: Self {
                    reader: v8::Global::new(scope, reader),
                    callback: DirectoryReaderCallback::Error(WindowWebIdlCallbackFunction::new(
                        scope,
                        host,
                        error_callback,
                    )),
                    outcome: DirectoryReaderRequestOutcome::TerminalError(v8::Global::new(
                        scope, error,
                    )),
                    request_id,
                },
            };
        }

        let callback = DirectoryReaderCallback::Success(WindowWebIdlCallbackFunction::new(
            scope,
            host,
            success_callback,
        ));
        if reader_is_done(scope, reader) {
            return DirectoryReaderCallbackAdmission::Task {
                kind: RendererPageFileReadingTaskKind::DirectoryTerminalEmpty,
                task: Self {
                    reader: v8::Global::new(scope, reader),
                    callback,
                    outcome: DirectoryReaderRequestOutcome::TerminalEmpty,
                    request_id,
                },
            };
        }

        let entries = reader_entries(scope, reader);
        let start = reader_offset(scope, reader);
        let length = entries.map_or(0, |entries| entries.length());
        let end = start
            .saturating_add(FILE_SYSTEM_DIRECTORY_READER_BATCH_SIZE)
            .min(length);
        let mut batch = Vec::with_capacity(end.saturating_sub(start) as usize);
        if let Some(entries) = entries {
            for index in start..end {
                if let Some(entry) = entries.get_index(scope, index) {
                    batch.push(v8::Global::new(scope, entry));
                }
            }
        }
        set_reader_active_request(scope, reader, Some(request_id));
        DirectoryReaderCallbackAdmission::Task {
            kind: RendererPageFileReadingTaskKind::DirectoryBatch,
            task: Self {
                reader: v8::Global::new(scope, reader),
                callback,
                outcome: DirectoryReaderRequestOutcome::Batch {
                    entries: batch,
                    next_offset: end,
                    marks_done: start >= length,
                },
                request_id,
            },
        }
    }

    /// Undo only the active request that this task installed. Publication
    /// failure must not consume entries or clear a later request.
    pub(crate) fn rollback_admission(&self, scope: &mut v8::PinScope<'_, '_>) {
        if !matches!(self.outcome, DirectoryReaderRequestOutcome::Batch { .. }) {
            return;
        }
        let reader = v8::Local::new(scope, &self.reader);
        if reader_active_request(scope, reader) == Some(self.request_id) {
            set_reader_active_request(scope, reader, None);
        }
    }

    /// Apply the exact reader transition and invoke the selected callback.
    ///
    /// The caller has already authorized the task's Page/Window/Document.
    /// Callback-Realm currentness remains independent and is checked before
    /// any callback argument is materialized in that Realm.
    pub(crate) fn invoke(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
    ) -> DirectoryReaderCallbackTaskEffect {
        let reader = v8::Local::new(scope, &self.reader);
        if let DirectoryReaderRequestOutcome::Batch {
            next_offset,
            marks_done,
            ..
        } = &self.outcome
        {
            if reader_active_request(scope, reader) != Some(self.request_id) {
                return DirectoryReaderCallbackTaskEffect::StaleReaderRequest;
            }
            set_reader_offset(scope, reader, *next_offset);
            set_reader_done(scope, reader, *marks_done);
            set_reader_active_request(scope, reader, None);
        }

        let callback = match &self.callback {
            DirectoryReaderCallback::Success(callback)
            | DirectoryReaderCallback::Error(callback) => callback,
        };
        let Some(identity) = callback.relevant_identity() else {
            return DirectoryReaderCallbackTaskEffect::CallbackNotInvoked;
        };
        if !unsafe { &*host_ptr }.window_execution_context_identity_is_current(identity) {
            return DirectoryReaderCallbackTaskEffect::CallbackNotInvoked;
        }
        let Some(relevant_context) = callback.relevant_context(scope) else {
            return DirectoryReaderCallbackTaskEffect::CallbackNotInvoked;
        };
        let scope = &mut v8::ContextScope::new(scope, relevant_context);
        let Some(callback) = callback.prepare_execution(scope, host_ptr, None) else {
            return DirectoryReaderCallbackTaskEffect::CallbackNotInvoked;
        };
        let argument = match &self.outcome {
            DirectoryReaderRequestOutcome::Batch { entries, .. } => {
                let array = v8::Array::new(scope, entries.len() as i32);
                for (index, entry) in entries.iter().enumerate() {
                    let entry = v8::Local::new(scope, entry);
                    let _ = array.set_index(scope, index as u32, entry);
                }
                array.into()
            }
            DirectoryReaderRequestOutcome::TerminalEmpty => v8::Array::new(scope, 0).into(),
            DirectoryReaderRequestOutcome::OverlappingInvalidState => {
                crate::context_bootstrap::new_dom_exception_value(
                    scope,
                    "A directory read is already in progress.",
                    "InvalidStateError",
                )
            }
            DirectoryReaderRequestOutcome::TerminalError(error) => v8::Local::new(scope, error),
        };
        let receiver = v8::undefined(scope);
        if callback.invoke_and_report(
            scope,
            "filesystemdirectoryreader",
            "FileSystemDirectoryReader.readEntries callback",
            receiver.into(),
            &[argument],
        ) {
            DirectoryReaderCallbackTaskEffect::CallbackInvoked
        } else {
            DirectoryReaderCallbackTaskEffect::CallbackNotInvoked
        }
    }
}

fn reader_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_value(scope, reader, FILE_SYSTEM_DIRECTORY_READER_ENTRIES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

fn reader_offset<'s>(scope: &mut v8::PinScope<'s, '_>, reader: v8::Local<'s, v8::Object>) -> u32 {
    get_private_value(scope, reader, FILE_SYSTEM_DIRECTORY_READER_OFFSET_SLOT)
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value as u32)
        .unwrap_or(0)
}

fn set_reader_offset<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
    offset: u32,
) {
    let offset = v8::Number::new(scope, f64::from(offset));
    set_private_value(
        scope,
        reader,
        FILE_SYSTEM_DIRECTORY_READER_OFFSET_SLOT,
        offset.into(),
    );
}

fn reader_active_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
) -> Option<RendererPageFileReadingTaskId> {
    let value = get_private_value(
        scope,
        reader,
        FILE_SYSTEM_DIRECTORY_READER_ACTIVE_REQUEST_SLOT,
    )?;
    let value = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (raw, lossless) = value.u64_value();
    lossless.then(|| RendererPageFileReadingTaskId::from_raw(raw))
}

fn set_reader_active_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
    request_id: Option<RendererPageFileReadingTaskId>,
) {
    let value = request_id
        .map(|request_id| v8::BigInt::new_from_u64(scope, request_id.as_u64()).into())
        .unwrap_or_else(|| v8::null(scope).into());
    set_private_value(
        scope,
        reader,
        FILE_SYSTEM_DIRECTORY_READER_ACTIVE_REQUEST_SLOT,
        value,
    );
}

fn reader_is_done<'s>(scope: &mut v8::PinScope<'s, '_>, reader: v8::Local<'s, v8::Object>) -> bool {
    get_private_value(scope, reader, FILE_SYSTEM_DIRECTORY_READER_DONE_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

fn set_reader_done<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
    done: bool,
) {
    let done = v8::Boolean::new(scope, done);
    set_private_value(
        scope,
        reader,
        FILE_SYSTEM_DIRECTORY_READER_DONE_SLOT,
        done.into(),
    );
}

fn reader_terminal_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reader: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, reader, FILE_SYSTEM_DIRECTORY_READER_ERROR_SLOT)
        .filter(|value| !value.is_null_or_undefined())
}

pub(crate) fn file_system_directory_reader_read_entries_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    // Conversion order is observable and must precede reader-state
    // inspection or mutation.
    let Some(parsed) = webidl::parse_args::<FileSystemDirectoryReaderReadEntriesArgs>(scope, &args)
    else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let _ = unsafe { &mut *host_ptr }.queue_directory_reader_callback_task(
        scope,
        args.this(),
        parsed.success_callback,
        parsed.error_callback,
    );
}
