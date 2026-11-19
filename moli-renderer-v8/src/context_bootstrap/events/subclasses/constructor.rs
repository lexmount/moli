use super::*;

// InputEvent retains native live Range snapshots. Other event constructors are shared.
fn event_subclass_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(kind) = EventSubclassKind::from_value(args.data()) else {
        throw_type_error(scope, "Invalid event subclass constructor.");
        return;
    };
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            &format!(
                "Failed to construct '{}': Please use the 'new' operator.",
                kind.constructor_name()
            ),
        );
        return;
    }
    if kind.required_arguments() == 2 && args.length() < 2 {
        throw_type_error(
            scope,
            &format!(
                "Failed to construct '{}': 2 arguments required, but only {} present.",
                kind.constructor_name(),
                args.length()
            ),
        );
        return;
    }
    let wrapper = args.this();
    let Some(event_type) = event_type_argument(scope, &args, kind.constructor_name()) else {
        return;
    };
    let init = match kind {
        EventSubclassKind::InputEvent => parse_event_dictionary::<
            moli_webapi::events::subclasses::InputEventInit,
        >(scope, &args, "InputEvent"),
        _ => unreachable!("value event templates are constructed in moli-webapi"),
    };
    let Some(init) = init else {
        return;
    };
    if let Some(wrapper) = moli_webapi::events::subclasses::construct_subclass(
        scope,
        wrapper,
        kind,
        event_type,
        init.values.event_flags(),
        |scope, event| input::initialize_input_event(scope, event, init),
    ) {
        rv.set(wrapper.into());
    }
}

pub(in crate::context_bootstrap) fn build_event_subclass_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    kind: EventSubclassKind,
) -> v8::Local<'s, v8::FunctionTemplate> {
    if let Some(template) = moli_webapi::events::subclasses::build_template(scope, kind) {
        return template;
    }
    let data: v8::Local<'s, v8::Value> = v8::Integer::new(scope, kind as i32).into();
    v8::FunctionTemplate::builder(event_subclass_constructor_callback)
        .data(data)
        .length(kind.required_arguments())
        .build(scope)
}
