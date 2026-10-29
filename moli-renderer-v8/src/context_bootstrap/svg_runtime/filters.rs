use moli_webapi_declare::WebApiFunctionTemplate;

use super::SVG_URI_HREF_SLOT;
use super::callbacks::{
    svg_animated_string_attribute_getter, svg_filter_primitive_animated_length_getter,
    svg_fit_to_view_box_getter,
};
use crate::{
    native_bridge::node_runtime_and_handle_from_object_or_detached,
    util::{callback_data_index_value, callback_data_item},
    web_api_interfaces, webidl,
};

// A mixin's attributes are installed on each concrete interface. Generate its
// receiver checks there so, for example, a flood getter cannot accept a tile.
macro_rules! filter_primitive {
    ($declaration:ident, $interface:ident, {
        $(($field:ident, $name:literal, $getter:ident, $index:literal)),* $(,)?
    }) => {
        #[derive(WebApiFunctionTemplate)]
        #[webapi(interface = web_api_interfaces::$interface, enumerable, receiver)]
        struct $declaration {
            #[webapi(accessor_property = "x", getter = animated_length_getter, data = callback_data_index_value(scope, 0))]
            x: (),
            #[webapi(accessor_property = "y", getter = animated_length_getter, data = callback_data_index_value(scope, 1))]
            y: (),
            #[webapi(accessor_property = "width", getter = animated_length_getter, data = callback_data_index_value(scope, 2))]
            width: (),
            #[webapi(accessor_property = "height", getter = animated_length_getter, data = callback_data_index_value(scope, 3))]
            height: (),
            #[webapi(accessor_property = "result", getter = animated_string_getter, data = callback_data_index_value(scope, 0))]
            result: (),
            $(
                #[webapi(accessor_property = $name, getter = $getter, data = callback_data_index_value(scope, $index))]
                $field: (),
            )*
        }
    };
}

filter_primitive!(ComponentTransferDeclaration, SVGFEComponentTransferElement, {
    (in1, "in1", animated_string_getter, 1),
});
filter_primitive!(FloodDeclaration, SVGFEFloodElement, {});
filter_primitive!(ImageDeclaration, SVGFEImageElement, {
    (href, "href", animated_string_getter, 2),
    (preserve_aspect_ratio, "preserveAspectRatio", preserve_aspect_ratio_getter, 1),
});
filter_primitive!(MergeDeclaration, SVGFEMergeElement, {});
filter_primitive!(TileDeclaration, SVGFETileElement, {
    (in1, "in1", animated_string_getter, 1),
});

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGFEMergeNodeElement, enumerable, receiver)]
struct MergeNodeDeclaration {
    #[webapi(accessor_property = "in1", getter = animated_string_getter, data = callback_data_index_value(scope, 1))]
    in1: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGFEDropShadowElement, enumerable, receiver)]
struct DropShadowMethodsDeclaration {
    #[webapi(method = "setStdDeviation", length = 2, callback = set_std_deviation)]
    set_std_deviation: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGFEGaussianBlurElement, enumerable, receiver)]
struct GaussianBlurMethodsDeclaration {
    #[webapi(method = "setStdDeviation", length = 2, callback = set_std_deviation)]
    set_std_deviation: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "setStdDeviation")]
struct SetStdDeviationArgs {
    #[webidl(required, converter = "raw")]
    x: webidl::Float,
    #[webidl(required, converter = "raw")]
    y: webidl::Float,
}

fn set_std_deviation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SetStdDeviationArgs>(scope, &args) else {
        return;
    };
    // Both float conversions complete before a single native mutation. The
    // f64 text view preserves the exact f32 values for existing live tear-offs.
    let value = format!("{} {}", f64::from(parsed.x.0), f64::from(parsed.y.0));
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let _ = runtime.set_attribute(scope, runtime_ptr, handle, "stdDeviation", &value);
}

pub(super) fn install_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "SVGFEComponentTransferElement" => {
            ComponentTransferDeclaration::initialize_prototype_template(scope, prototype)
        }
        "SVGFEFloodElement" => FloodDeclaration::initialize_prototype_template(scope, prototype),
        "SVGFEDropShadowElement" => {
            DropShadowMethodsDeclaration::initialize_prototype_template(scope, prototype);
        }
        "SVGFEGaussianBlurElement" => {
            GaussianBlurMethodsDeclaration::initialize_prototype_template(scope, prototype);
        }
        "SVGFEImageElement" => ImageDeclaration::initialize_prototype_template(scope, prototype),
        "SVGFEMergeElement" => MergeDeclaration::initialize_prototype_template(scope, prototype),
        "SVGFEMergeNodeElement" => {
            MergeNodeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "SVGFETileElement" => TileDeclaration::initialize_prototype_template(scope, prototype),
        _ => {}
    }
}

fn animated_length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    // Tear-off objects belong to the element's realm, including when the
    // accessor was obtained from a different Window.
    let Some(context) = args.this().get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    svg_filter_primitive_animated_length_getter(scope, args, rv);
}

fn animated_string_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((slot, attribute)) = callback_data_item(
        scope,
        &args,
        &[
            ("__moliSvgFilterResult", "result"),
            ("__moliSvgFilterInput", "in"),
            (SVG_URI_HREF_SLOT, "href"),
        ],
        "SVG filter string attributes",
    ) else {
        return;
    };
    let Some(context) = args.this().get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    svg_animated_string_attribute_getter(scope, args, rv, slot, attribute);
}

fn preserve_aspect_ratio_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(context) = args.this().get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    svg_fit_to_view_box_getter(scope, args, rv);
}
