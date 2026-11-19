use super::super::init::EventInit;
use super::{
    EventSubclassKind, SubclassInit, construct_native_subclass, construct_trusted_subclass,
};
use moli_webidl as webidl;
use webidl::WebIdlDictionarySerialize;

/// Convert EventInit first, followed by ErrorEventInit's own members in lexical
/// order. All conversion completes before either base or payload state is set.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ErrorEventInit", snapshot)]
pub(super) struct ErrorEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(default = 0)]
    colno: u32,
    #[webidl(converter = "raw", snapshot_default = v8::undefined(scope).into())]
    error: Option<v8::Local<'s, v8::Value>>,
    #[webidl(default = "", converter = "usv_string")]
    filename: String,
    #[webidl(default = 0)]
    lineno: u32,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    message: webidl::DomString16,
}

pub(super) fn parse_error_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<ErrorEventInit<'s>> {
    super::super::init::parse_event_dictionary(scope, args, "ErrorEvent")
}

impl<'s> ErrorEventInit<'s> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.base.bubbles, self.base.cancelable, self.base.composed)
    }

    pub(super) fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        self.initialize_snapshot(scope, event)
            .expect("ErrorEvent state should initialize");
        // Existing onerror dispatch reads this native backing, bypassing public getters.
    }
}

/// Reports carry converted values, without invoking author constructors or getters.
pub fn construct_original_error_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
    filename: &str,
    lineno: u32,
    colno: u32,
    error: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let init = ErrorEventInit {
        base: EventInit {
            cancelable: true,
            ..EventInit::default()
        },
        message: webidl::DomString16(message.encode_utf16().collect()),
        filename: filename.to_owned(),
        lineno,
        colno,
        error: Some(error),
    };
    construct_native_subclass(
        scope,
        EventSubclassKind::ErrorEvent,
        "error",
        SubclassInit::Error(init),
    )
}

pub fn construct_native_navigation_error_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    error: v8::Local<'s, v8::Value>,
    filename: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let message = error
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let init = ErrorEventInit {
        base: EventInit::default(),
        message: webidl::DomString16(message.encode_utf16().collect()),
        filename: filename.to_owned(),
        lineno: 1,
        colno: 1,
        error: (!error.is_undefined()).then_some(error),
    };
    construct_trusted_subclass(
        scope,
        EventSubclassKind::ErrorEvent,
        "navigateerror",
        SubclassInit::Error(init),
    )
}
