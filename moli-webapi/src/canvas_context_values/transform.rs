use crate::geometry::{DomMatrix2DInit, same_dom_matrix_2d_number};
use moli_v8_util::throw_type_error;
use moli_webidl as webidl;

pub fn transform_arguments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    prefix: &'static str,
) -> Option<[f64; 6]> {
    if args.length() < 6 {
        throw_type_error(scope, &format!("{prefix} requires six arguments"));
        return None;
    }
    let mut values = [0.0; 6];
    for (index, value) in values.iter_mut().enumerate() {
        match webidl::argument::<webidl::UnrestrictedDouble>(
            scope,
            args,
            index as i32,
            webidl::Context::argument(prefix, index + 1),
        ) {
            Ok(parsed) => *value = parsed.0,
            Err(error) => {
                webidl::throw_error(scope, &error);
                return None;
            }
        }
    }
    Some(values)
}

pub fn set_transform_arguments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<[f64; 6]> {
    const PREFIX: &str = "CanvasRenderingContext2D.setTransform";
    if args.length() > 1 {
        return transform_arguments(scope, args, PREFIX);
    }
    let init = match webidl::parse_dictionary::<DomMatrix2DInit>(
        scope,
        args.get(0),
        webidl::Context::argument(PREFIX, 1),
    ) {
        Ok(init) => init.unwrap_or_default(),
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    validate_transform_init(scope, init)
}

pub fn validate_transform_init(
    scope: &mut v8::PinScope<'_, '_>,
    init: DomMatrix2DInit,
) -> Option<[f64; 6]> {
    let mut values = [0.0; 6];
    for (value, (alias, component, default)) in values.iter_mut().zip(init.affine_aliases()) {
        if let (Some(a), Some(b)) = (alias, component)
            && !same_dom_matrix_2d_number(a, b)
        {
            throw_type_error(scope, "DOMMatrix2DInit contains inconsistent aliases");
            return None;
        }
        *value = alias.or(component).unwrap_or(default);
    }
    Some(values)
}
