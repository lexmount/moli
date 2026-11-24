//! Preserve V8's native Function construction while checking conversion-time
//! TrustedScript identity before V8 reduces its arguments to a code-like flag.

use super::*;

const FUNCTION_CONSTRUCTORS_INSTALLED_SLOT: &str = "__moliFunctionConstructorsInstalled";

#[derive(WebApiObject)]
#[webapi(plain)]
struct FunctionConstructorHandlerDeclaration<'s> {
    reflect_construct: v8::Local<'s, v8::Function>,
    #[webapi(method, callback = function_constructor_apply, length = 3)]
    apply: (),
    #[webapi(method, callback = function_constructor_construct, data = self.reflect_construct, length = 3)]
    construct: (),
}

pub(super) fn install_function_constructor_argument_conversion<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    if get_private_value(scope, global, FUNCTION_CONSTRUCTORS_INSTALLED_SLOT).is_some() {
        return Ok(());
    }
    let reflect = global
        .get(scope, v8str(scope, "Reflect").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow!("native Reflect is unavailable during Function bootstrap"))?;
    let reflect_construct = reflect
        .get(scope, v8str(scope, "construct").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .ok_or_else(|| {
            anyhow!("native Reflect.construct is unavailable during Function bootstrap")
        })?;
    let mut function = None;
    for (index, expression) in [
        "Function.prototype",
        "Object.getPrototypeOf(async function() {})",
        "Object.getPrototypeOf(function*() {})",
        "Object.getPrototypeOf(async function*() {})",
    ]
    .into_iter()
    .enumerate()
    {
        // Bootstrap runs before author code. Capture the actual intrinsic
        // prototypes and constructors, including Function.prototype's callability.
        let source = v8str(scope, expression);
        let prototype = v8::Script::compile(scope, source, None)
            .and_then(|script| crate::script_execution::execute_compiled_script(scope, script))
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .ok_or_else(|| anyhow!("native Function prototype is unavailable during bootstrap"))?;
        let constructor = prototype
            .get(scope, v8str(scope, "constructor").into())
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
            .ok_or_else(|| {
                anyhow!("native Function constructor is unavailable during bootstrap")
            })?;
        let handler = FunctionConstructorHandlerDeclaration::new(reflect_construct).bind(scope)?;
        let proxy: v8::Local<v8::Value> = v8::Proxy::new(scope, constructor.into(), handler)
            .ok_or_else(|| anyhow!("failed to initialize native Function argument conversion"))?
            .into();
        if index == 0 {
            function = Some(proxy);
            define_global_value(scope, global, "Function", proxy)?;
        } else if constructor.set_prototype(scope, function.expect("Function initialized first"))
            != Some(true)
        {
            return Err(anyhow!(
                "failed to preserve the native Function constructor ancestry"
            ));
        }
        if prototype.define_own_property(
            scope,
            v8str(scope, "constructor").into(),
            proxy,
            v8::PropertyAttribute::DONT_ENUM,
        ) != Some(true)
        {
            return Err(anyhow!(
                "failed to initialize native Function constructor identity"
            ));
        }
    }
    let installed = v8::Boolean::new(scope, true);
    set_private_value(
        scope,
        global,
        FUNCTION_CONSTRUCTORS_INSTALLED_SLOT,
        installed.into(),
    );
    Ok(())
}

fn converted_function_arguments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: v8::Local<'s, v8::Value>,
) -> Option<Vec<v8::Local<'s, v8::Value>>> {
    let values = v8::Local::<v8::Array>::try_from(values).ok()?;
    if values.length() == 0 {
        // Function() and Function("") have the same body and parameters. The
        // explicit body also prevents old V8 versions from treating no arguments
        // as an all-trusted argument list.
        return Some(vec![v8::String::empty(scope).into()]);
    }
    let mut converted = Vec::with_capacity(values.length() as usize);
    for index in 0..values.length() {
        let value = values.get_index(scope, index)?;
        let trusted = trusted_type_string16(scope, value, TrustedTypeKind::Script);
        let source = value.to_string(scope)?;
        let units = crate::util::v8_string_to_u16_string(scope, source);
        if trusted.as_deref() == Some(units.as_slice()) {
            converted.push(converted_trusted_function_argument(scope, source)?.into());
        } else {
            // V8's normal compilation callback applies the default policy once
            // to the complete Function source and performs the real CSP check.
            converted.push(source.into());
        }
    }
    Some(converted)
}

fn converted_trusted_function_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::String>,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let constructor = get_private_value(scope, global, TRUSTED_SCRIPT_CODE_LIKE_CONSTRUCTOR_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let argument = constructor.new_instance(scope, &[])?;
    TrustedTypeObjectDeclaration::new(source)
        .initialize(scope, argument)
        .ok()?;
    TrustedTypeKind::Script
        .interface()
        .initialize(scope, argument)
        .ok()?;
    // This private argument carries the already converted, matching trusted
    // data. Its native stringifier cannot invoke an author's prototype again.
    argument
        .set_prototype(scope, v8::null(scope).into())?
        .then_some(())?;
    let to_primitive = v8::Function::builder(converted_function_argument_to_primitive)
        .data(source.into())
        .build(scope)?;
    argument
        .define_own_property(
            scope,
            v8::Symbol::get_to_primitive(scope).into(),
            to_primitive.into(),
            v8::PropertyAttribute::READ_ONLY
                | v8::PropertyAttribute::DONT_ENUM
                | v8::PropertyAttribute::DONT_DELETE,
        )?
        .then_some(())?;
    Some(argument)
}

fn converted_function_argument_to_primitive<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(args.data());
}

fn function_constructor_apply<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Function>::try_from(args.get(0)) else {
        return;
    };
    let Some(arguments) = converted_function_arguments(scope, args.get(2)) else {
        return;
    };
    if let Some(function) = crate::util::call_script_visible_function(
        scope,
        target,
        args.get(1),
        &arguments,
        "Function constructor",
    ) {
        rv.set(function);
    }
}

fn function_constructor_construct<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(arguments) = converted_function_arguments(scope, args.get(1)) else {
        return;
    };
    let Ok(construct) = v8::Local::<v8::Function>::try_from(args.data()) else {
        return;
    };
    let arguments = v8::Array::new_with_elements(scope, &arguments);
    if let Some(function) = crate::util::call_script_visible_function(
        scope,
        construct,
        v8::undefined(scope).into(),
        &[args.get(0), arguments.into(), args.get(2)],
        "Function constructor with new.target",
    ) {
        rv.set(function);
    }
}
