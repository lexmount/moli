use super::*;
use moli_webapi_declare::initialize_web_api_constructor_receiver;

const CONSTRAINT_SLOT: &str = "__moliOverconstrainedErrorConstraint";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "OverconstrainedError")]
struct ConstructorArgs {
    #[webidl(required, converter = "raw")]
    constraint: webidl::DomString16,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    message: webidl::DomString16,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::OverconstrainedError)]
struct ObjectDeclaration<'s> {
    #[webapi(slot = CONSTRAINT_SLOT)]
    constraint: v8::Local<'s, v8::String>,
    #[webapi(slot = DOM_EXCEPTION_MESSAGE_SLOT)]
    message: v8::Local<'s, v8::String>,
    #[webapi(slot = DOM_EXCEPTION_NAME_SLOT, init = string("OverconstrainedError"))]
    name: (),
    #[webapi(slot = DOM_EXCEPTION_CODE_SLOT, init = 0)]
    code: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::OverconstrainedError, receiver, enumerable)]
struct PrototypeDeclaration {
    #[webapi(accessor_property, getter = constraint_getter)]
    constraint: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    PrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

pub(crate) fn overconstrained_error_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "OverconstrainedError constructor requires 'new'.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<ConstructorArgs>(scope, &args) else {
        return;
    };
    if !initialize_web_api_constructor_receiver(scope, args.this(), "OverconstrainedError") {
        return;
    }
    let Some(constraint) =
        v8::String::new_from_two_byte(scope, &parsed.constraint.0, v8::NewStringType::Normal)
    else {
        return;
    };
    let Some(message) =
        v8::String::new_from_two_byte(scope, &parsed.message.0, v8::NewStringType::Normal)
    else {
        return;
    };
    if ObjectDeclaration::new(constraint, message)
        .initialize(scope, args.this())
        .is_ok()
    {
        capture_dom_exception_stack(scope, args.this());
        rv.set(args.this().into());
    }
}

fn constraint_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("OverconstrainedError receiver was validated");
    if let Some(value) = get_private_value(scope, receiver, CONSTRAINT_SLOT) {
        rv.set(value);
    }
}
