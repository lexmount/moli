use super::*;
use crate::{context_bootstrap::geometry_runtime, web_api_interfaces};
use moli_geometry::DomMatrixComponents;

mod arguments;
mod indexed;
mod matrix;
mod reification;
mod serialization;

pub(super) use reification::from_native;
pub(super) use serialization::serialize;

const OPERANDS_SLOT: &str = "__moliCssTransformOperands";
const KIND_SLOT: &str = "__moliCssTransformKind";
const IS_2D_SLOT: &str = "__moliCssTransformIs2D";
const COMPONENTS_SLOT: &str = "__moliCssTransformComponents";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Translate,
    Rotate,
    Scale,
    Skew,
    SkewX,
    SkewY,
    Perspective,
    Matrix,
}

impl Kind {
    fn fixed_dimension(self) -> Option<bool> {
        match self {
            Self::Skew | Self::SkewX | Self::SkewY => Some(true),
            Self::Perspective => Some(false),
            _ => None,
        }
    }
}

macro_rules! component {
    ($declaration:ident, $interface:ident, $callback:ident, $kind:ident) => {
        #[derive(WebApiObject)]
        #[webapi(interface = web_api_interfaces::$interface)]
        struct $declaration<'s> {
            #[webapi(slot = OPERANDS_SLOT)]
            operands: v8::Local<'s, v8::Array>,
            #[webapi(slot = IS_2D_SLOT)]
            is_2d: bool,
            #[webapi(slot = KIND_SLOT)]
            kind: u32,
        }

        pub(in crate::context_bootstrap) fn $callback<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            mut rv: v8::ReturnValue<'_, v8::Value>,
        ) {
            let Some((operands, is_2d)) = arguments::constructor(scope, &args, Kind::$kind) else {
                return;
            };
            $declaration::new(operands, is_2d, Kind::$kind as u32)
                .initialize(scope, args.this())
                .expect("CSS transform component should initialize");
            rv.set(args.this().into());
        }
    };
}

component!(
    TranslateDeclaration,
    CSSTranslate,
    css_translate_constructor_callback,
    Translate
);
component!(
    RotateDeclaration,
    CSSRotate,
    css_rotate_constructor_callback,
    Rotate
);
component!(
    ScaleDeclaration,
    CSSScale,
    css_scale_constructor_callback,
    Scale
);
component!(
    SkewDeclaration,
    CSSSkew,
    css_skew_constructor_callback,
    Skew
);
component!(
    SkewXDeclaration,
    CSSSkewX,
    css_skew_x_constructor_callback,
    SkewX
);
component!(
    SkewYDeclaration,
    CSSSkewY,
    css_skew_y_constructor_callback,
    SkewY
);
component!(
    PerspectiveDeclaration,
    CSSPerspective,
    css_perspective_constructor_callback,
    Perspective
);
component!(
    MatrixDeclaration,
    CSSMatrixComponent,
    css_matrix_component_constructor_callback,
    Matrix
);

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CSSTransformValue)]
struct TransformValueDeclaration<'s> {
    #[webapi(slot = COMPONENTS_SLOT)]
    components: v8::Local<'s, v8::Array>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSTransformComponent, enumerable, receiver)]
struct ComponentPrototype {
    #[webapi(name = "is2D", accessor_property, getter = is_2d_getter, setter = is_2d_setter)]
    is_2d: (),
    #[webapi(method = "toMatrix", callback = matrix::component_callback, length = 0)]
    to_matrix: (),
    #[webapi(method = "toString", callback = serialization::component_callback, length = 0)]
    to_string: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSTransformValue, enumerable, receiver)]
struct ValuePrototype {
    #[webapi(accessor_property, getter = length_getter)]
    length: (),
    #[webapi(name = "is2D", accessor_property, getter = value_is_2d_getter)]
    is_2d: (),
    #[webapi(method = "toMatrix", callback = matrix::value_callback, length = 0)]
    to_matrix: (),
    #[webapi(intrinsic_data_property = v8::Intrinsic::ArrayProtoEntries)]
    entries: (),
    #[webapi(intrinsic_data_property = v8::Intrinsic::ArrayProtoKeys)]
    keys: (),
    #[webapi(intrinsic_data_property = v8::Intrinsic::ArrayProtoValues)]
    values: (),
    #[webapi(intrinsic_data_property = v8::Intrinsic::ArrayProtoForEach)]
    for_each: (),
    #[webapi(intrinsic_data_property = v8::Intrinsic::ArrayProtoValues, symbol = "iterator")]
    iterator: (),
}

