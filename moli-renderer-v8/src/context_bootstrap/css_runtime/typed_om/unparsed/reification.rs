use super::*;
use style::typed_om::{UnparsedSegment, UnparsedValue};

// Keep fallback nesting off the Rust call stack. These are native objects with
// the same indexed handlers and brands as constructor-created values; public
// constructors and author-modified Array methods are never invoked.
pub(in crate::context_bootstrap::css_runtime::typed_om) fn from_native<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: UnparsedValue,
) -> v8::Local<'s, v8::Object> {
    let mut frames = vec![(value.into_iter(), Vec::new(), None)];
    loop {
        let (parts, values, _) = frames.last_mut().expect("unparsed root frame");
        match parts.next() {
            Some(UnparsedSegment::String(text)) => {
                values.push(v8_string(scope, &text).expect("unparsed text").into());
            }
            Some(UnparsedSegment::VariableReference(reference)) => {
                if reference.has_fallback {
                    frames.push((
                        reference.fallback.into_iter(),
                        Vec::new(),
                        Some(reference.variable),
                    ));
                } else {
                    let fallback = v8::null(scope).into();
                    let value = VariableReferenceDeclaration::new(reference.variable, fallback)
                        .bind(scope)
                        .expect("CSSVariableReferenceValue declaration should bind");
                    values.push(value.into());
                }
            }
            None => {
                let (_, values, variable) = frames.pop().expect("completed unparsed frame");
                let segments = v8::Array::new_with_elements(scope, &values);
                let template = v8::ObjectTemplate::new(scope);
                indexed::install(template);
                let object = template
                    .new_instance(scope)
                    .expect("CSSUnparsedValue instance");
                UnparsedValueDeclaration::new(segments)
                    .bind_into(scope, object)
                    .expect("CSSUnparsedValue declaration should bind");
                let Some(variable) = variable else {
                    return object;
                };
                let reference = VariableReferenceDeclaration::new(variable, object.into())
                    .bind(scope)
                    .expect("CSSVariableReferenceValue declaration should bind");
                frames
                    .last_mut()
                    .expect("fallback parent")
                    .1
                    .push(reference.into());
            }
        }
    }
}
