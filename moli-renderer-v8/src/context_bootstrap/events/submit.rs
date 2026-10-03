use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SubmitEvent.respondWith")]
struct RespondWithArgs<'s> {
    #[webidl(required)]
    response: v8::Local<'s, v8::Promise>,
}

pub(in crate::context_bootstrap) fn submit_event_agent_invoked_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(
        event_private_value(scope, args.this(), SUBMIT_EVENT_AGENT_INVOKED_SLOT)
            .is_some_and(|value| value.is_true()),
    );
}

pub(in crate::context_bootstrap) fn submit_event_respond_with_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<RespondWithArgs<'s>>(scope, &args) else {
        return;
    };
    let event = args.this();
    let agent_invoked = event_private_value(scope, event, SUBMIT_EVENT_AGENT_INVOKED_SLOT)
        .is_some_and(|value| value.is_true());
    if !agent_invoked
        || !event_bool_attribute(scope, event, "defaultPrevented")
        || !event_is_dispatching(scope, event)
    {
        crate::native_bridge::throw_dom_exception(
            scope,
            "InvalidStateError",
            11,
            "respondWith requires an agent-invoked submit event during dispatch after preventDefault()",
        );
        return;
    }
    set_event_private_value(
        scope,
        event,
        SUBMIT_EVENT_RESPONSE_SLOT,
        parsed.response.into(),
    );
}

pub(crate) fn mark_agent_submit_event(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<'_, v8::Object>,
) {
    set_event_private_value(
        scope,
        event,
        SUBMIT_EVENT_AGENT_INVOKED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
}

pub(crate) fn take_submit_event_response<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let response = event_private_value(scope, event, SUBMIT_EVENT_RESPONSE_SLOT)
        .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok());
    set_event_private_value(
        scope,
        event,
        SUBMIT_EVENT_RESPONSE_SLOT,
        v8::undefined(scope).into(),
    );
    response
}
