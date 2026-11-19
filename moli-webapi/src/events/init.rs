use moli_webidl as webidl;

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "EventInit", snapshot)]
pub struct EventInit {
    #[webidl(default = false)]
    pub bubbles: bool,
    #[webidl(default = false)]
    pub cancelable: bool,
    #[webidl(default = false)]
    pub composed: bool,
}

pub fn parse_event_dictionary<'s, T: webidl::WebIdlDictionary<'s>>(
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

pub fn event_type_argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    constructor_name: &'static str,
) -> Option<v8::Local<'s, v8::String>> {
    if args.length() == 0 {
        moli_v8_util::throw_type_error(
            scope,
            &format!("Failed to construct '{constructor_name}': 1 argument required."),
        );
        return None;
    }
    args.get(0).to_string(scope)
}
