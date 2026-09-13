use anyhow::{Result, anyhow};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use crate::{util::throw_type_error, web_api_interfaces, webidl};

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Scheduling)]
struct SchedulingObjectDeclaration {}

#[derive(Default, WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Scheduling, enumerable)]
struct SchedulingPrototypeDeclaration {
    #[webapi(method, length = 0, callback = is_input_pending_callback)]
    is_input_pending: (),
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "IsInputPendingOptions")]
struct IsInputPendingOptions {
    #[webidl(name = "includeContinuous", default = false)]
    _include_continuous: bool,
}

pub(super) fn install_scheduling_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    if interface_name == "Scheduling" {
        SchedulingPrototypeDeclaration::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        );
    }
}

pub(super) fn build_scheduling_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Object>> {
    SchedulingObjectDeclaration::new()
        .bind(scope)
        .map_err(|error| anyhow!("failed to bind Scheduling object: {error}"))
}

fn is_input_pending_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !web_api_interfaces::Scheduling::is_instance(scope, args.this()) {
        throw_type_error(scope, "Illegal invocation");
        return;
    }
    // Even the idle compatibility profile must perform dictionary conversion:
    // includeContinuous getters and their exceptions are observable WebIDL.
    if let Err(error) = webidl::parse_dictionary::<IsInputPendingOptions>(
        scope,
        args.get(0),
        webidl::Context::argument("Scheduling.isInputPending", 1),
    ) {
        webidl::throw_error(scope, &error);
        return;
    }
    // Capability shim: pending-input prediction is not connected to the owner
    // queue yet. Report the idle state, not a measurement of queued CDP input.
    rv.set_bool(false);
}
