use super::graph::Unit;
use super::*;

fn arguments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<Vec<v8::Local<'s, v8::Object>>> {
    let realm = args.this().get_creation_context(scope)?;
    (0..args.length())
        .map(|i| {
            numberish_in_realm(
                scope,
                args.get(i),
                webidl::Context::argument("CSSNumericValue", i as usize + 1),
                realm,
            )
            .map_err(|error| webidl::throw_error(scope, &error))
            .ok()
        })
        .collect()
}

fn unary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    operation: Kind,
    realm: v8::Local<'s, v8::Context>,
) -> Option<v8::Local<'s, v8::Object>> {
    if kind(scope, object) == Some(operation) {
        return operands(scope, object)?.first().copied();
    }
    if let Some(mut value) = Unit::read(scope, object) {
        if operation == Kind::Negate {
            value.value = -value.value;
            return Some(value.bind(scope, realm));
        }
        if value.unit == "number" {
            if value.value == 0.0 {
                crate::util::throw_range_error(scope, "Cannot divide by zero");
                return None;
            }
            value.value = 1.0 / value.value;
            return Some(value.bind(scope, realm));
        }
    }
    math_value(scope, operation, &[object], realm)
}

// Preserve NaN and signed zero instead of Rust's min/max NaN suppression.
pub(super) fn minimum(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b && a == 0.0 {
        if a.is_sign_negative() || b.is_sign_negative() {
            -0.0
        } else {
            0.0
        }
    } else {
        a.min(b)
    }
}

pub(super) fn maximum(a: f64, b: f64) -> f64 {
    -minimum(-a, -b)
}

fn arithmetic<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    operation: Kind,
    transform: Option<Kind>,
) -> Option<v8::Local<'s, v8::Object>> {
    // The binding checks this before conversion. Finish converting *all*
    // arguments before reading mutable operands or testing types/zero divisors.
    let realm = args.this().get_creation_context(scope)?;
    let mut items = arguments(scope, args)?;
    if let Some(transform) = transform {
        for item in &mut items {
            *item = unary(scope, *item, transform, realm)?;
        }
    }
    let mut all = if kind(scope, args.this()) == Some(operation) {
        operands(scope, args.this())?
    } else {
        vec![args.this()]
    };
    all.extend(items);
    let units = all
        .iter()
        .map(|&item| Unit::read(scope, item))
        .collect::<Option<Vec<_>>>();
    if let Some(units) = units {
        let first = &units[0];
        let mut result = first.clone();
        let simplify = if operation == Kind::Product {
            let mut dimension = units.iter().filter(|v| v.unit != "number");
            result.unit = dimension
                .next()
                .map_or("number", |v| v.unit.as_str())
                .to_owned();
            dimension.next().is_none()
        } else {
            units.iter().all(|v| v.unit == first.unit)
        };
        if simplify {
            for unit in &units[1..] {
                result.value = match operation {
                    Kind::Sum => result.value + unit.value,
                    Kind::Product => result.value * unit.value,
                    Kind::Min => minimum(result.value, unit.value),
                    Kind::Max => maximum(result.value, unit.value),
                    _ => unreachable!(),
                };
            }
            return Some(result.bind(scope, realm));
        }
    }
    math_value(scope, operation, &all, realm)
}

macro_rules! operation {
    ($callback:ident, $kind:ident, $transform:expr) => {
        pub(super) fn $callback<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            mut rv: v8::ReturnValue<'_, v8::Value>,
        ) {
            if let Some(value) = arithmetic(scope, &args, Kind::$kind, $transform) {
                rv.set(value.into());
            }
        }
    };
}
operation!(add_callback, Sum, None);
operation!(sub_callback, Sum, Some(Kind::Negate));
operation!(mul_callback, Product, None);
operation!(div_callback, Product, Some(Kind::Invert));
operation!(min_callback, Min, None);
operation!(max_callback, Max, None);

pub(super) fn equals_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(items) = arguments(scope, &args) else {
        return;
    };
    if let Some(equal) = graph::equals(scope, args.this(), &items) {
        rv.set_bool(equal);
    }
}