macro_rules! attributes {
    ($prototype:ident, $interface:ident, $($name:ident => $getter:ident, $setter:ident);+ $(;)?) => {
        #[derive(WebApiFunctionTemplate)]
        #[webapi(interface = web_api_interfaces::$interface, enumerable, receiver)]
        struct $prototype {
            $(#[webapi(accessor_property, getter = $getter, setter = $setter)] $name: (),)+
        }
    };
}
attributes!(TranslatePrototype, CSSTranslate, x => first_getter, first_setter; y => second_getter, second_setter; z => third_getter, third_setter);
attributes!(RotatePrototype, CSSRotate, x => first_getter, first_setter; y => second_getter, second_setter; z => third_getter, third_setter; angle => fourth_getter, fourth_setter);
attributes!(ScalePrototype, CSSScale, x => first_getter, first_setter; y => second_getter, second_setter; z => third_getter, third_setter);
attributes!(SkewPrototype, CSSSkew, ax => first_getter, first_setter; ay => second_getter, second_setter);
attributes!(SkewXPrototype, CSSSkewX, ax => first_getter, first_setter);
attributes!(SkewYPrototype, CSSSkewY, ay => first_getter, first_setter);
attributes!(PerspectivePrototype, CSSPerspective, length => first_getter, first_setter);
attributes!(MatrixPrototype, CSSMatrixComponent, matrix => first_getter, first_setter);

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "CSSTransformValue" => {
            ValuePrototype::initialize_prototype_template(scope, prototype);
            indexed::install(template.instance_template(scope));
        }
        "CSSTransformComponent" => {
            ComponentPrototype::initialize_prototype_template(scope, prototype)
        }
        "CSSTranslate" => TranslatePrototype::initialize_prototype_template(scope, prototype),
        "CSSRotate" => RotatePrototype::initialize_prototype_template(scope, prototype),
        "CSSScale" => ScalePrototype::initialize_prototype_template(scope, prototype),
        "CSSSkew" => SkewPrototype::initialize_prototype_template(scope, prototype),
        "CSSSkewX" => SkewXPrototype::initialize_prototype_template(scope, prototype),
        "CSSSkewY" => SkewYPrototype::initialize_prototype_template(scope, prototype),
        "CSSPerspective" => PerspectivePrototype::initialize_prototype_template(scope, prototype),
        "CSSMatrixComponent" => MatrixPrototype::initialize_prototype_template(scope, prototype),
        _ => {}
    }
}

fn kind<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<'s, v8::Object>) -> Option<Kind> {
    let index = get_private_value(scope, object, KIND_SLOT)?.uint32_value(scope)?;
    [
        Kind::Translate,
        Kind::Rotate,
        Kind::Scale,
        Kind::Skew,
        Kind::SkewX,
        Kind::SkewY,
        Kind::Perspective,
        Kind::Matrix,
    ]
    .get(index as usize)
    .copied()
}

fn array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Array>> {
    v8::Local::try_from(get_private_object(scope, object, slot)?).ok()
}

fn operands<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    array(scope, object, OPERANDS_SLOT)
}
fn components<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    array(scope, object, COMPONENTS_SLOT)
}
fn item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    items: v8::Local<'s, v8::Array>,
    index: u32,
) -> Option<v8::Local<'s, v8::Object>> {
    v8::Local::try_from(items.get_index(scope, index)?).ok()
}
fn is_2d<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<'s, v8::Object>) -> bool {
    get_private_value(scope, object, IS_2D_SLOT).is_some_and(|value| value.is_true())
}

fn is_2d_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(is_2d(scope, args.this()));
}
fn is_2d_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if kind(scope, args.this()).is_some_and(|kind| kind.fixed_dimension().is_none()) {
        let value = v8::Boolean::new(scope, args.get(0).boolean_value(scope));
        set_private_value(scope, args.this(), IS_2D_SLOT, value.into());
    }
}
fn length_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(items) = components(scope, args.this()) {
        rv.set_uint32(items.length());
    }
}
fn value_is_2d_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(items) = components(scope, args.this()) {
        rv.set_bool(
            (0..items.length())
                .all(|i| item(scope, items, i).is_some_and(|component| is_2d(scope, component))),
        );
    }
}

macro_rules! operand_callbacks {
    ($getter:ident, $setter:ident, $index:expr) => {
        fn $getter<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            mut rv: v8::ReturnValue<'_, v8::Value>,
        ) {
            if let Some(value) =
                operands(scope, args.this()).and_then(|items| items.get_index(scope, $index))
            {
                rv.set(value);
            }
        }
        fn $setter<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            _rv: v8::ReturnValue<'_, v8::Value>,
        ) {
            arguments::set_operand(scope, args.this(), $index, args.get(0));
        }
    };
}
operand_callbacks!(first_getter, first_setter, 0);
operand_callbacks!(second_getter, second_setter, 1);
operand_callbacks!(third_getter, third_setter, 2);
operand_callbacks!(fourth_getter, fourth_setter, 3);

struct Component<'s>(v8::Local<'s, v8::Object>);
impl<'s> webidl::WebIdlConverter<'s> for Component<'s> {
    type Options = ();
    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        arguments::interface(
            scope,
            value,
            web_api_interfaces::CSSTransformComponent::is_instance,
            context,
        )
        .map(Self)
    }
}

pub(in crate::context_bootstrap) fn css_transform_value_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() || args.length() == 0 {
        throw_type_error(
            scope,
            "CSSTransformValue requires new and a sequence of components",
        );
        return;
    }
    let items = match webidl::convert::<webidl::Sequence<Component<'s>>>(
        scope,
        args.get(0),
        webidl::Context::argument("CSSTransformValue", 1),
    ) {
        Ok(values) => values.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if items.is_empty() {
        throw_type_error(scope, "CSSTransformValue requires at least one component");
        return;
    }
    let items = items
        .into_iter()
        .map(|item| item.0.into())
        .collect::<Vec<_>>();
    let items = v8::Array::new_with_elements(scope, &items);
    TransformValueDeclaration::new(items)
        .initialize(scope, args.this())
        .expect("CSSTransformValue should initialize");
    rv.set(args.this().into());
}
