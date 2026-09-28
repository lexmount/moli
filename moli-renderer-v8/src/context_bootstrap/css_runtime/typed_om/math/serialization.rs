use super::*;

enum Task<'s> {
    Value(v8::Local<'s, v8::Object>, bool, bool),
    Text(&'static str),
}

/// Serialize the native expression graph without recursing on the Rust stack
/// or invoking any public getters, iterators or stringifiers on its operands.
pub(in crate::context_bootstrap::css_runtime::typed_om) fn serialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let mut tasks = vec![Task::Value(object, false, false)];
    let mut output = String::new();
    let mut visited = 0usize;
    while let Some(task) = tasks.pop() {
        visited += 1;
        // Shared expression subtrees can expand exponentially. Bound the
        // expansion as well as stack depth instead of exhausting native memory.
        if visited > 1_000_000 || output.len() > 16 * 1024 * 1024 {
            crate::util::throw_range_error(scope, "CSS math serialization is too large");
            return None;
        }
        let Task::Value(object, nested, paren_less) = task else {
            if let Task::Text(text) = task {
                output.push_str(text);
            }
            continue;
        };
        if values::css_unit_value_unit(scope, object).is_some() {
            output.push_str(&values::serialize(scope, object)?);
            continue;
        }
        let op = kind(scope, object)?;
        let items = children(scope, object)?;
        let child = |scope: &mut v8::PinScope<'s, '_>, i| {
            items
                .get_index(scope, i)
                .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())
        };
        if matches!(op, Kind::Min | Kind::Max | Kind::Clamp) {
            output.push_str(op.name());
            output.push('(');
            tasks.push(Task::Text(")"));
            for i in (0..items.length()).rev() {
                tasks.push(Task::Value(child(scope, i)?, true, true));
                if i > 0 {
                    tasks.push(Task::Text(", "));
                }
            }
            continue;
        }
        if !paren_less {
            output.push_str(if nested { "(" } else { "calc(" });
            tasks.push(Task::Text(")"));
        }
        if matches!(op, Kind::Negate | Kind::Invert) {
            output.push_str(if op == Kind::Negate { "-" } else { "1 / " });
            tasks.push(Task::Value(child(scope, 0)?, true, false));
        } else {
            for i in (0..items.length()).rev() {
                let mut value = child(scope, i)?;
                let special = if op == Kind::Sum {
                    Kind::Negate
                } else {
                    Kind::Invert
                };
                let separator = if i > 0 && kind(scope, value) == Some(special) {
                    value = children(scope, value)?
                        .get_index(scope, 0)
                        .and_then(|v| v8::Local::<v8::Object>::try_from(v).ok())?;
                    if op == Kind::Sum { " - " } else { " / " }
                } else if op == Kind::Sum {
                    " + "
                } else {
                    " * "
                };
                tasks.push(Task::Value(value, true, false));
                if i > 0 {
                    tasks.push(Task::Text(separator));
                }
            }
        }
    }
    Some(output)
}
