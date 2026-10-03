use super::super::ERROR_EVENT_HANDLER_ARGUMENTS_SLOT;
use crate::util::{set_private_value, v8_string, v8_string_from_utf16_units};
use crate::webidl;
use moli_webapi_declare::WebApiObject;

/// Convert EventInit first, followed by ErrorEventInit's own members in lexical
/// order. All conversion completes before either base or payload state is set.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ErrorEventInit")]
pub(super) struct ErrorEventInit<'s> {
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(default = false)]
    composed: bool,
    #[webidl(default = 0)]
    colno: u32,
    #[webidl(converter = "raw")]
    error: Option<v8::Local<'s, v8::Value>>,
    #[webidl(default = "", converter = "usv_string")]
    filename: String,
    #[webidl(default = 0)]
    lineno: u32,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    message: webidl::DomString16,
}

impl Default for ErrorEventInit<'_> {
    fn default() -> Self {
        Self {
            bubbles: false,
            cancelable: false,
            composed: false,
            colno: 0,
            error: None,
            filename: String::new(),
            lineno: 0,
            message: webidl::DomString16(Vec::new()),
        }
    }
}

pub(super) fn parse_error_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<ErrorEventInit<'s>> {
    let parsed = webidl::dictionary_arg(args, 1, webidl::Context::argument("ErrorEvent", 2))
        .and_then(|object| match object {
            Some(object) => webidl::parse_dictionary_object(scope, object),
            None => Ok(ErrorEventInit::default()),
        });
    match parsed {
        Ok(init) => Some(init),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct ErrorEventState<'s> {
    message: v8::Local<'s, v8::String>,
    filename: v8::Local<'s, v8::String>,
    lineno: u32,
    colno: u32,
    error: v8::Local<'s, v8::Value>,
}

impl<'s> ErrorEventInit<'s> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.bubbles, self.cancelable, self.composed)
    }

    pub(super) fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        let message = v8_string_from_utf16_units(scope, &self.message.0)
            .expect("ErrorEvent DOMString message");
        let filename = v8_string(scope, &self.filename).expect("ErrorEvent USVString filename");
        let error = self.error.unwrap_or_else(|| v8::undefined(scope).into());
        ErrorEventState::new(message, filename, self.lineno, self.colno, error)
            .initialize(scope, event)
            .expect("ErrorEvent state should initialize");
        // Legacy onerror receives the converted private payload, independently
        // of author-defined getters that shadow the public attributes.
        let arguments = [
            message.into(),
            filename.into(),
            v8::Integer::new_from_unsigned(scope, self.lineno).into(),
            v8::Integer::new_from_unsigned(scope, self.colno).into(),
            error,
        ];
        let arguments = v8::Array::new_with_elements(scope, &arguments);
        set_private_value(
            scope,
            event,
            ERROR_EVENT_HANDLER_ARGUMENTS_SLOT,
            arguments.into(),
        );
    }
}
