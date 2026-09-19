use super::*;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaQueryList.addListener")]
struct MediaQueryListAddListenerArgs {
    #[webidl(required, converter = "callback_interface", nullable)]
    listener: Option<webidl::WebIdlCallbackInterface>,
}

pub(in crate::context_bootstrap) fn media_query_list_add_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<MediaQueryListAddListenerArgs>(scope, &args) else {
        return;
    };
    let Some(listener) = parsed.listener else {
        return;
    };
    simple_object_event_target_register_webidl_listener(
        scope,
        args.this(),
        MEDIA_QUERY_LIST_LISTENERS_SLOT,
        "change".to_owned(),
        listener,
        webidl::EventListenerOptions::default(),
        None,
    );
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaQueryList.removeListener")]
struct MediaQueryListRemoveListenerArgs {
    #[webidl(required, converter = "callback_interface", nullable)]
    listener: Option<webidl::WebIdlCallbackInterface>,
}

pub(in crate::context_bootstrap) fn media_query_list_remove_listener_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<MediaQueryListRemoveListenerArgs>(scope, &args) else {
        return;
    };
    let Some(listener) = parsed.listener else {
        return;
    };
    let listener_value = listener.value(scope);
    simple_object_event_remove_listener_value_for_type(
        scope,
        args.this(),
        MEDIA_QUERY_LIST_LISTENERS_SLOT,
        "change",
        listener_value,
        false,
    );
}
