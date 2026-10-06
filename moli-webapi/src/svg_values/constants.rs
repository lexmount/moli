use crate::interfaces as web_api_interfaces;
use moli_webapi_declare::WebApiFunctionTemplate;

pub const SVG_PRESERVE_ASPECT_RATIO_UNKNOWN: u32 = 0;
pub const SVG_PRESERVE_ASPECT_RATIO_NONE: u32 = 1;
pub const SVG_PRESERVE_ASPECT_RATIO_X_MIN_Y_MIN: u32 = 2;
pub const SVG_PRESERVE_ASPECT_RATIO_X_MID_Y_MIN: u32 = 3;
pub const SVG_PRESERVE_ASPECT_RATIO_X_MAX_Y_MIN: u32 = 4;
pub const SVG_PRESERVE_ASPECT_RATIO_X_MIN_Y_MID: u32 = 5;
pub const SVG_PRESERVE_ASPECT_RATIO_X_MID_Y_MID: u32 = 6;
pub const SVG_PRESERVE_ASPECT_RATIO_X_MAX_Y_MID: u32 = 7;
pub const SVG_PRESERVE_ASPECT_RATIO_X_MIN_Y_MAX: u32 = 8;
pub const SVG_PRESERVE_ASPECT_RATIO_X_MID_Y_MAX: u32 = 9;
pub const SVG_PRESERVE_ASPECT_RATIO_X_MAX_Y_MAX: u32 = 10;
pub const SVG_MEET_OR_SLICE_UNKNOWN: u32 = 0;
pub const SVG_MEET_OR_SLICE_MEET: u32 = 1;
pub const SVG_MEET_OR_SLICE_SLICE: u32 = 2;
pub const SVG_UNIT_TYPE_UNKNOWN: u32 = 0;
pub const SVG_UNIT_TYPE_USER_SPACE_ON_USE: u32 = 1;
pub const SVG_UNIT_TYPE_OBJECT_BOUNDING_BOX: u32 = 2;
pub const SVG_SPREAD_METHOD_UNKNOWN: u32 = 0;
pub const SVG_SPREAD_METHOD_PAD: u32 = 1;
pub const SVG_SPREAD_METHOD_REFLECT: u32 = 2;
pub const SVG_SPREAD_METHOD_REPEAT: u32 = 3;
pub const SVG_COMPONENT_TRANSFER_TYPE_UNKNOWN: u32 = 0;
pub const SVG_COMPONENT_TRANSFER_TYPE_IDENTITY: u32 = 1;
pub const SVG_COMPONENT_TRANSFER_TYPE_TABLE: u32 = 2;
pub const SVG_COMPONENT_TRANSFER_TYPE_DISCRETE: u32 = 3;
pub const SVG_COMPONENT_TRANSFER_TYPE_LINEAR: u32 = 4;
pub const SVG_COMPONENT_TRANSFER_TYPE_GAMMA: u32 = 5;
pub const SVG_FE_BLEND_MODE_UNKNOWN: u32 = 0;
pub const SVG_FE_BLEND_MODE_NORMAL: u32 = 1;
pub const SVG_FE_BLEND_MODE_MULTIPLY: u32 = 2;
pub const SVG_FE_BLEND_MODE_SCREEN: u32 = 3;
pub const SVG_FE_BLEND_MODE_DARKEN: u32 = 4;
pub const SVG_FE_BLEND_MODE_LIGHTEN: u32 = 5;
pub const SVG_FE_BLEND_MODE_OVERLAY: u32 = 6;
pub const SVG_FE_BLEND_MODE_COLOR_DODGE: u32 = 7;
pub const SVG_FE_BLEND_MODE_COLOR_BURN: u32 = 8;
pub const SVG_FE_BLEND_MODE_HARD_LIGHT: u32 = 9;
pub const SVG_FE_BLEND_MODE_SOFT_LIGHT: u32 = 10;
pub const SVG_FE_BLEND_MODE_DIFFERENCE: u32 = 11;
pub const SVG_FE_BLEND_MODE_EXCLUSION: u32 = 12;
pub const SVG_FE_BLEND_MODE_HUE: u32 = 13;
pub const SVG_FE_BLEND_MODE_SATURATION: u32 = 14;
pub const SVG_FE_BLEND_MODE_COLOR: u32 = 15;
pub const SVG_FE_BLEND_MODE_LUMINOSITY: u32 = 16;
pub const SVG_FE_COLOR_MATRIX_TYPE_UNKNOWN: u32 = 0;
pub const SVG_FE_COLOR_MATRIX_TYPE_MATRIX: u32 = 1;
pub const SVG_FE_COLOR_MATRIX_TYPE_SATURATE: u32 = 2;
pub const SVG_FE_COLOR_MATRIX_TYPE_HUE_ROTATE: u32 = 3;
pub const SVG_FE_COLOR_MATRIX_TYPE_LUMINANCE_TO_ALPHA: u32 = 4;
pub const SVG_FE_COMPOSITE_OPERATOR_UNKNOWN: u32 = 0;
pub const SVG_FE_COMPOSITE_OPERATOR_OVER: u32 = 1;
pub const SVG_FE_COMPOSITE_OPERATOR_IN: u32 = 2;
pub const SVG_FE_COMPOSITE_OPERATOR_OUT: u32 = 3;
pub const SVG_FE_COMPOSITE_OPERATOR_ATOP: u32 = 4;
pub const SVG_FE_COMPOSITE_OPERATOR_XOR: u32 = 5;
pub const SVG_FE_COMPOSITE_OPERATOR_ARITHMETIC: u32 = 6;
pub const SVG_FE_COMPOSITE_OPERATOR_LIGHTER: u32 = 7;
pub const SVG_EDGE_MODE_UNKNOWN: u32 = 0;
pub const SVG_EDGE_MODE_DUPLICATE: u32 = 1;
pub const SVG_EDGE_MODE_WRAP: u32 = 2;
pub const SVG_EDGE_MODE_NONE: u32 = 3;
pub const SVG_CHANNEL_UNKNOWN: u32 = 0;
pub const SVG_CHANNEL_R: u32 = 1;
pub const SVG_CHANNEL_G: u32 = 2;
pub const SVG_CHANNEL_B: u32 = 3;
pub const SVG_CHANNEL_A: u32 = 4;
pub const SVG_MORPHOLOGY_OPERATOR_UNKNOWN: u32 = 0;
pub const SVG_MORPHOLOGY_OPERATOR_ERODE: u32 = 1;
pub const SVG_MORPHOLOGY_OPERATOR_DILATE: u32 = 2;
pub const SVG_TURBULENCE_TYPE_UNKNOWN: u32 = 0;
pub const SVG_TURBULENCE_TYPE_FRACTAL_NOISE: u32 = 1;
pub const SVG_TURBULENCE_TYPE_TURBULENCE: u32 = 2;
pub const SVG_STITCH_TYPE_UNKNOWN: u32 = 0;
pub const SVG_STITCH_TYPE_STITCH: u32 = 1;
pub const SVG_STITCH_TYPE_NO_STITCH: u32 = 2;
pub const SVG_TEXT_PATH_METHOD_TYPE_UNKNOWN: u32 = 0;
pub const SVG_TEXT_PATH_METHOD_TYPE_ALIGN: u32 = 1;
pub const SVG_TEXT_PATH_METHOD_TYPE_STRETCH: u32 = 2;
pub const SVG_TEXT_PATH_SPACING_TYPE_UNKNOWN: u32 = 0;
pub const SVG_TEXT_PATH_SPACING_TYPE_AUTO: u32 = 1;
pub const SVG_TEXT_PATH_SPACING_TYPE_EXACT: u32 = 2;
pub const SVG_TEXT_PATH_SIDE_TYPE_UNKNOWN: u32 = 0;
pub const SVG_TEXT_PATH_SIDE_TYPE_LEFT: u32 = 1;
pub const SVG_TEXT_PATH_SIDE_TYPE_RIGHT: u32 = 2;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGUnitTypes, enumerable)]
struct SvgUnitTypesTemplateConstantsDeclaration {
    #[webapi(constant = "SVG_UNIT_TYPE_UNKNOWN", value = SVG_UNIT_TYPE_UNKNOWN)]
    unknown: (),

