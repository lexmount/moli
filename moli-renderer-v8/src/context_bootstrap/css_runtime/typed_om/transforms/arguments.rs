use super::*;

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "CSSMatrixComponentOptions")]
struct MatrixOptions {
    #[webidl(name = "is2D")]
    is_2d: Option<bool>,
}

pub(super) fn interface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    brand: impl FnOnce(&mut v8::PinScope<'s, '_>, v8::Local<'s, v8::Object>) -> bool,
    context: webidl::Context,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && brand(scope, object)
    {
        Ok(object)
    } else {
        Err(webidl::WebIdlError::cannot_convert(
            context,
            "CSS transform operand",
        ))
    }
}

fn convert<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: Kind,
    index: u32,
    value: v8::Local<'s, v8::Value>,
    realm: v8::Local<'s, v8::Context>,
    constructing: bool,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    let context = webidl::Context::argument("CSS transform", index as usize + 1);
    match kind {
        Kind::Scale | Kind::Rotate if index < 3 => {
            math::numberish_in_realm(scope, value, context, realm)
        }
        Kind::Matrix => {
            if constructing {
                interface(
                    scope,
                    value,
                    web_api_interfaces::DOMMatrixReadOnly::is_instance,
                    context,
                )
            } else {
                interface(
                    scope,
                    value,
                    web_api_interfaces::DOMMatrix::is_instance,
                    context,
                )
            }
        }
        Kind::Perspective => {
            if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
                && (web_api_interfaces::CSSNumericValue::is_instance(scope, object)
                    || web_api_interfaces::CSSKeywordValue::is_instance(scope, object))
            {
                return Ok(object);
            }
            let keyword = webidl::convert::<webidl::UsvString>(scope, value, context)?.0;
            let scope = &mut v8::ContextScope::new(scope, realm);
            Ok(values::keyword_value(scope, keyword))
        }
        _ => interface(
            scope,
            value,
            web_api_interfaces::CSSNumericValue::is_instance,
            context,
        ),
    }
}

fn valid<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: Kind,
    index: u32,
    value: v8::Local<'s, v8::Object>,
) -> bool {
    match kind {
        Kind::Matrix => true,
        Kind::Translate => math::matches_dimension(scope, value, "px", index < 2),
        Kind::Scale => math::matches_dimension(scope, value, "number", false),
        Kind::Rotate if index < 3 => math::matches_dimension(scope, value, "number", false),
        Kind::Rotate | Kind::Skew | Kind::SkewX | Kind::SkewY => {
            math::matches_dimension(scope, value, "deg", false)
        }
        Kind::Perspective if web_api_interfaces::CSSKeywordValue::is_instance(scope, value) => {
            values::serialize(scope, value).is_some_and(|text| text.eq_ignore_ascii_case("none"))
        }
        Kind::Perspective => math::matches_dimension(scope, value, "px", false),
    }
}

pub(super) fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    kind: Kind,
) -> Option<(v8::Local<'s, v8::Array>, bool)> {
    if !args.is_construct_call() {
        throw_type_error(scope, "CSS transform constructors require new");
        return None;
    }
    let arity = match kind {
        Kind::Translate | Kind::Scale | Kind::Skew => 2,
        Kind::Rotate if args.length() >= 4 => 4,
        Kind::Rotate if args.length() != 1 => {
            throw_type_error(scope, "CSSRotate requires one or four arguments");
            return None;
        }
        _ => 1,
    };
    if args.length() < arity {
        throw_type_error(scope, "Missing CSS transform constructor argument");
        return None;
    }
    let realm = scope.get_current_context();
    let has_z = matches!(kind, Kind::Translate | Kind::Scale) && !args.get(2).is_undefined();
    let count = if has_z { 3 } else { arity };
    let mut items = Vec::new();
    // Complete WebIDL conversion before validating dimension constraints or
    // snapshotting a matrix: later argument getters can mutate earlier inputs.
    for index in 0..count {
        let operand_index = if kind == Kind::Rotate && count == 1 {
            3
        } else {
            index as u32
        };
        let value = match convert(scope, kind, operand_index, args.get(index), realm, true) {
            Ok(value) => value,
            Err(error) => {
                webidl::throw_error(scope, &error);
                return None;
            }
        };
        items.push(value);
    }
    let options = if kind == Kind::Matrix {
        match webidl::parse_dictionary::<MatrixOptions>(
            scope,
            args.get(1),
            webidl::Context::argument("CSSMatrixComponent", 2),
        ) {
            Ok(options) => options.unwrap_or_default(),
            Err(error) => {
                webidl::throw_error(scope, &error);
                return None;
            }
        }
    } else {
        MatrixOptions::default()
    };
    for (index, &value) in items.iter().enumerate() {
        let operand_index = if kind == Kind::Rotate && count == 1 {
            3
        } else {
            index as u32
        };
        if !valid(scope, kind, operand_index, value) {
            throw_type_error(scope, "Invalid CSS transform operand type");
            return None;
        }
    }
    let is_2d = match kind {
        Kind::Translate | Kind::Scale => {
            if !has_z {
                items.push(values::unit_value(
                    scope,
                    if kind == Kind::Scale { 1.0 } else { 0.0 },
                    if kind == Kind::Scale { "number" } else { "px" }.into(),
                ));
            }
            !has_z
        }
        Kind::Rotate => {
            if count == 1 {
                let angle = items[0];
                items = vec![
                    values::unit_value(scope, 0.0, "number".into()),
                    values::unit_value(scope, 0.0, "number".into()),
                    values::unit_value(scope, 1.0, "number".into()),
                    angle,
                ];
            }
            count == 1
        }
        Kind::Matrix => {
            let (_, is_2d, data) = geometry_runtime::dom_matrix_clone_data(scope, items[0])?;
            items[0] = geometry_runtime::build_dom_matrix_clone_object(scope, true, is_2d, data);
            options.is_2d.unwrap_or(is_2d)
        }
        _ => kind.fixed_dimension().unwrap(),
    };
    let items = items.into_iter().map(Into::into).collect::<Vec<_>>();
    Some((v8::Array::new_with_elements(scope, &items), is_2d))
}

pub(super) fn set_operand<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    index: u32,
    value: v8::Local<'s, v8::Value>,
) {
    let Some(kind) = kind(scope, object) else {
        return;
    };
    let Some(realm) = object.get_creation_context(scope) else {
        return;
    };
    let value = match convert(scope, kind, index, value, realm, false) {
        Ok(value) => value,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if !valid(scope, kind, index, value) {
        throw_type_error(scope, "Invalid CSS transform operand type");
        return;
    }
    if let Some(items) = operands(scope, object) {
        moli_webapi_declare::define_array_data_property(scope, items, index, value.into());
    }
}
