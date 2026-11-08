use crate::webidl;

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "EventInit", snapshot)]
pub(in crate::context_bootstrap) struct EventInit {
    #[webidl(default = false)]
    pub(in crate::context_bootstrap) bubbles: bool,
    #[webidl(default = false)]
    pub(in crate::context_bootstrap) cancelable: bool,
    #[webidl(default = false)]
    pub(in crate::context_bootstrap) composed: bool,
}

pub(in crate::context_bootstrap) fn parse_event_dictionary<'s, T: webidl::WebIdlDictionary<'s>>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    constructor: &'static str,
) -> Option<T> {
    let parsed = webidl::convert::<webidl::Dictionary<T>>(
        scope,
        args.get(1),
        webidl::Context::argument(constructor, 2),
    );
    match parsed {
        Ok(init) => Some(init.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

pub(super) fn read_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<(bool, bool, bool)> {
    parse_event_dictionary::<EventInit>(scope, args, "Event")
        .map(|init| (init.bubbles, init.cancelable, init.composed))
}
