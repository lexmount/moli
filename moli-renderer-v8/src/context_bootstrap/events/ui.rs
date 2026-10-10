use super::event_backing;
use crate::webidl;
use moli_webapi_declare::WebApiObject;

pub(in crate::context_bootstrap) fn ui_event_which_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    // The declaration's generated receiver check requires a native UIEvent.
    // Older native UI event producers do not store the legacy which member.
    let state = event_backing(scope, args.this());
    let Some(value) = state.get(scope, args.data()) else {
        return;
    };
    if value.is_undefined() {
        rv.set_uint32(0);
    } else {
        rv.set(value);
    }
}

fn nullable_window_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: Option<v8::Local<'s, v8::Object>>,
) -> v8::Local<'s, v8::Value> {
    window
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into())
}

/// Own UIEventInit members, including legacy which, in dictionary order.
#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "UIEventInit")]
struct UiEventInitMembers<'s> {
    #[webidl(default = 0)]
    detail: i32,
    #[webidl(nullable, interface = crate::web_api_interfaces::Window, brand_check = crate::context_bootstrap::is_window_receiver)]
    view: Option<v8::Local<'s, v8::Object>>,
    #[webidl(default = 0)]
    which: u32,
}

#[derive(Default)]
pub(super) struct UiEventInit<'s> {
    flags: (bool, bool, bool),
    members: UiEventInitMembers<'s>,
}

impl<'s> webidl::WebIdlDictionary<'s> for UiEventInit<'s> {
    fn parse_dictionary(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Result<Self, webidl::WebIdlError> {
        // Read inherited members before own members. Derived dictionaries
        // delegate here so each observable getter runs once, base-first.
        Ok(Self {
            flags: super::init::parse_event_init(scope, Some(object))?,
            members: webidl::parse_dictionary_object(scope, object)?,
        })
    }
}

impl<'s> UiEventInit<'s> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        self.flags
    }

    pub(super) fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        let view = nullable_window_value(scope, self.members.view);
        UiEventState::new(view, self.members.detail, self.members.which)
            .initialize(scope, event)
            .expect("UIEvent state should initialize");
    }
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct UiEventState<'s> {
    view: v8::Local<'s, v8::Value>,
    detail: i32,
    which: u32,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct LegacyUiEventState<'s> {
    view: v8::Local<'s, v8::Value>,
    detail: i32,
}

pub(in crate::context_bootstrap) fn initialize_legacy_ui_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    view: Option<v8::Local<'s, v8::Object>>,
    detail: i32,
) {
    let view = nullable_window_value(scope, view);
    LegacyUiEventState::new(view, detail)
        .initialize(scope, event)
        .expect("legacy UIEvent state should initialize");
}
