use super::*;
use crate::web_api_interfaces;

mod array;
mod serialization;
mod types;

pub(super) use serialization::serialize;
use types::NumericType;

const VALUES_SLOT: &str = "__moliCssMathValues";
const KIND_SLOT: &str = "__moliCssMathKind";
const TYPE_SLOT: &str = "__moliCssNumericType";
const ARRAY_SLOT: &str = "__moliCssNumericArray";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Sum,
    Product,
    Negate,
    Invert,
    Min,
    Max,
    Clamp,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Sum => "sum",
            Self::Product => "product",
            Self::Negate => "negate",
            Self::Invert => "invert",
            Self::Min => "min",
            Self::Max => "max",
            Self::Clamp => "clamp",
        }
    }

    fn arity(self) -> Option<usize> {
        match self {
            Self::Negate | Self::Invert => Some(1),
            Self::Clamp => Some(3),
            _ => None,
        }
    }
}

macro_rules! math_class {
    ($object:ident, $interface:ident, $callback:ident, $kind:ident) => {
        #[derive(WebApiObject)]
        #[webapi(interface = web_api_interfaces::$interface)]
        struct $object<'s> {
            #[webapi(slot = VALUES_SLOT)]
            values: v8::Local<'s, v8::Array>,
            #[webapi(slot = KIND_SLOT)]
            kind: u32,
            #[webapi(slot = TYPE_SLOT)]
            numeric_type: v8::Local<'s, v8::Array>,
        }

        pub(in crate::context_bootstrap) fn $callback<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            mut rv: v8::ReturnValue<'_, v8::Value>,
        ) {
            let Some((values, numeric_type)) = constructor_arguments(scope, &args, Kind::$kind)
            else {
                return;
            };
            $object::new(values, Kind::$kind as u32, numeric_type)
                .initialize(scope, args.this())
                .expect("CSS math object should initialize");
            rv.set(args.this().into());
        }
    };
}
math_class!(
    SumDeclaration,
    CSSMathSum,
    css_math_sum_constructor_callback,
    Sum
);
math_class!(
    ProductDeclaration,
    CSSMathProduct,
    css_math_product_constructor_callback,
    Product
);
math_class!(
    NegateDeclaration,
    CSSMathNegate,
    css_math_negate_constructor_callback,
    Negate
);
math_class!(
    InvertDeclaration,
    CSSMathInvert,
    css_math_invert_constructor_callback,
    Invert
);
math_class!(
    MinDeclaration,
    CSSMathMin,
    css_math_min_constructor_callback,
    Min
);
math_class!(
    MaxDeclaration,
    CSSMathMax,
    css_math_max_constructor_callback,
    Max
);
math_class!(
    ClampDeclaration,
    CSSMathClamp,
    css_math_clamp_constructor_callback,
    Clamp
);

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSNumericValue, enumerable, receiver)]
struct NumericPrototype {
    #[webapi(method = "type", callback = type_callback, length = 0)]
    numeric_type: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSMathValue, enumerable, receiver)]
struct MathPrototype {
    #[webapi(accessor_property, getter = operator_getter)]
    operator: (),
}

macro_rules! variadic_prototype {
    ($prototype:ident, $interface:ident) => {
        #[derive(WebApiFunctionTemplate)]
        #[webapi(interface = web_api_interfaces::$interface, enumerable, receiver)]
        struct $prototype {
            #[webapi(accessor_property, getter = values_getter)]
            values: (),
        }
    };
}
variadic_prototype!(SumPrototype, CSSMathSum);
variadic_prototype!(ProductPrototype, CSSMathProduct);
variadic_prototype!(MinPrototype, CSSMathMin);
variadic_prototype!(MaxPrototype, CSSMathMax);

macro_rules! unary_prototype {
    ($prototype:ident, $interface:ident) => {
        #[derive(WebApiFunctionTemplate)]
        #[webapi(interface = web_api_interfaces::$interface, enumerable, receiver)]
        struct $prototype {
            #[webapi(accessor_property, getter = first_getter)]
            value: (),
        }
    };
}
unary_prototype!(NegatePrototype, CSSMathNegate);
unary_prototype!(InvertPrototype, CSSMathInvert);

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSMathClamp, enumerable, receiver)]
struct ClampPrototype {
    #[webapi(accessor_property, getter = first_getter)]
    lower: (),
    #[webapi(accessor_property, getter = middle_getter)]
    value: (),
    #[webapi(accessor_property, getter = last_getter)]
    upper: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "CSSNumericValue" => NumericPrototype::initialize_prototype_template(scope, prototype),
        "CSSMathValue" => MathPrototype::initialize_prototype_template(scope, prototype),
        "CSSMathSum" => SumPrototype::initialize_prototype_template(scope, prototype),
        "CSSMathProduct" => ProductPrototype::initialize_prototype_template(scope, prototype),
        "CSSMathMin" => MinPrototype::initialize_prototype_template(scope, prototype),
        "CSSMathMax" => MaxPrototype::initialize_prototype_template(scope, prototype),
        "CSSMathNegate" => NegatePrototype::initialize_prototype_template(scope, prototype),
        "CSSMathInvert" => InvertPrototype::initialize_prototype_template(scope, prototype),
        "CSSMathClamp" => ClampPrototype::initialize_prototype_template(scope, prototype),
        "CSSNumericArray" => array::install(scope, template),
        _ => {}
    }
}

fn numberish<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && web_api_interfaces::CSSNumericValue::is_instance(scope, object)
    {
        return Ok(object);
    }
    // The union's numeric branch performs ToNumber, including user conversion
    // and its exceptions; an author Proxy does not inherit the target's brand.
    let number = f64::from(webidl::convert::<webidl::Double>(scope, value, context)?);
    Ok(values::unit_value(scope, number, "number".into()))
}

