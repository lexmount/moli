use super::*;
use moli_webapi_declare::WebApiFunctionTemplate;
use moli_webidl as webidl;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SubmitEvent, enumerable)]
struct SubmitEventMethodsDeclaration {
    #[webapi(accessor_property = "agentInvoked", getter = submit_event_agent_invoked_getter_function)]
    agent_invoked: (),
    #[webapi(method = "respondWith", length = 1, callback = submit_event_respond_with_callback)]
    respond_with: (),
}

pub fn install_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    SubmitEventMethodsDeclaration::initialize_prototype_template(scope, prototype);
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SubmitEvent.respondWith")]
struct RespondWithArgs<'s> {
    #[webidl(required)]
    response: v8::Local<'s, v8::Promise>,
}

fn submit_event_agent_invoked_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(
        event_private_value(scope, args.this(), SUBMIT_EVENT_AGENT_INVOKED_SLOT)
            .is_some_and(|value| value.is_true()),
    );
}

fn submit_event_respond_with_callback<'s>(
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
        crate::embedding::throw_dom_exception_value(
            scope,
            "respondWith requires an agent-invoked submit event during dispatch after preventDefault()",
            "InvalidStateError",
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

pub fn mark_agent_submit_event(scope: &mut v8::PinScope<'_, '_>, event: v8::Local<'_, v8::Object>) {
    set_event_private_value(
        scope,
        event,
        SUBMIT_EVENT_AGENT_INVOKED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
}

pub fn take_submit_event_response<'s>(
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
