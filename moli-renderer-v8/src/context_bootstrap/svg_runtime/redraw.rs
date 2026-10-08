//! SVG2 defines these deprecated redraw methods to have no effect. Native
//! receiver validation and WebIDL argument conversion still apply.
use crate::{web_api_interfaces, webidl};
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGSVGElement, enumerable, receiver)]
struct RedrawMethodsDeclaration {
    #[webapi(method = "suspendRedraw", length = 1, callback = suspend_redraw)]
    suspend_redraw: (),
    #[webapi(method = "unsuspendRedraw", length = 1, callback = unsuspend_redraw)]
    unsuspend_redraw: (),
    #[webapi(method = "unsuspendRedrawAll", length = 0, callback = no_redraw)]
    unsuspend_redraw_all: (),
    #[webapi(method = "forceRedraw", length = 0, callback = no_redraw)]
    force_redraw: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVGSVGElement.suspendRedraw")]
struct SuspendRedrawArgs {
    #[webidl(required, converter = "unsigned_long")]
    _max_wait_milliseconds: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVGSVGElement.unsuspendRedraw")]
struct UnsuspendRedrawArgs {
    #[webidl(required, converter = "unsigned_long")]
    _suspend_handle_id: u32,
}

fn suspend_redraw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if webidl::parse_args::<SuspendRedrawArgs>(scope, &args).is_none() {
        return;
    }
    rv.set_int32(1);
}

fn unsuspend_redraw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let _ = webidl::parse_args::<UnsuspendRedrawArgs>(scope, &args);
}

fn no_redraw(
    _scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    RedrawMethodsDeclaration::initialize_prototype_template(scope, prototype);
}
