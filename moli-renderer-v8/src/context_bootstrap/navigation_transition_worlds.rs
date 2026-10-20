use super::navigation_activation::NavigationTransitionObjectDeclaration;
use super::world_wrappers;
use crate::context_bootstrap::navigation_entry::wrappers as entry_wrappers;

pub(super) fn transition_in_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transition: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Context>,
) -> Option<v8::Local<'s, v8::Object>> {
    let transition = world_wrappers::owner(scope, transition);
    if world_wrappers::belongs_to_world(scope, transition, context) {
        return Some(transition);
    }
    if let Some(wrapper) = world_wrappers::get(scope, transition, context) {
        return Some(wrapper);
    }
    let scope = &mut v8::ContextScope::new(scope, context);
    let mut view = NavigationTransitionObjectDeclaration::from_private_state(scope, transition)?;
    view.from = entry_wrappers::in_realm(scope, view.from, context);
    if let Ok(to) = v8::Local::<v8::Object>::try_from(view.to) {
        view.to = super::navigation_events::navigation_destination_for_realm(scope, to)?.into();
    }
    view.committed = promise_in_current_realm(scope, view.committed)?;
    view.finished = promise_in_current_realm(scope, view.finished)?;
    let wrapper = view.bind(scope).ok()?;
    world_wrappers::insert(scope, transition, wrapper);
    Some(wrapper)
}

fn promise_in_current_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    promise: v8::Local<'s, v8::Promise>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let context = scope.get_current_context();
    if promise.get_creation_context(scope) == Some(context) {
        return Some(promise);
    }
    let resolver = v8::PromiseResolver::new(scope)?;
    let result = resolver.get_promise(scope);
    let fulfilled = v8::Function::builder(resolve_promise)
        .data(resolver.into())
        .build(scope)?;
    let rejected = v8::Function::builder(reject_promise)
        .data(resolver.into())
        .build(scope)?;
    let reaction = promise.then2(scope, fulfilled, rejected)?;
    super::navigation_result::suppress_unhandled_rejection(scope, reaction);
    super::navigation_result::suppress_unhandled_rejection(scope, result);
    Some(result)
}

fn resolve_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(object) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    let resolver = unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) };
    let context = scope.get_current_context();
    let value = entry_wrappers::value_in_realm(scope, args.get(0), context);
    let _ = resolver.resolve(scope, value);
}

fn reject_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(object) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    let resolver = unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) };
    let _ = resolver.reject(scope, args.get(0));
}
