use super::*;
use moli_webapi_declare::WebApiFunctionTemplate;
use moli_webidl as webidl;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::FontFaceSetLoadEvent, enumerable, receiver)]
struct FontFaceSetLoadEventPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = font_face_set_load_event_fontfaces_getter
    )]
    fontfaces: (),
}

pub fn install_font_face_set_load_event_template_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    FontFaceSetLoadEventPrototypeDeclaration::initialize_prototype_template(scope, prototype);
}

fn font_face_set_load_event_fontfaces_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let value = event_private_value(scope, args.this(), FONT_FACE_SET_LOAD_EVENT_FONTFACES_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(value);
}

pub fn frozen_font_face_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: impl IntoIterator<Item = v8::Local<'s, v8::Value>>,
) -> v8::Local<'s, v8::Array> {
    let values = values.into_iter().collect::<Vec<_>>();
    let array = v8::Array::new_with_elements(scope, &values);
    let _ = array.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
    array
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "FontFaceSetLoadEventInit")]
pub(super) struct FontFaceSetLoadEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(sequence, interface = web_api_interfaces::FontFace, default = Vec::new())]
    fontfaces: Vec<v8::Local<'s, v8::Object>>,
}

impl<'s> FontFaceSetLoadEventInit<'s> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.base.bubbles, self.base.cancelable, self.base.composed)
    }
    pub(super) fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        let fontfaces = frozen_font_face_array(scope, self.fontfaces.into_iter().map(Into::into));
        set_private_value(
            scope,
            event,
            FONT_FACE_SET_LOAD_EVENT_FONTFACES_SLOT,
            fontfaces.into(),
        );
    }
}
