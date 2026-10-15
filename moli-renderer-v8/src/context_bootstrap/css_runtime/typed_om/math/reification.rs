use super::*;
use style::typed_om::{MathValue, NumericValue};

enum Work<'a> {
    Value(&'a NumericValue),
    Bind(Kind, usize),
}

/// Project the style engine's computed calculation tree directly into native
/// Typed OM objects. An explicit stack bounds work and avoids recursive V8
/// allocation; no public constructors or serialized CSS participate.
pub(in crate::context_bootstrap::css_runtime::typed_om) fn from_native<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &NumericValue,
) -> Option<v8::Local<'s, v8::Object>> {
    reify_native(scope, value, true)
}

pub(super) fn from_native_declared<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &NumericValue,
) -> Option<v8::Local<'s, v8::Object>> {
    reify_native(scope, value, false)
}

fn reify_native<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &NumericValue,
    fold_unary_sum: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let realm = scope.get_current_context();
    let mut work = vec![Work::Value(value)];
    let mut objects = Vec::new();
    let mut remaining = graph::MAX_WORK;
    while let Some(next) = work.pop() {
        remaining = remaining.checked_sub(1).or_else(|| {
            crate::util::throw_range_error(scope, "CSS numeric value is too large");
            None
        })?;
        match next {
            Work::Value(NumericValue::Unit(unit)) => {
                let name = values::native_unit_name(unit.unit_str());
                objects.push(values::unit_value(
                    scope,
                    f64::from(unit.value),
                    name.to_owned(),
                ));
            }
            Work::Value(NumericValue::Math(value)) => {
                let (kind, children): (_, &[NumericValue]) = match value {
                    MathValue::Sum(sum) => (Kind::Sum, &sum.values),
                    MathValue::Product(children) => (Kind::Product, children),
                    MathValue::Min(children) => (Kind::Min, children),
                    MathValue::Max(children) => (Kind::Max, children),
                    MathValue::Clamp(children) => (Kind::Clamp, children.as_ref()),
                    MathValue::Negate(child) => (Kind::Negate, std::slice::from_ref(child)),
                    MathValue::Invert(child) => (Kind::Invert, std::slice::from_ref(child)),
                };
                // The style engine wraps a scalar calculation in a unary sum.
                // At computed-value time it is the scalar CSSUnitValue.
                if fold_unary_sum && kind == Kind::Sum && children.len() == 1 {
                    work.push(Work::Value(&children[0]));
                    continue;
                }
                work.push(Work::Bind(kind, children.len()));
                work.extend(children.iter().rev().map(Work::Value));
            }
            Work::Bind(kind, count) => {
                let start = objects.len().checked_sub(count)?;
                let object = math_value(scope, kind, &objects[start..], realm)?;
                objects.truncate(start);
                objects.push(object);
            }
        }
    }
    objects.pop()
}
