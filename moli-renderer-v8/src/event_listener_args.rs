//! The shared EventTarget argument boundary, before target-specific mutation.

use crate::abort_signal_route::{ResolvedAbortSignal, event_listener_signal_from_options_value};
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "EventTarget.addEventListener")]
pub(crate) struct AddEventListenerArgs<'s> {
    #[webidl(required, name = "type")]
    pub(crate) event_type: String,
    #[webidl(required, converter = "callback_interface", nullable)]
    pub(crate) listener: Option<webidl::WebIdlCallbackInterface>,
    #[webidl(with = add_event_listener_options)]
    pub(crate) options: AddEventListenerOptions<'s>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "EventTarget.removeEventListener")]
pub(crate) struct RemoveEventListenerArgs {
    #[webidl(required, name = "type")]
    pub(crate) event_type: String,
    #[webidl(required, converter = "callback_interface", nullable)]
    pub(crate) listener: Option<webidl::WebIdlCallbackInterface>,
    #[webidl(with = webidl::event_listener_options)]
    pub(crate) options: webidl::EventListenerOptions,
}

pub(crate) struct AddEventListenerOptions<'s> {
    pub(crate) options: webidl::EventListenerOptions,
    pub(crate) signal: Option<ResolvedAbortSignal<'s>>,
}

fn add_event_listener_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<AddEventListenerOptions<'s>, webidl::WebIdlError> {
    // Inherited members come first, followed by this dictionary's members in
    // lexical order. Finish conversion even when the callback is null.
    let value = args.get(index);
    let options = webidl::add_event_listener_options_value(scope, value)?;
    let signal = event_listener_signal_from_options_value(scope, value)?;
    Ok(AddEventListenerOptions { options, signal })
}