fn constructor_arguments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    kind: Kind,
) -> Option<(v8::Local<'s, v8::Array>, v8::Local<'s, v8::Array>)> {
    if !args.is_construct_call() {
        throw_type_error(scope, "CSS math constructors require new");
        return None;
    }
    let length = args.length() as usize;
    if kind.arity().is_some_and(|arity| length < arity) {
        throw_type_error(scope, "Missing CSS math constructor argument");
        return None;
    }
    let count = kind.arity().unwrap_or(length);
    let mut items = Vec::with_capacity(count);
    // Finish every WebIDL conversion before validating any combination of types.
    for index in 0..count {
        match numberish(
            scope,
            args.get(index as i32),
            webidl::Context::argument("CSS math constructor", index + 1),
        ) {
            Ok(item) => items.push(item),
            Err(error) => {
                webidl::throw_error(scope, &error);
                return None;
            }
        }
    }
    if items.is_empty() {
        webidl::throw_dom_exception(scope, "SyntaxError", "CSS math requires at least one value");
        return None;
    }
    let numeric_type = combined_type(scope, kind, &items).or_else(|| {
        throw_type_error(scope, "Incompatible CSS numeric types");
        None
    })?;
    let items = items.into_iter().map(Into::into).collect::<Vec<_>>();
    Some((
        v8::Array::new_with_elements(scope, &items),
        type_array(scope, numeric_type),
    ))
}

fn combined_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: Kind,
    items: &[v8::Local<'s, v8::Object>],
) -> Option<NumericType> {
    let mut result = numeric_type(scope, *items.first()?)?;
    if kind == Kind::Invert {
        return result.invert();
    }
    for &item in &items[1..] {
        let next = numeric_type(scope, item)?;
        result = if kind == Kind::Product {
            result.multiply(next)?
        } else {
            result.add(next)?
        };
    }
    Some(result)
}

fn type_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: NumericType,
) -> v8::Local<'s, v8::Array> {
    let entries = value
        .powers
        .into_iter()
        .chain([value.hint.map_or(-1, |i| i as i32)])
        .map(|i| v8::Integer::new(scope, i).into())
        .collect::<Vec<_>>();
    v8::Array::new_with_elements(scope, &entries)
}

fn numeric_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<NumericType> {
    if let Some(unit) = values::css_unit_value_unit(scope, object) {
        return NumericType::from_unit(&unit);
    }
    // Children and units are immutable, so the type is immutable even when a
    // descendant CSSUnitValue.value changes. Caching also bounds DAG traversal.
    let array = get_private_object(scope, object, TYPE_SLOT)
        .and_then(|v| v8::Local::<v8::Array>::try_from(v).ok())?;
    let mut result = NumericType::default();
    for (i, power) in result.powers.iter_mut().enumerate() {
        *power = array.get_index(scope, i as u32)?.int32_value(scope)?;
    }
    let hint = array.get_index(scope, 7)?.int32_value(scope)?;
    result.hint = (hint >= 0).then_some(hint as usize);
    Some(result)
}

fn type_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(numeric_type) = numeric_type(scope, args.this()) else {
        return;
    };
    let result = v8::Object::new(scope);
    let mut members: Vec<(&str, v8::Local<'s, v8::Value>)> = Vec::new();
    for (name, power) in types::BASE_NAMES.into_iter().zip(numeric_type.powers) {
        if power != 0 {
            members.push((name, v8::Integer::new(scope, power).into()));
        }
    }
    if let Some(hint) = numeric_type.hint {
        let value = v8_string(scope, types::BASE_NAMES[hint]).expect("CSS numeric hint");
        members.push(("percentHint", value.into()));
    }
    // WebIDL dictionary conversion defines own data properties in
    // lexicographic member order, including percentHint among the exponents.
    members.sort_unstable_by_key(|(name, _)| *name);
    for (name, value) in members {
        let key = v8_string(scope, name).expect("CSS numeric type member");
        result.create_data_property(scope, key.into(), value);
    }
    rv.set(result.into());
}

fn children<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_object(scope, object, VALUES_SLOT)
        .and_then(|v| v8::Local::<v8::Array>::try_from(v).ok())
}

fn kind<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<'s, v8::Object>) -> Option<Kind> {
    let value = get_private_value(scope, object, KIND_SLOT)?.uint32_value(scope)?;
    [
        Kind::Sum,
        Kind::Product,
        Kind::Negate,
        Kind::Invert,
        Kind::Min,
        Kind::Max,
        Kind::Clamp,
    ]
    .get(value as usize)
    .copied()
}

fn operator_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(kind) = kind(scope, args.this()) {
        rv.set(
            v8_string(scope, kind.name())
                .expect("CSS math operator")
                .into(),
        );
    }
}

fn values_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(array) = get_private_object(scope, args.this(), ARRAY_SLOT) {
        rv.set(array.into());
        return;
    }
    let Some(items) = children(scope, args.this()) else {
        return;
    };
    let context = args
        .this()
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    let array = array::new(scope, items);
    set_private_value(scope, args.this(), ARRAY_SLOT, array.into());
    rv.set(array.into());
}

macro_rules! child_getter {
    ($name:ident, $index:literal) => {
        fn $name<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            mut rv: v8::ReturnValue<'_, v8::Value>,
        ) {
            if let Some(value) =
                children(scope, args.this()).and_then(|v| v.get_index(scope, $index))
            {
                rv.set(value);
            }
        }
    };
}
child_getter!(first_getter, 0);
child_getter!(middle_getter, 1);
child_getter!(last_getter, 2);
