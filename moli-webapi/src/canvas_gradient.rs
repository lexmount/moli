use crate::interfaces as web_api_interfaces;
use moli_v8_util::global_constructor_prototype;
use moli_webapi_declare::WebApiFunctionTemplate;
use moli_webidl as webidl;
#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CanvasGradient, enumerable)]
struct CanvasGradientTemplateMethodsDeclaration {
    #[webapi(
        method = "addColorStop",
        length = 2,
        callback = canvas_gradient_add_color_stop_callback
    )]
    add_color_stop: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CanvasGradient.addColorStop")]
struct CanvasGradientAddColorStopArgs {
    #[webidl(required)]
    offset: f64,
    #[webidl(required)]
    _color: String,
}

pub fn canvas_gradient_add_color_stop_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<CanvasGradientAddColorStopArgs>(scope, &args) else {
        return;
    };
    if !(0.0..=1.0).contains(&parsed.offset) {
        webidl::throw_index_size_error(scope);
        return;
    }
    rv.set_undefined();
}

pub fn build<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    let gradient = v8::Object::new(scope);
    if let Some(prototype) = global_constructor_prototype(scope, "CanvasGradient") {
        let _ = gradient.set_prototype(scope, prototype.into());
    }
    gradient
}
pub fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    CanvasGradientTemplateMethodsDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}
