use crate::interfaces as web_api_interfaces;
use moli_v8_util::{get_private_value, throw_type_error};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
const TEXT_METRICS_WIDTH_SLOT: &str = "__moliTextMetricsWidth";
#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::TextMetrics)]
pub struct CanvasTextMetricsDeclaration {
    #[webapi(slot = TEXT_METRICS_WIDTH_SLOT)]
    pub width: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TextMetrics, enumerable)]
struct TextMetricsPrototypeDeclaration {
    #[webapi(accessor_property, getter = text_metrics_width_getter)]
    width: (),
}

pub fn install_text_metrics_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    TextMetricsPrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

fn text_metrics_width_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(width) = get_private_value(scope, args.this(), TEXT_METRICS_WIDTH_SLOT) else {
        throw_type_error(scope, "TextMetrics.width called on incompatible receiver.");
        return;
    };
    rv.set(width);
}
