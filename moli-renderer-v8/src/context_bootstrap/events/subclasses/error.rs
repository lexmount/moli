use crate::context_bootstrap::events::EventInit;
use crate::util::v8_string;
use crate::webidl;
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
    crate::context_bootstrap::events::parse_event_dictionary(scope, args, "ErrorEvent")
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

/// Runtime reports carry already converted payloads. Use the protected
/// constructor with no dictionary so author getters and replacements never
/// participate in native error reporting.
pub(crate) fn construct_original_error_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
    filename: &str,
    lineno: u32,
    colno: u32,
    error: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let constructor =
        crate::context_bootstrap::exposed_interfaces::ensure_intrinsic_interface_constructor(
            scope,
            "ErrorEvent",
        )
        .ok()?;
    let event_type = v8_string(scope, "error")?;
    let event = constructor.new_instance(scope, &[event_type.into()])?;
    let state = super::super::event_backing(scope, event);
    ErrorEventInit {
        base: EventInit {
            cancelable: true,
            ..EventInit::default()
        },
        message: webidl::DomString16(message.encode_utf16().collect()),
        filename: filename.to_owned(),
        lineno,
        colno,
        error: Some(error),
    }
    .initialize_snapshot(scope, state)?;
    Some(event)
}
