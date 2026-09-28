use super::*;

fn component<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let kind = kind(scope, object)?;
    let is_2d = is_2d(scope, object);
    let items = operands(scope, object)?;
    let text = |scope: &mut v8::PinScope<'s, '_>, index| {
        let value = item(scope, items, index)?;
        values::serialize(scope, value)
    };
    let (name, indices): (&str, &[u32]) = match kind {
        Kind::Translate if is_2d => ("translate", &[0, 1]),
        Kind::Translate => ("translate3d", &[0, 1, 2]),
        Kind::Scale if is_2d => {
            let x = item(scope, items, 0)?;
            let y = item(scope, items, 1)?;
            if math::equal_values(scope, x, &[y])? {
                ("scale", &[0])
            } else {
                ("scale", &[0, 1])
            }
        }
        Kind::Scale => ("scale3d", &[0, 1, 2]),
        Kind::Rotate if is_2d => ("rotate", &[3]),
        Kind::Rotate => ("rotate3d", &[0, 1, 2, 3]),
        Kind::Skew => {
            let y = item(scope, items, 1)?;
            if values::css_unit_value_number(scope, y) == Some(0.0) {
                ("skew", &[0])
            } else {
                ("skew", &[0, 1])
            }
        }
        Kind::SkewX => ("skewX", &[0]),
        Kind::SkewY => ("skewY", &[0]),
        Kind::Perspective => {
            let value = item(scope, items, 0)?;
            let text = text(scope, 0)?;
            // A negative unit is valid as a Typed OM operand but needs calc()
            // to defer the transform function's nonnegative range restriction.
            return Some(
                if values::css_unit_value_number(scope, value)
                    .is_some_and(|n| n < 0.0 && n.is_finite())
                {
                    format!("perspective(calc({text}))")
                } else {
                    format!("perspective({text})")
                },
            );
        }
        Kind::Matrix => {
            let (matrix, _) = matrix::component(scope, object)?;
            let result = matrix.dom_matrix_text_with_dimension(is_2d);
            if result.is_none() {
                webidl::throw_dom_exception(
                    scope,
                    "InvalidStateError",
                    "Matrix contains non-finite values",
                );
            }
            return result;
        }
    };
    let texts = indices
        .iter()
        .map(|&index| text(scope, index))
        .collect::<Option<Vec<_>>>()?;
    Some(format!("{name}({})", texts.join(", ")))
}

pub(in crate::context_bootstrap::css_runtime::typed_om) fn serialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let items = components(scope, object)?;
    let mut output = String::new();
    for i in 0..items.length() {
        let value = item(scope, items, i)?;
        let text = component(scope, value)?;
        if i > 0 {
            output.push(' ');
        }
        output.push_str(&text);
        if output.len() > 16 * 1024 * 1024 {
            crate::util::throw_range_error(scope, "CSS transform serialization is too large");
            return None;
        }
    }
    Some(output)
}

pub(super) fn component_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(text) = component(scope, args.this()).and_then(|text| v8_string(scope, &text)) {
        rv.set(text.into());
    }
}
