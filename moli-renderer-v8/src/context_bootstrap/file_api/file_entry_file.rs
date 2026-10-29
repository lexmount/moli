//! Typed `FileSystemFileEntry.file()` callback task.
//!
//! The Entries API converts the required success callback and then its
//! optional, non-nullable error callback before starting asynchronous work.
//! Moli's FileEntry already owns an immutable in-memory File, so the
//! admitted task captures that File and the success callback immediately.
//! The existing DOM-manipulation source supplies asynchronous FIFO ordering;
//! the selected Page-task dispatcher owns checkpoint and runtime follow-up.

use crate::{
    native_bridge::JsContextHost, util::context_host_ptr_from_global_bridge, webidl,
    window_webidl_callback::WindowWebIdlCallbackFunction,
};
use moli_webidl_callback::WebIdlCallbackFunction;

use super::data_transfer::file_system_file_entry_file_object;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FileSystemFileEntry.file")]
struct FileSystemFileEntryFileArgs {
    #[webidl(required, converter = "callback_function")]
    success_callback: webidl::WebIdlCallbackFunction,
    #[webidl(converter = "callback_function")]
    error_callback: Option<webidl::WebIdlCallbackFunction>,
}

/// Result produced only after an admitted callback task reaches its body.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FileEntryFileCallbackTaskEffect {
    CallbackInvoked,
    CallbackNotInvoked,
}

/// One-shot success callback plus the File captured by the entry algorithm.
///
/// The callback residence owns relevant/incumbent Web IDL contexts. The
/// enclosing DOM-manipulation ledger separately owns the exact calling
/// Window/Document. A callback from another Realm therefore does not change
/// which Document owns the browser task.
pub(crate) struct FileEntryFileCallbackTask {
    callback: WindowWebIdlCallbackFunction,
    file: v8::Global<v8::Object>,
}

impl FileEntryFileCallbackTask {
    pub(crate) fn new(
        scope: &mut v8::PinScope<'_, '_>,
        host: &JsContextHost,
        callback: WebIdlCallbackFunction,
        file: v8::Local<'_, v8::Object>,
    ) -> Self {
        Self {
            callback: WindowWebIdlCallbackFunction::new(scope, host, callback),
            file: v8::Global::new(scope, file),
        }
    }

    pub(crate) fn invoke(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
    ) -> FileEntryFileCallbackTaskEffect {
        let Some(callback) = self.callback.prepare_execution(scope, host_ptr, None) else {
            return FileEntryFileCallbackTaskEffect::CallbackNotInvoked;
        };
        let file = v8::Local::new(scope, &self.file);
        let receiver = v8::undefined(scope);
        if callback.invoke_and_report(
            scope,
            "filesystemfileentry",
            "FileSystemFileEntry.file callback",
            receiver.into(),
            &[file.into()],
        ) {
            FileEntryFileCallbackTaskEffect::CallbackInvoked
        } else {
            FileEntryFileCallbackTaskEffect::CallbackNotInvoked
        }
    }
}

pub(crate) fn file_system_file_entry_file_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    // Web IDL conversion order is observable. Convert both callback arguments
    // before inspecting the entry or publishing any task.
    let Some(parsed) = webidl::parse_args::<FileSystemFileEntryFileArgs>(scope, &args) else {
        return;
    };
    let _converted_error_callback = parsed.error_callback;
    let Some(file) = file_system_file_entry_file_object(scope, args.this()) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let _ = unsafe { &mut *host_ptr }.queue_file_entry_file_callback_task(
        scope,
        parsed.success_callback,
        file,
    );
}
