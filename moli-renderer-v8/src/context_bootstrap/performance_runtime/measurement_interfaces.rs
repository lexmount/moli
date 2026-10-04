//! Prototype surfaces for measurement interfaces whose producers are not available yet.
//! Keep native receiver validation and reject access without fabricated measurements.

use moli_webapi_declare::WebApiFunctionTemplate;

use crate::{native_bridge::throw_dom_exception, web_api_interfaces};

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::LargestContentfulPaint, enumerable, receiver)]
struct LargestContentfulPaintPrototypeDeclaration {
    #[webapi(accessor_property = "loadTime", getter = measurement_unavailable)]
    load_time: (),
    #[webapi(accessor_property = "renderTime", getter = measurement_unavailable)]
    render_time: (),
    #[webapi(accessor_property = "size", getter = measurement_unavailable)]
    size: (),
    #[webapi(accessor_property = "id", getter = measurement_unavailable)]
    id: (),
    #[webapi(accessor_property = "url", getter = measurement_unavailable)]
    url: (),
    #[webapi(accessor_property = "element", getter = measurement_unavailable)]
    element: (),
    #[webapi(method = "toJSON", length = 0, callback = measurement_unavailable)]
    to_json: (),
    #[webapi(accessor_property = "paintTime", getter = measurement_unavailable)]
    paint_time: (),
    #[webapi(accessor_property = "presentationTime", getter = measurement_unavailable)]
    presentation_time: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PerformanceEventTiming, enumerable, receiver)]
struct PerformanceEventTimingPrototypeDeclaration {
    #[webapi(accessor_property = "processingStart", getter = measurement_unavailable)]
    processing_start: (),
    #[webapi(accessor_property = "processingEnd", getter = measurement_unavailable)]
    processing_end: (),
    #[webapi(accessor_property = "cancelable", getter = measurement_unavailable)]
    cancelable: (),
    #[webapi(accessor_property = "target", getter = measurement_unavailable)]
    target: (),
    #[webapi(accessor_property = "targetSelector", getter = measurement_unavailable)]
    target_selector: (),
    #[webapi(accessor_property = "interactionId", getter = measurement_unavailable)]
    interaction_id: (),
    #[webapi(method = "toJSON", length = 0, callback = measurement_unavailable)]
    to_json: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PerformancePaintTiming, enumerable, receiver)]
struct PerformancePaintTimingPrototypeDeclaration {
    #[webapi(method = "toJSON", length = 0, callback = measurement_unavailable)]
    to_json: (),
    #[webapi(accessor_property = "paintTime", getter = measurement_unavailable)]
    paint_time: (),
    #[webapi(accessor_property = "presentationTime", getter = measurement_unavailable)]
    presentation_time: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PerformanceServerTiming, enumerable, receiver)]
struct PerformanceServerTimingPrototypeDeclaration {
    #[webapi(accessor_property = "name", getter = measurement_unavailable)]
    name: (),
    #[webapi(accessor_property = "duration", getter = measurement_unavailable)]
    duration: (),
    #[webapi(accessor_property = "description", getter = measurement_unavailable)]
    description: (),
    #[webapi(method = "toJSON", length = 0, callback = measurement_unavailable)]
    to_json: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "LargestContentfulPaint" => {
            LargestContentfulPaintPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "PerformanceEventTiming" => {
            PerformanceEventTimingPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "PerformancePaintTiming" => {
            PerformancePaintTimingPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "PerformanceServerTiming" => {
            PerformanceServerTimingPrototypeDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}

fn measurement_unavailable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    throw_dom_exception(
        scope,
        "NotSupportedError",
        9,
        "Performance measurements for this interface are not implemented.",
    );
}
