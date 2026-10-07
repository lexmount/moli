//! TimeEvent instances are produced by the native SVG timeline. The interface
//! has no public constructor and is not a Document.createEvent legacy alias.

use super::{
    define_event_property, initialize_event_object, mark_event_trusted, new_event_state,
    new_event_wrapper, reinitialize_event_object,
};
use crate::{web_api_interfaces, webidl};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TimeEvent.initTimeEvent")]
struct InitTimeEventArgs<'s> {
    #[webidl(required, converter = "raw")]
    event_type: webidl::DomString16,
    #[webidl(nullable, interface = web_api_interfaces::Window,
        brand_check = super::super::is_window_receiver)]
    view: Option<v8::Local<'s, v8::Object>>,
    #[webidl(default = 0)]
    detail: i32,
}

pub(in crate::context_bootstrap) fn time_event_init_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<InitTimeEventArgs>(scope, &args) else {
        return;
    };
    let event = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated TimeEvent receiver validation");
    if !reinitialize_event_object(scope, event, &parsed.event_type.0, false, false) {
        return;
    }
    let view = parsed
        .view
        .map_or_else(|| v8::null(scope).into(), Into::into);
    define_event_property(scope, event, "view", view);
    define_event_property(
        scope,
        event,
        "detail",
        v8::Integer::new(scope, parsed.detail).into(),
    );
}

pub(crate) fn construct_svg_time_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: moli_svg::SvgAnimationEventKind,
) -> Option<v8::Local<'s, v8::Object>> {
    let state = new_event_state(scope);
    initialize_event_object(scope, state, kind.name(), false, false);
    let view = scope.get_current_context().global(scope);
    define_event_property(scope, state, "view", view.into());
    let detail = match kind {
        moli_svg::SvgAnimationEventKind::Repeat(iteration) => {
            // TimeEvent.detail is a WebIDL long, including its modulo conversion.
            let number = v8::Number::new(scope, iteration);
            webidl::convert::<webidl::Long>(
                scope,
                number.into(),
                webidl::Context::member("TimeEvent", "detail"),
            )
            .ok()?
            .0
        }
        _ => 0,
    };
    define_event_property(
        scope,
        state,
        "detail",
        v8::Integer::new(scope, detail).into(),
    );
    web_api_interfaces::TimeEvent::DESCRIPTOR
        .initialize(scope, state)
        .ok()?;
    let event = new_event_wrapper(scope, state)?;
    mark_event_trusted(scope, event);
    Some(event)
}