    #[webapi(
        constant = "SVG_UNIT_TYPE_USERSPACEONUSE",
        value = SVG_UNIT_TYPE_USER_SPACE_ON_USE
    )]
    user_space_on_use: (),

    #[webapi(
        constant = "SVG_UNIT_TYPE_OBJECTBOUNDINGBOX",
        value = SVG_UNIT_TYPE_OBJECT_BOUNDING_BOX
    )]
    object_bounding_box: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGGradientElement, enumerable)]
struct SvgGradientElementTemplateConstantsDeclaration {
    #[webapi(
        constant = "SVG_SPREADMETHOD_UNKNOWN",
        value = SVG_SPREAD_METHOD_UNKNOWN
    )]
    spread_method_unknown: (),

    #[webapi(
        constant = "SVG_SPREADMETHOD_PAD",
        value = SVG_SPREAD_METHOD_PAD
    )]
    spread_method_pad: (),

    #[webapi(
        constant = "SVG_SPREADMETHOD_REFLECT",
        value = SVG_SPREAD_METHOD_REFLECT
    )]
    spread_method_reflect: (),

    #[webapi(
        constant = "SVG_SPREADMETHOD_REPEAT",
        value = SVG_SPREAD_METHOD_REPEAT
    )]
    spread_method_repeat: (),
}

