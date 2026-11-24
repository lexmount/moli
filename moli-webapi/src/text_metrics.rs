use crate::interfaces as web_api_interfaces;
use moli_v8_util::{get_private_value, throw_type_error};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
const TEXT_METRICS_WIDTH_SLOT: &str = "__moliTextMetricsWidth";
const TEXT_METRICS_ASCENT_SLOT: &str = "__moliTextMetricsActualBoundingBoxAscent";
const TEXT_METRICS_DESCENT_SLOT: &str = "__moliTextMetricsActualBoundingBoxDescent";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::TextMetrics)]
pub struct CanvasTextMetricsDeclaration {
    #[webapi(slot = TEXT_METRICS_WIDTH_SLOT)]
    pub width: f64,
    #[webapi(slot = TEXT_METRICS_ASCENT_SLOT)]
    pub actual_bounding_box_ascent: f64,
    #[webapi(slot = TEXT_METRICS_DESCENT_SLOT)]
    pub actual_bounding_box_descent: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TextMetrics, enumerable)]
struct TextMetricsPrototypeDeclaration {
    #[webapi(accessor_property, getter = text_metrics_width_getter)]
    width: (),
    #[webapi(accessor_property = "actualBoundingBoxAscent", getter = text_metrics_ascent_getter)]
    actual_bounding_box_ascent: (),
    #[webapi(accessor_property = "actualBoundingBoxDescent", getter = text_metrics_descent_getter)]
    actual_bounding_box_descent: (),
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
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    text_metric(scope, args.this(), rv, "width", TEXT_METRICS_WIDTH_SLOT);
}

fn text_metrics_ascent_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    text_metric(
        scope,
        args.this(),
        rv,
        "actualBoundingBoxAscent",
        TEXT_METRICS_ASCENT_SLOT,
    );
}

fn text_metrics_descent_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    text_metric(
        scope,
        args.this(),
        rv,
        "actualBoundingBoxDescent",
        TEXT_METRICS_DESCENT_SLOT,
    );
}

fn text_metric<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    name: &str,
    slot: &'static str,
) {
    if let Some(value) = get_private_value(scope, receiver, slot) {
        rv.set(value);
    } else {
        throw_type_error(
            scope,
            &format!("TextMetrics.{name} called on incompatible receiver."),
        );
    }
}
