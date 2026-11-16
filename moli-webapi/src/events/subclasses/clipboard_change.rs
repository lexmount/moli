use super::*;
use moli_webidl as webidl;

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ClipboardChangeEventInit")]
pub(super) struct ClipboardChangeEventInitMembers<'scope> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(name = "changeId", with = clipboard_change_event_change_id_member)]
    change_id: v8::Local<'scope, v8::BigInt>,
    #[webidl(with = clipboard_change_event_types_member)]
    types: Vec<v8::Local<'scope, v8::String>>,
}

struct ClipboardChangeEventType<'scope>(v8::Local<'scope, v8::String>);

impl<'scope> webidl::WebIdlConverter<'scope> for ClipboardChangeEventType<'scope> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'scope, '_>,
        value: v8::Local<'scope, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        value
            .to_string(scope)
            .map(Self)
            .ok_or_else(|| webidl::WebIdlError::pending_exception(context))
    }
}

impl ClipboardChangeEventInitMembers<'_> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.base.bubbles, self.base.cancelable, self.base.composed)
    }
}

fn clipboard_change_event_change_id_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<v8::Local<'s, v8::BigInt>, webidl::WebIdlError> {
    let context = webidl::Context::member("ClipboardChangeEventInit", name);
    let Some(value) = webidl::property_result(scope, object, name, context)? else {
        return Ok(v8::BigInt::new_from_u64(scope, 0));
    };
    if value.is_undefined() {
        return Ok(v8::BigInt::new_from_u64(scope, 0));
    }
    value
        .to_big_int(scope)
        .ok_or_else(|| webidl::WebIdlError::pending_exception(context))
}

fn clipboard_change_event_types_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<Vec<v8::Local<'s, v8::String>>, webidl::WebIdlError> {
    let context = webidl::Context::member("ClipboardChangeEventInit", name);
    let Some(value) = webidl::property_result(scope, object, name, context)? else {
        return Ok(Vec::new());
    };
    if value.is_undefined() {
        return Ok(Vec::new());
    }
    webidl::convert::<webidl::Sequence<ClipboardChangeEventType<'s>>>(scope, value, context)
        .map(|sequence| sequence.0.into_iter().map(|value| value.0).collect())
}

pub(super) fn parse_clipboard_change_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<ClipboardChangeEventInitMembers<'s>> {
    let context = webidl::Context::argument("ClipboardChangeEvent", 2);
    let object = match webidl::dictionary_arg(args, 1, context) {
        Ok(object) => object,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    let Some(object) = object else {
        return Some(ClipboardChangeEventInitMembers {
            base: EventInit::default(),
            change_id: v8::BigInt::new_from_u64(scope, 0),
            types: Vec::new(),
        });
    };
    match webidl::parse_dictionary_object(scope, object) {
        Ok(init) => Some(init),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

pub fn initialize_clipboard_change_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: ClipboardChangeEventInitMembers<'s>,
) -> bool {
    let mut elements = Vec::with_capacity(init.types.len());
    for event_type in init.types {
        elements.push(event_type.into());
    }
    let types = v8::Array::new_with_elements(scope, &elements);
    if types.set_integrity_level(scope, v8::IntegrityLevel::Frozen) != Some(true) {
        return false;
    }
    set_private_value(
        scope,
        event,
        CLIPBOARD_CHANGE_EVENT_TYPES_SLOT,
        types.into(),
    );
    set_private_value(
        scope,
        event,
        CLIPBOARD_CHANGE_EVENT_CHANGE_ID_SLOT,
        init.change_id.into(),
    );
    true
}