macro_rules! define_svg_enumeration_constants {
    ($declaration:ident, $interface:literal, {
        $($field:ident => $constant:literal = $value:expr),+ $(,)?
    }) => {
        #[derive(WebApiFunctionTemplate)]
        #[webapi(name = $interface, enumerable)]
        pub struct $declaration {
            $(
                #[webapi(constant = $constant, value = $value)]
                $field: (),
            )+
        }
    };
}

define_svg_enumeration_constants!(
    SvgPreserveAspectRatioTemplateConstantsDeclaration,
    "SVGPreserveAspectRatio",
    {
        preserve_aspect_ratio_unknown => "SVG_PRESERVEASPECTRATIO_UNKNOWN" = SVG_PRESERVE_ASPECT_RATIO_UNKNOWN,
        preserve_aspect_ratio_none => "SVG_PRESERVEASPECTRATIO_NONE" = SVG_PRESERVE_ASPECT_RATIO_NONE,
        preserve_aspect_ratio_x_min_y_min => "SVG_PRESERVEASPECTRATIO_XMINYMIN" = SVG_PRESERVE_ASPECT_RATIO_X_MIN_Y_MIN,
        preserve_aspect_ratio_x_mid_y_min => "SVG_PRESERVEASPECTRATIO_XMIDYMIN" = SVG_PRESERVE_ASPECT_RATIO_X_MID_Y_MIN,
        preserve_aspect_ratio_x_max_y_min => "SVG_PRESERVEASPECTRATIO_XMAXYMIN" = SVG_PRESERVE_ASPECT_RATIO_X_MAX_Y_MIN,
        preserve_aspect_ratio_x_min_y_mid => "SVG_PRESERVEASPECTRATIO_XMINYMID" = SVG_PRESERVE_ASPECT_RATIO_X_MIN_Y_MID,
        preserve_aspect_ratio_x_mid_y_mid => "SVG_PRESERVEASPECTRATIO_XMIDYMID" = SVG_PRESERVE_ASPECT_RATIO_X_MID_Y_MID,
        preserve_aspect_ratio_x_max_y_mid => "SVG_PRESERVEASPECTRATIO_XMAXYMID" = SVG_PRESERVE_ASPECT_RATIO_X_MAX_Y_MID,
        preserve_aspect_ratio_x_min_y_max => "SVG_PRESERVEASPECTRATIO_XMINYMAX" = SVG_PRESERVE_ASPECT_RATIO_X_MIN_Y_MAX,
        preserve_aspect_ratio_x_mid_y_max => "SVG_PRESERVEASPECTRATIO_XMIDYMAX" = SVG_PRESERVE_ASPECT_RATIO_X_MID_Y_MAX,
        preserve_aspect_ratio_x_max_y_max => "SVG_PRESERVEASPECTRATIO_XMAXYMAX" = SVG_PRESERVE_ASPECT_RATIO_X_MAX_Y_MAX,
        meet_or_slice_unknown => "SVG_MEETORSLICE_UNKNOWN" = SVG_MEET_OR_SLICE_UNKNOWN,
        meet_or_slice_meet => "SVG_MEETORSLICE_MEET" = SVG_MEET_OR_SLICE_MEET,
        meet_or_slice_slice => "SVG_MEETORSLICE_SLICE" = SVG_MEET_OR_SLICE_SLICE,
    }
);

