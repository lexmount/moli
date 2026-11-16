use super::*;
use moli_webapi_declare::WebApiObject;
use moli_webidl as webidl;
use webidl::WebIdlDictionarySerialize;

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "CustomEventInit", snapshot)]
pub(super) struct CustomEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(converter = "raw", snapshot_default = v8::null(scope).into())]
    detail: Option<v8::Local<'s, v8::Value>>,
}

impl<'s> CustomEventInit<'s> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.base.bubbles, self.base.cancelable, self.base.composed)
    }
    pub(super) fn initialize(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        self.initialize_snapshot(scope, event)
            .expect("CustomEventInit state");
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "CustomEvent.initCustomEvent")]
struct InitCustomEventArgs<'s> {
    #[webidl(
        required,
        converter = "raw",
        missing_message = "Failed to execute 'initCustomEvent': 1 argument required."
    )]
    event_type: webidl::DomString16,
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(index = 3, converter = "raw")]
    detail: Option<v8::Local<'s, v8::Value>>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct LegacyCustomEventInitDeclaration<'scope> {
    detail: v8::Local<'scope, v8::Value>,
}

pub(super) fn custom_event_init_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let event = event_backing(scope, args.this());
    let Some(parsed) = webidl::parse_args::<InitCustomEventArgs>(scope, &args) else {
        return;
    };
    if !reinitialize_event_object(
        scope,
        event,
        &parsed.event_type.0,
        parsed.bubbles,
        parsed.cancelable,
    ) {
        return;
    }
    let detail = parsed.detail.unwrap_or_else(|| v8::null(scope).into());
    LegacyCustomEventInitDeclaration::new(detail)
        .initialize(scope, event)
        .expect("legacy CustomEvent init declaration should initialize");
}
