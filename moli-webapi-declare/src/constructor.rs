//! WebIDL argument conversion precedes the single Get(NewTarget, "prototype").
//! V8 allocates API constructor receivers before entering their callbacks. A
//! private NewTarget lets the native callback perform that Get at the specified
//! point, without guessing from V8's already selected Object.prototype fallback.

use moli_v8_util::{
    callable_relevant_context, get_private_object, get_private_value, global_constructor_prototype,
    new_null_prototype_object, private_key, throw_type_error, v8str,
};

const REFLECT_CONSTRUCT: &str = "moli.webidl.reflectConstruct";
const REFLECT_SET: &str = "moli.webidl.reflectSet";
const INVOCATION: &str = "moli.webidl.constructorInvocation";
const NEW_TARGET: &str = "moli.webidl.originalNewTarget";
const SELECTED_PROTOTYPE: &str = "moli.webidl.selectedConstructorPrototype";

/// Capture before author script runs, including before lazy interface access.
pub fn capture_web_api_constructor_intrinsics<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<()> {
    let global = scope.get_current_context().global(scope);
    if get_private_value(scope, global, REFLECT_CONSTRUCT).is_some()
        && get_private_value(scope, global, REFLECT_SET).is_some()
    {
        return Some(());
    }
    let reflect = global.get(scope, v8str(scope, "Reflect").into())?;
    let reflect = v8::Local::<v8::Object>::try_from(reflect).ok()?;
    for (name, slot) in [("construct", REFLECT_CONSTRUCT), ("set", REFLECT_SET)] {
        let function = reflect.get(scope, v8str(scope, name).into())?;
        let function = v8::Local::<v8::Function>::try_from(function).ok()?;
        let key = private_key(scope, slot)?;
        global
            .set_private(scope, key, function.into())?
            .then_some(())?;
    }
    Some(())
}

/// The realm's captured Reflect.set, including its original receiver semantics.
/// Native exotic objects must not consult a possibly replaced author global.
pub fn web_api_reflect_set<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Function>> {
    let global = scope.get_current_context().global(scope);
    get_private_value(scope, global, REFLECT_SET)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
}

/// Keep the native function as the intrinsic constructor; publish this entry
/// for JavaScript construction. Native callback data and instance templates are
/// preserved by invoking the original function with a private NewTarget.
pub fn web_api_constructor_with_deferred_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor: v8::Local<'s, v8::Function>,
) -> Option<v8::Local<'s, v8::Proxy>> {
    let global = scope.get_current_context().global(scope);
    let construct = get_private_value(scope, global, REFLECT_CONSTRUCT)?;
    let trap = v8::Function::builder(construct_trap)
        .data(construct)
        .length(3)
        .build(scope)?;
    let handler = new_null_prototype_object(scope);
    handler
        .define_own_property(
            scope,
            v8str(scope, "construct").into(),
            trap.into(),
            v8::PropertyAttribute::NONE,
        )?
        .then_some(())?;
    v8::Proxy::new(scope, constructor.into(), handler)
}

fn construct_trap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let construct =
        v8::Local::<v8::Function>::try_from(args.data()).expect("captured Reflect.construct");
    let Some(hidden_new_target) = v8::Function::new(scope, private_constructor) else {
        return;
    };
    let prototype = new_null_prototype_object(scope);
    if !hidden_new_target
        .define_own_property(
            scope,
            v8str(scope, "prototype").into(),
            prototype.into(),
            v8::PropertyAttribute::READ_ONLY
                | v8::PropertyAttribute::DONT_ENUM
                | v8::PropertyAttribute::DONT_DELETE,
        )
        .unwrap_or(false)
    {
        return;
    }
    let invocation = new_null_prototype_object(scope);
    let Some(new_target_key) = private_key(scope, NEW_TARGET) else {
        return;
    };
    let Some(invocation_key) = private_key(scope, INVOCATION) else {
        return;
    };
    if !invocation
        .set_private(scope, new_target_key, args.get(2))
        .unwrap_or(false)
        || !hidden_new_target
            .set_private(scope, invocation_key, invocation.into())
            .unwrap_or(false)
    {
        return;
    }
    if let Some(result) = construct.call(
        scope,
        v8::undefined(scope).into(),
        &[args.get(0), args.get(1), hidden_new_target.into()],
    ) {
        rv.set(result);
    }
}

fn private_constructor(
    _scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_>,
) {
}

pub(crate) fn restore_constructor_new_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
) -> Option<(v8::FunctionCallbackArguments<'s>, bool)> {
    if !args.is_construct_call() || args.new_target().is_proxy() {
        return Some((args, false));
    }
    let Ok(new_target) = v8::Local::<v8::Object>::try_from(args.new_target()) else {
        return Some((args, false));
    };
    let Some(invocation) = get_private_object(scope, new_target, INVOCATION) else {
        return Some((args, false));
    };
    let original = get_private_value(scope, invocation, NEW_TARGET)?;
    let key = private_key(scope, INVOCATION)?;
    args.this()
        .set_private(scope, key, invocation.into())?
        .then_some((args.with_new_target(original), true))
}

pub(crate) fn clear_constructor_invocation(
    scope: &mut v8::PinScope<'_, '_>,
    receiver: v8::Local<'_, v8::Object>,
) {
    if let Some(key) = private_key(scope, INVOCATION) {
        let _ = receiver.delete_private(scope, key);
    }
}

/// Call after overload resolution and all WebIDL argument conversion, before
/// constructor initialization. Internal construction through the cached native
/// function retains its V8 receiver and does not inspect author properties.
pub fn initialize_web_api_constructor_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    default_interface_name: &str,
) -> bool {
    let Some(invocation) = get_private_object(scope, receiver, INVOCATION) else {
        return true;
    };
    if get_private_value(scope, invocation, SELECTED_PROTOTYPE).is_some() {
        return true;
    }
    let Some(new_target) = get_private_object(scope, invocation, NEW_TARGET) else {
        return false;
    };
    let Some(prototype) = new_target.get(scope, v8str(scope, "prototype").into()) else {
        return false;
    };
    let prototype = if let Ok(prototype) = v8::Local::<v8::Object>::try_from(prototype) {
        prototype
    } else {
        let Some(context) = callable_relevant_context(scope, new_target.into()) else {
            return false;
        };
        let prototype = {
            let scope = &mut v8::ContextScope::new(scope, context);
            let Some(prototype) = global_constructor_prototype(scope, default_interface_name)
            else {
                throw_type_error(scope, "WebIDL constructor prototype is unavailable");
                return false;
            };
            v8::Global::new(scope, prototype)
        };
        v8::Local::new(scope, &prototype)
    };
    let Some(key) = private_key(scope, SELECTED_PROTOTYPE) else {
        return false;
    };
    receiver
        .set_prototype(scope, prototype.into())
        .unwrap_or(false)
        && invocation
            .set_private(scope, key, prototype.into())
            .unwrap_or(false)
}