define_svg_enumeration_constants!(
    SvgComponentTransferFunctionElementTemplateConstantsDeclaration,
    "SVGComponentTransferFunctionElement",
    {
        unknown => "SVG_FECOMPONENTTRANSFER_TYPE_UNKNOWN" = SVG_COMPONENT_TRANSFER_TYPE_UNKNOWN,
        identity => "SVG_FECOMPONENTTRANSFER_TYPE_IDENTITY" = SVG_COMPONENT_TRANSFER_TYPE_IDENTITY,
        table => "SVG_FECOMPONENTTRANSFER_TYPE_TABLE" = SVG_COMPONENT_TRANSFER_TYPE_TABLE,
        discrete => "SVG_FECOMPONENTTRANSFER_TYPE_DISCRETE" = SVG_COMPONENT_TRANSFER_TYPE_DISCRETE,
        linear => "SVG_FECOMPONENTTRANSFER_TYPE_LINEAR" = SVG_COMPONENT_TRANSFER_TYPE_LINEAR,
        gamma => "SVG_FECOMPONENTTRANSFER_TYPE_GAMMA" = SVG_COMPONENT_TRANSFER_TYPE_GAMMA,
    }
);

define_svg_enumeration_constants!(
    SvgFeBlendElementTemplateConstantsDeclaration,
    "SVGFEBlendElement",
    {
        unknown => "SVG_FEBLEND_MODE_UNKNOWN" = SVG_FE_BLEND_MODE_UNKNOWN,
        normal => "SVG_FEBLEND_MODE_NORMAL" = SVG_FE_BLEND_MODE_NORMAL,
        multiply => "SVG_FEBLEND_MODE_MULTIPLY" = SVG_FE_BLEND_MODE_MULTIPLY,
        screen => "SVG_FEBLEND_MODE_SCREEN" = SVG_FE_BLEND_MODE_SCREEN,
        darken => "SVG_FEBLEND_MODE_DARKEN" = SVG_FE_BLEND_MODE_DARKEN,
        lighten => "SVG_FEBLEND_MODE_LIGHTEN" = SVG_FE_BLEND_MODE_LIGHTEN,
        overlay => "SVG_FEBLEND_MODE_OVERLAY" = SVG_FE_BLEND_MODE_OVERLAY,
        color_dodge => "SVG_FEBLEND_MODE_COLOR_DODGE" = SVG_FE_BLEND_MODE_COLOR_DODGE,
        color_burn => "SVG_FEBLEND_MODE_COLOR_BURN" = SVG_FE_BLEND_MODE_COLOR_BURN,
        hard_light => "SVG_FEBLEND_MODE_HARD_LIGHT" = SVG_FE_BLEND_MODE_HARD_LIGHT,
        soft_light => "SVG_FEBLEND_MODE_SOFT_LIGHT" = SVG_FE_BLEND_MODE_SOFT_LIGHT,
        difference => "SVG_FEBLEND_MODE_DIFFERENCE" = SVG_FE_BLEND_MODE_DIFFERENCE,
        exclusion => "SVG_FEBLEND_MODE_EXCLUSION" = SVG_FE_BLEND_MODE_EXCLUSION,
        hue => "SVG_FEBLEND_MODE_HUE" = SVG_FE_BLEND_MODE_HUE,
        saturation => "SVG_FEBLEND_MODE_SATURATION" = SVG_FE_BLEND_MODE_SATURATION,
        color => "SVG_FEBLEND_MODE_COLOR" = SVG_FE_BLEND_MODE_COLOR,
        luminosity => "SVG_FEBLEND_MODE_LUMINOSITY" = SVG_FE_BLEND_MODE_LUMINOSITY,
    }
);

define_svg_enumeration_constants!(
    SvgFeColorMatrixElementTemplateConstantsDeclaration,
    "SVGFEColorMatrixElement",
    {
        unknown => "SVG_FECOLORMATRIX_TYPE_UNKNOWN" = SVG_FE_COLOR_MATRIX_TYPE_UNKNOWN,
        matrix => "SVG_FECOLORMATRIX_TYPE_MATRIX" = SVG_FE_COLOR_MATRIX_TYPE_MATRIX,
        saturate => "SVG_FECOLORMATRIX_TYPE_SATURATE" = SVG_FE_COLOR_MATRIX_TYPE_SATURATE,
        hue_rotate => "SVG_FECOLORMATRIX_TYPE_HUEROTATE" = SVG_FE_COLOR_MATRIX_TYPE_HUE_ROTATE,
        luminance_to_alpha => "SVG_FECOLORMATRIX_TYPE_LUMINANCETOALPHA" = SVG_FE_COLOR_MATRIX_TYPE_LUMINANCE_TO_ALPHA,
    }
);

