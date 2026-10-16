use crate::{
    native_bridge::throw_dom_exception,
    util::{get_private_value, set_private_value, throw_type_error},
    web_api_interfaces, webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const ID: &str = "__moliVttRegionId";
const WIDTH: &str = "__moliVttRegionWidth";
const LINES: &str = "__moliVttRegionLines";
const REGION_X: &str = "__moliVttRegionAnchorX";
const REGION_Y: &str = "__moliVttRegionAnchorY";
const VIEWPORT_X: &str = "__moliVttRegionViewportAnchorX";
const VIEWPORT_Y: &str = "__moliVttRegionViewportAnchorY";
const SCROLL: &str = "__moliVttRegionScroll";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::VTTRegion)]
struct RegionStateDeclaration {
    #[webapi(slot = ID, constructor_default = "")]
    id: &'static str,
    #[webapi(slot = WIDTH, constructor_default = 100.0)]
    width: f64,
    #[webapi(slot = LINES, constructor_default = 3u32)]
    lines: u32,
    #[webapi(slot = REGION_X, constructor_default = 0.0)]
    region_x: f64,
    #[webapi(slot = REGION_Y, constructor_default = 100.0)]
    region_y: f64,
    #[webapi(slot = VIEWPORT_X, constructor_default = 0.0)]
    viewport_x: f64,
    #[webapi(slot = VIEWPORT_Y, constructor_default = 100.0)]
    viewport_y: f64,
    #[webapi(slot = SCROLL, constructor_default = "")]
    scroll: &'static str,
}

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "ScrollSetting")]
enum ScrollSetting {
    #[webidl(token = "")]
    None,
    #[webidl(token = "up")]
    Up,
}

#[derive(Clone, Copy)]
enum Attribute {
    Id,
    Width,
    Lines,
    RegionX,
    RegionY,
    ViewportX,
    ViewportY,
    Scroll,
}

impl Attribute {
    fn from_callback(args: &v8::FunctionCallbackArguments<'_>) -> Self {
        let index = v8::Local::<v8::Integer>::try_from(args.data())
            .expect("VTTRegion attribute has native callback data")
            .value();
        match index {
            0 => Self::Id,
            1 => Self::Width,
            2 => Self::Lines,
            3 => Self::RegionX,
            4 => Self::RegionY,
            5 => Self::ViewportX,
            6 => Self::ViewportY,
            7 => Self::Scroll,
            _ => unreachable!("VTTRegion attribute index"),
        }
    }

    fn slot(self) -> &'static str {
        match self {
            Self::Id => ID,
            Self::Width => WIDTH,
            Self::Lines => LINES,
            Self::RegionX => REGION_X,
            Self::RegionY => REGION_Y,
            Self::ViewportX => VIEWPORT_X,
            Self::ViewportY => VIEWPORT_Y,
            Self::Scroll => SCROLL,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Width => "width",
            Self::Lines => "lines",
            Self::RegionX => "regionAnchorX",
            Self::RegionY => "regionAnchorY",
            Self::ViewportX => "viewportAnchorX",
            Self::ViewportY => "viewportAnchorY",
            Self::Scroll => "scroll",
        }
    }
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::VTTRegion, enumerable, receiver)]
struct RegionPrototypeDeclaration {
    #[webapi(accessor_property, getter = get_attribute, setter = set_attribute, data = v8::Integer::new(scope, 0), setter_data = v8::Integer::new(scope, 0))]
    id: (),
    #[webapi(accessor_property, getter = get_attribute, setter = set_attribute, data = v8::Integer::new(scope, 1), setter_data = v8::Integer::new(scope, 1))]
    width: (),
    #[webapi(accessor_property, getter = get_attribute, setter = set_attribute, data = v8::Integer::new(scope, 2), setter_data = v8::Integer::new(scope, 2))]
    lines: (),
    #[webapi(accessor_property = "regionAnchorX", getter = get_attribute, setter = set_attribute, data = v8::Integer::new(scope, 3), setter_data = v8::Integer::new(scope, 3))]
    region_anchor_x: (),
    #[webapi(accessor_property = "regionAnchorY", getter = get_attribute, setter = set_attribute, data = v8::Integer::new(scope, 4), setter_data = v8::Integer::new(scope, 4))]
    region_anchor_y: (),
    #[webapi(accessor_property = "viewportAnchorX", getter = get_attribute, setter = set_attribute, data = v8::Integer::new(scope, 5), setter_data = v8::Integer::new(scope, 5))]
    viewport_anchor_x: (),
    #[webapi(accessor_property = "viewportAnchorY", getter = get_attribute, setter = set_attribute, data = v8::Integer::new(scope, 6), setter_data = v8::Integer::new(scope, 6))]
    viewport_anchor_y: (),
    #[webapi(accessor_property, getter = get_attribute, setter = set_attribute, data = v8::Integer::new(scope, 7), setter_data = v8::Integer::new(scope, 7))]
    scroll: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    RegionPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

pub(in crate::context_bootstrap) fn vtt_region_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "VTTRegion must be constructed with new.");
        return;
    }
    RegionStateDeclaration::new()
        .initialize(scope, args.this())
        .expect("VTTRegion state declaration should initialize");
    rv.set(args.this().into());
}

fn get_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("VTTRegion receiver validated by WebIDL");
    let attribute = Attribute::from_callback(&args);
    rv.set(
        get_private_value(scope, target, attribute.slot()).expect("VTTRegion initialized state"),
    );
}

fn set_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("VTTRegion receiver validated by WebIDL");
    let attribute = Attribute::from_callback(&args);
    let context = webidl::Context::member("VTTRegion", attribute.name());
    let converted = match attribute {
        Attribute::Id => {
            webidl::convert::<webidl::DomString16>(scope, args.get(0), context).map(|value| {
                v8::String::new_from_two_byte(scope, &value.0, v8::NewStringType::Normal)
                    .expect("VTTRegion id allocation")
                    .into()
            })
        }
        Attribute::Lines => webidl::convert::<webidl::UnsignedLong>(scope, args.get(0), context)
            .map(|value| v8::Integer::new_from_unsigned(scope, value.0).into()),
        Attribute::Scroll => {
            let value = match webidl::convert::<webidl::DomString>(scope, args.get(0), context) {
                Ok(value) => value.0,
                Err(error) => {
                    webidl::throw_error(scope, &error);
                    return;
                }
            };
            // Invalid enum values are ignored by attribute setters.
            let Some(value) = <ScrollSetting as webidl::WebIdlEnum>::parse_token(&value) else {
                return;
            };
            let token = match value {
                ScrollSetting::None => "",
                ScrollSetting::Up => "up",
            };
            Ok(crate::util::v8str(scope, token).into())
        }
        _ => webidl::convert::<webidl::Double>(scope, args.get(0), context)
            .map(|value| v8::Number::new(scope, value.0).into()),
    };
    let value: v8::Local<v8::Value> = match converted {
        Ok(value) => value,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if !matches!(
        attribute,
        Attribute::Id | Attribute::Lines | Attribute::Scroll
    ) && !(0.0..=100.0).contains(
        &v8::Local::<v8::Number>::try_from(value)
            .expect("converted VTTRegion percentage")
            .value(),
    ) {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "VTTRegion percentage must be between 0 and 100.",
        );
        return;
    }
    set_private_value(scope, target, attribute.slot(), value);
}
