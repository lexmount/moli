use super::{history_runtime::native, world_wrappers};
use crate::util::v8str;
use crate::web_api_interfaces;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::NavigationTransition, own_to_string_tag = "NavigationTransition", readonly_to_string_tag)]
struct TransitionView<'scope> {
    #[webapi(data_property)]
    from: v8::Local<'scope, v8::Value>,
    #[webapi(data_property)]
    to: v8::Local<'scope, v8::Value>,
    #[webapi(data_property)]
    navigation_type: v8::Local<'scope, v8::Value>,
    #[webapi(data_property)]
    committed: v8::Local<'scope, v8::Promise>,
    #[webapi(data_property)]
    finished: v8::Local<'scope, v8::Promise>,
}

pub(super) fn transition_in_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    transition: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Context>,
) -> Option<v8::Local<'s, v8::Object>> {
    let transition = world_wrappers::owner(scope, transition);
    if let Some(wrapper) = world_wrappers::get(scope, transition, context) {
        return Some(wrapper);
    }
    let scope = &mut v8::ContextScope::new(scope, context);
    let from = transition.get(scope, v8str(scope, "from").into())?;
    let from = native::entry_value_in_realm(scope, from, context);
    let to = transition.get(scope, v8str(scope, "to").into())?;
    let to = if let Ok(to) = v8::Local::<v8::Object>::try_from(to) {
        super::navigation_events::navigation_destination_for_realm(scope, to)?.into()
    } else {
        to
    };
    let navigation_type = transition.get(scope, v8str(scope, "navigationType").into())?;
    let committed = transition.get(scope, v8str(scope, "committed").into())?;
    let committed =
        promise_in_current_realm(scope, v8::Local::<v8::Promise>::try_from(committed).ok()?)?;
    let finished = transition.get(scope, v8str(scope, "finished").into())?;
    let finished =
        promise_in_current_realm(scope, v8::Local::<v8::Promise>::try_from(finished).ok()?)?;
    let wrapper = TransitionView {
        from,
        to,
        navigation_type,
        committed,
        finished,
    }
    .bind(scope)
    .ok()?;
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
    let value = native::entry_value_in_realm(scope, args.get(0), context);
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