define_svg_enumeration_constants!(
    SvgFeCompositeElementTemplateConstantsDeclaration,
    "SVGFECompositeElement",
    {
        unknown => "SVG_FECOMPOSITE_OPERATOR_UNKNOWN" = SVG_FE_COMPOSITE_OPERATOR_UNKNOWN,
        over => "SVG_FECOMPOSITE_OPERATOR_OVER" = SVG_FE_COMPOSITE_OPERATOR_OVER,
        input => "SVG_FECOMPOSITE_OPERATOR_IN" = SVG_FE_COMPOSITE_OPERATOR_IN,
        out => "SVG_FECOMPOSITE_OPERATOR_OUT" = SVG_FE_COMPOSITE_OPERATOR_OUT,
        atop => "SVG_FECOMPOSITE_OPERATOR_ATOP" = SVG_FE_COMPOSITE_OPERATOR_ATOP,
        xor => "SVG_FECOMPOSITE_OPERATOR_XOR" = SVG_FE_COMPOSITE_OPERATOR_XOR,
        lighter => "SVG_FECOMPOSITE_OPERATOR_LIGHTER" = SVG_FE_COMPOSITE_OPERATOR_LIGHTER,
        arithmetic => "SVG_FECOMPOSITE_OPERATOR_ARITHMETIC" = SVG_FE_COMPOSITE_OPERATOR_ARITHMETIC,
    }
);

define_svg_enumeration_constants!(
    SvgFeConvolveMatrixElementTemplateConstantsDeclaration,
    "SVGFEConvolveMatrixElement",
    {
        unknown => "SVG_EDGEMODE_UNKNOWN" = SVG_EDGE_MODE_UNKNOWN,
        duplicate => "SVG_EDGEMODE_DUPLICATE" = SVG_EDGE_MODE_DUPLICATE,
        wrap => "SVG_EDGEMODE_WRAP" = SVG_EDGE_MODE_WRAP,
        none => "SVG_EDGEMODE_NONE" = SVG_EDGE_MODE_NONE,
    }
);

define_svg_enumeration_constants!(
    SvgFeGaussianBlurElementTemplateConstantsDeclaration,
    "SVGFEGaussianBlurElement",
    {
        unknown => "SVG_EDGEMODE_UNKNOWN" = SVG_EDGE_MODE_UNKNOWN,
        duplicate => "SVG_EDGEMODE_DUPLICATE" = SVG_EDGE_MODE_DUPLICATE,
        wrap => "SVG_EDGEMODE_WRAP" = SVG_EDGE_MODE_WRAP,
        none => "SVG_EDGEMODE_NONE" = SVG_EDGE_MODE_NONE,
    }
);

define_svg_enumeration_constants!(
    SvgFeDisplacementMapElementTemplateConstantsDeclaration,
    "SVGFEDisplacementMapElement",
    {
        unknown => "SVG_CHANNEL_UNKNOWN" = SVG_CHANNEL_UNKNOWN,
        red => "SVG_CHANNEL_R" = SVG_CHANNEL_R,
        green => "SVG_CHANNEL_G" = SVG_CHANNEL_G,
        blue => "SVG_CHANNEL_B" = SVG_CHANNEL_B,
        alpha => "SVG_CHANNEL_A" = SVG_CHANNEL_A,
    }
);

define_svg_enumeration_constants!(
    SvgFeMorphologyElementTemplateConstantsDeclaration,
    "SVGFEMorphologyElement",
    {
        unknown => "SVG_MORPHOLOGY_OPERATOR_UNKNOWN" = SVG_MORPHOLOGY_OPERATOR_UNKNOWN,
        erode => "SVG_MORPHOLOGY_OPERATOR_ERODE" = SVG_MORPHOLOGY_OPERATOR_ERODE,
        dilate => "SVG_MORPHOLOGY_OPERATOR_DILATE" = SVG_MORPHOLOGY_OPERATOR_DILATE,
    }
);

