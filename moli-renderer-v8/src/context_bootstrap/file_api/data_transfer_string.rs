//! Typed `DataTransferItem.getAsString` callback task.
//!
//! HTML leaves this method's task source underspecified. Chromium posts from
//! the calling `ScriptState` to its user-interaction runner, so admission
//! freezes that exact Window/Document in the existing UserInteraction source.
//! This module owns only Web IDL conversion, the captured string, and callback
//! invocation. DataTransfer eligibility, Page scheduling, exact-target
//! authorization, checkpointing, and runtime follow-up stay with their
//! existing owners.

use crate::{
    host::report_event_callback_exception,
    native_bridge::JsContextHost,
    util::{context_host_ptr_from_global_bridge, v8_string},
    webidl,
    window_webidl_callback::{
        WindowWebIdlCallbackFunction, WindowWebIdlCallbackFunctionOutcome,
        invoke_window_webidl_callback_function,
    },
};
use moli_webidl_callback::WebIdlCallbackFunction;

use super::data_transfer::{item_is_readable, item_kind, item_string_value};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DataTransferItem.getAsString")]
struct DataTransferItemGetAsStringArgs {
    #[webidl(required, nullable, converter = "callback_function")]
    callback: Option<webidl::WebIdlCallbackFunction>,
}

/// Result produced only after an admitted callback task reaches its body.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DataTransferStringCallbackTaskEffect {
    CallbackInvoked,
    CallbackNotInvoked,
}

/// One-shot callback plus the string captured by the DataTransfer algorithm.
///
/// The callback residence owns relevant/incumbent Web IDL contexts. The
/// enclosing UserInteraction ledger separately owns the exact target
/// Window/Document selected by the calling Realm.
pub(crate) struct DataTransferStringCallbackTask {
    callback: WindowWebIdlCallbackFunction,
    value: String,
}

impl DataTransferStringCallbackTask {
    pub(crate) fn new(
        scope: &mut v8::PinScope<'_, '_>,
        host: &JsContextHost,
        callback: WebIdlCallbackFunction,
        value: String,
    ) -> Self {
        Self {
            callback: WindowWebIdlCallbackFunction::new(scope, host, callback),
            value,
        }
    }

    pub(crate) fn invoke(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
    ) -> DataTransferStringCallbackTaskEffect {
        let callback = self.callback.prepare(scope);
        let relevant_identity = callback.relevant_identity();
        let Some(value) = v8_string(scope, &self.value) else {
            return DataTransferStringCallbackTaskEffect::CallbackNotInvoked;
        };
        let receiver = v8::undefined(scope);
        match invoke_window_webidl_callback_function(
            scope,
            host_ptr,
            "FunctionStringCallback",
            "DataTransferItem.getAsString callback threw",
            "DataTransferItem.getAsString callback",
            &callback,
            receiver.into(),
            &[value.into()],
        ) {
            WindowWebIdlCallbackFunctionOutcome::Returned => {
                DataTransferStringCallbackTaskEffect::CallbackInvoked
            }
            WindowWebIdlCallbackFunctionOutcome::Threw(report) => {
                report_event_callback_exception(
                    scope,
                    host_ptr,
                    "datatransferitem",
                    relevant_identity,
                    None,
                    &report,
                );
                DataTransferStringCallbackTaskEffect::CallbackInvoked
            }
            WindowWebIdlCallbackFunctionOutcome::Retired => {
                DataTransferStringCallbackTaskEffect::CallbackNotInvoked
            }
        }
    }
}

pub(crate) fn data_transfer_item_get_as_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DataTransferItemGetAsStringArgs>(scope, &args) else {
        return;
    };
    let Some(callback) = parsed.callback else {
        return;
    };
    if !item_is_readable(scope, args.this())
        || item_kind(scope, args.this()).as_deref() != Some("string")
    {
        return;
    }
    let Some(value) = item_string_value(scope, args.this()) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let _ =
        unsafe { &mut *host_ptr }.queue_data_transfer_string_callback_task(scope, callback, value);
}
