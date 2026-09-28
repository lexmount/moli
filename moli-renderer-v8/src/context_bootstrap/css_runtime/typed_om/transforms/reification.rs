use super::*;
use style::typed_om::{PerspectiveValue, TransformComponent, TransformValue};

/// Preserve the style engine's component kinds and dimensional flags. Do not
/// collapse a transform list to a matrix: percentages and relative units must
/// remain available to Typed OM callers without a layout context.
pub(in crate::context_bootstrap::css_runtime::typed_om) fn from_native<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &TransformValue,
) -> Option<v8::Local<'s, v8::Object>> {
    let mut components = Vec::with_capacity(value.len());
    for component in value {
        let (kind, is_2d, operands): (_, _, Vec<_>) = match component {
            TransformComponent::Translate(v) => (Kind::Translate, v.is_2d, vec![&v.x, &v.y, &v.z]),
            TransformComponent::Rotate(v) => {
                (Kind::Rotate, v.is_2d, vec![&v.x, &v.y, &v.z, &v.angle])
            }
            TransformComponent::Scale(v) => (Kind::Scale, v.is_2d, vec![&v.x, &v.y, &v.z]),
            TransformComponent::Skew(v) => (Kind::Skew, true, vec![&v.ax, &v.ay]),
            TransformComponent::SkewX(v) => (Kind::SkewX, true, vec![v]),
            TransformComponent::SkewY(v) => (Kind::SkewY, true, vec![v]),
            TransformComponent::Perspective(v) => (
                Kind::Perspective,
                false,
                match &v.length {
                    PerspectiveValue::Numeric(v) => vec![v],
                    PerspectiveValue::Keyword(_) => vec![],
                },
            ),
            TransformComponent::Matrix(v) => (Kind::Matrix, v.is_2d, vec![]),
        };
        let mut items = operands
            .into_iter()
            .map(|value| math::from_native(scope, value).map(Into::into))
            .collect::<Option<Vec<_>>>()?;
        match component {
            TransformComponent::Perspective(v) => {
                if let PerspectiveValue::Keyword(value) = &v.length {
                    items.push(values::keyword_value(scope, value.0.clone()).into());
                }
            }
            TransformComponent::Matrix(v) => {
                let m = &v.matrix;
                let data = [
                    m.m11, m.m12, m.m13, m.m14, m.m21, m.m22, m.m23, m.m24, m.m31, m.m32, m.m33,
                    m.m34, m.m41, m.m42, m.m43, m.m44,
                ]
                .map(f64::from);
                items.push(
                    geometry_runtime::build_dom_matrix_clone_object(scope, true, v.is_2d, data)
                        .into(),
                );
            }
            _ => {}
        }
        let items = v8::Array::new_with_elements(scope, &items);
        macro_rules! bind {
            ($declaration:ident) => {
                $declaration::new(items, is_2d, kind as u32)
                    .bind(scope)
                    .expect("CSS transform should bind")
            };
        }
        let object = match kind {
            Kind::Translate => bind!(TranslateDeclaration),
            Kind::Rotate => bind!(RotateDeclaration),
            Kind::Scale => bind!(ScaleDeclaration),
            Kind::Skew => bind!(SkewDeclaration),
            Kind::SkewX => bind!(SkewXDeclaration),
            Kind::SkewY => bind!(SkewYDeclaration),
            Kind::Perspective => bind!(PerspectiveDeclaration),
            Kind::Matrix => bind!(MatrixDeclaration),
        };
        components.push(object.into());
    }
    let components = v8::Array::new_with_elements(scope, &components);
    let template = v8::ObjectTemplate::new(scope);
    indexed::install(template);
    let object = template.new_instance(scope)?;
    TransformValueDeclaration::new(components)
        .bind_into(scope, object)
        .expect("CSSTransformValue should bind");
    Some(object)
}