define_svg_enumeration_constants!(
    SvgFeTurbulenceElementTemplateConstantsDeclaration,
    "SVGFETurbulenceElement",
    {
        turbulence_unknown => "SVG_TURBULENCE_TYPE_UNKNOWN" = SVG_TURBULENCE_TYPE_UNKNOWN,
        fractal_noise => "SVG_TURBULENCE_TYPE_FRACTALNOISE" = SVG_TURBULENCE_TYPE_FRACTAL_NOISE,
        turbulence => "SVG_TURBULENCE_TYPE_TURBULENCE" = SVG_TURBULENCE_TYPE_TURBULENCE,
        stitch_unknown => "SVG_STITCHTYPE_UNKNOWN" = SVG_STITCH_TYPE_UNKNOWN,
        stitch => "SVG_STITCHTYPE_STITCH" = SVG_STITCH_TYPE_STITCH,
        no_stitch => "SVG_STITCHTYPE_NOSTITCH" = SVG_STITCH_TYPE_NO_STITCH,
    }
);

define_svg_enumeration_constants!(
    SvgTextPathElementTemplateConstantsDeclaration,
    "SVGTextPathElement",
    {
        method_unknown => "TEXTPATH_METHODTYPE_UNKNOWN" = SVG_TEXT_PATH_METHOD_TYPE_UNKNOWN,
        method_align => "TEXTPATH_METHODTYPE_ALIGN" = SVG_TEXT_PATH_METHOD_TYPE_ALIGN,
        method_stretch => "TEXTPATH_METHODTYPE_STRETCH" = SVG_TEXT_PATH_METHOD_TYPE_STRETCH,
        spacing_unknown => "TEXTPATH_SPACINGTYPE_UNKNOWN" = SVG_TEXT_PATH_SPACING_TYPE_UNKNOWN,
        spacing_auto => "TEXTPATH_SPACINGTYPE_AUTO" = SVG_TEXT_PATH_SPACING_TYPE_AUTO,
        spacing_exact => "TEXTPATH_SPACINGTYPE_EXACT" = SVG_TEXT_PATH_SPACING_TYPE_EXACT,
        side_unknown => "TEXTPATH_SIDETYPE_UNKNOWN" = SVG_TEXT_PATH_SIDE_TYPE_UNKNOWN,
        side_left => "TEXTPATH_SIDETYPE_LEFT" = SVG_TEXT_PATH_SIDE_TYPE_LEFT,
        side_right => "TEXTPATH_SIDETYPE_RIGHT" = SVG_TEXT_PATH_SIDE_TYPE_RIGHT,
    }
);

pub fn install_svg_unit_types_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    SvgUnitTypesTemplateConstantsDeclaration::initialize_template(scope, template);
    SvgUnitTypesTemplateConstantsDeclaration::initialize_prototype_template(scope, prototype);
}

pub fn install_svg_gradient_element_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    SvgGradientElementTemplateConstantsDeclaration::initialize_template(scope, template);
    SvgGradientElementTemplateConstantsDeclaration::initialize_prototype_template(scope, prototype);
}

pub fn install_svg_enumeration_constant_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    macro_rules! install_constants {
        ($declaration:ty) => {{
            <$declaration>::initialize_template(scope, template);
            <$declaration>::initialize_prototype_template(scope, prototype);
        }};
    }
    match interface_name {
        "SVGComponentTransferFunctionElement" => {
            install_constants!(SvgComponentTransferFunctionElementTemplateConstantsDeclaration);
        }
        "SVGFEBlendElement" => {
            install_constants!(SvgFeBlendElementTemplateConstantsDeclaration);
        }
        "SVGFEColorMatrixElement" => {
            install_constants!(SvgFeColorMatrixElementTemplateConstantsDeclaration);
        }
        "SVGFECompositeElement" => {
            install_constants!(SvgFeCompositeElementTemplateConstantsDeclaration);
        }
        "SVGFEConvolveMatrixElement" => {
            install_constants!(SvgFeConvolveMatrixElementTemplateConstantsDeclaration);
        }
        "SVGFEGaussianBlurElement" => {
            install_constants!(SvgFeGaussianBlurElementTemplateConstantsDeclaration);
        }
        "SVGFEDisplacementMapElement" => {
            install_constants!(SvgFeDisplacementMapElementTemplateConstantsDeclaration);
        }
        "SVGFEMorphologyElement" => {
            install_constants!(SvgFeMorphologyElementTemplateConstantsDeclaration);
        }
        "SVGFETurbulenceElement" => {
            install_constants!(SvgFeTurbulenceElementTemplateConstantsDeclaration);
        }
        "SVGTextPathElement" => {
            install_constants!(SvgTextPathElementTemplateConstantsDeclaration);
        }
        _ => {}
    }
}
