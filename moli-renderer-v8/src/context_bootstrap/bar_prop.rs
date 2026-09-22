use anyhow::{Result, anyhow};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use super::shared::throw_error;
use crate::{
    native_bridge::OwnerDispatchScope,
    util::{context_host_ptr_from_global_bridge, get_private_value, set_private_value},
    web_api_interfaces,
};

const NAMES: &[&str] = &[
    "locationbar",
    "menubar",
    "personalbar",
    "scrollbars",
    "statusbar",
    "toolbar",
];
const WINDOW_CACHE_SLOT: &str = "__moliWindowBarProps";
const OWNER_SLOT: &str = "__moliBarPropOwner";
const POPUP_SLOT: &str = "__moliBarPropPopup";
const POPUP_WINDOW_SLOT: &str = "__moliBarPropPopupWindow";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::BarProp)]
struct BarPropObjectDeclaration<'s> {
    #[webapi(prototype)]
    prototype: v8::Local<'s, v8::Object>,
    #[webapi(slot = OWNER_SLOT)]
    owner: v8::Local<'s, v8::Array>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::BarProp, enumerable, receiver)]
struct BarPropPrototypeDeclaration {
    #[webapi(accessor_property, getter = visible_getter)]
    visible: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct WindowBarPropsDeclaration<'s> {
    locationbar_data: v8::Local<'s, v8::Value>,
    menubar_data: v8::Local<'s, v8::Value>,
    personalbar_data: v8::Local<'s, v8::Value>,
    scrollbars_data: v8::Local<'s, v8::Value>,
    statusbar_data: v8::Local<'s, v8::Value>,
    toolbar_data: v8::Local<'s, v8::Value>,
    #[webapi(accessor_property, enumerable, getter = bar_getter, setter = bar_setter,
        data = self.locationbar_data, setter_data = self.locationbar_data)]
    locationbar: (),
    #[webapi(accessor_property, enumerable, getter = bar_getter, setter = bar_setter,
        data = self.menubar_data, setter_data = self.menubar_data)]
    menubar: (),
    #[webapi(accessor_property, enumerable, getter = bar_getter, setter = bar_setter,
        data = self.personalbar_data, setter_data = self.personalbar_data)]
    personalbar: (),
    #[webapi(accessor_property, enumerable, getter = bar_getter, setter = bar_setter,
        data = self.scrollbars_data, setter_data = self.scrollbars_data)]
    scrollbars: (),
    #[webapi(accessor_property, enumerable, getter = bar_getter, setter = bar_setter,
        data = self.statusbar_data, setter_data = self.statusbar_data)]
    statusbar: (),
    #[webapi(accessor_property, enumerable, getter = bar_getter, setter = bar_setter,
        data = self.toolbar_data, setter_data = self.toolbar_data)]
    toolbar: (),
}

pub(super) fn install_bar_prop_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    if name == "BarProp" {
        BarPropPrototypeDeclaration::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        );
    }
}

// Keep the cache in its original realm, including after V8 detaches the
// WindowProxy. Each accessor retains this anchor; BarProp remains lazy.
pub(crate) fn install_window_bar_props<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    popup: Option<(
        u64,
        crate::window_document_identity::LightweightPopupLocalWindowId,
    )>,
) -> Result<()> {
    let empty = v8::undefined(scope).into();
    let cache = v8::Array::new_with_elements(scope, &[empty; 6]);
    if let Some((popup_id, local_window_id)) = popup {
        set_private_value(
            scope,
            cache.into(),
            POPUP_SLOT,
            v8::BigInt::new_from_u64(scope, popup_id).into(),
        );
        set_private_value(
            scope,
            cache.into(),
            POPUP_WINDOW_SLOT,
            v8::BigInt::new_from_u64(scope, local_window_id.as_u64()).into(),
        );
        let constructor = super::ensure_intrinsic_interface_constructor(scope, "BarProp")?;
        crate::util::define_non_enumerable_static_property(
            scope,
            window,
            "BarProp",
            constructor.into(),
        );
    }
    set_private_value(scope, window, WINDOW_CACHE_SLOT, cache.into());
    let locationbar_data =
        super::window_receiver::bound_callback_data(scope, 0, window, cache.into());
    let menubar_data = super::window_receiver::bound_callback_data(scope, 1, window, cache.into());
    let personalbar_data =
        super::window_receiver::bound_callback_data(scope, 2, window, cache.into());
    let scrollbars_data =
        super::window_receiver::bound_callback_data(scope, 3, window, cache.into());
    let statusbar_data =
        super::window_receiver::bound_callback_data(scope, 4, window, cache.into());
    let toolbar_data = super::window_receiver::bound_callback_data(scope, 5, window, cache.into());
    WindowBarPropsDeclaration {
        locationbar_data,
        locationbar: (),
        menubar_data,
        menubar: (),
        personalbar_data,
        personalbar: (),
        scrollbars_data,
        scrollbars: (),
        statusbar_data,
        statusbar: (),
        toolbar_data,
        toolbar: (),
    }
    .initialize(scope, window)?;
    Ok(())
}

fn bar_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((name, detached_cache)) =
        super::window_receiver::bound_callback_data_item(scope, &args, NAMES, "Window BarProp")
    else {
        return;
    };
    let Some(cache) = get_private_value(scope, args.this(), WINDOW_CACHE_SLOT)
        .or(detached_cache)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        return;
    };
    let index = NAMES
        .iter()
        .position(|candidate| *candidate == name)
        .unwrap() as u32;
    let result = (|| -> Result<v8::Local<'s, v8::Object>> {
        if let Some(value) = cache
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            return Ok(value);
        }
        let context = cache
            .get_creation_context(scope)
            .ok_or_else(|| anyhow!("BarProp cache has no realm"))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let prototype = super::ensure_intrinsic_interface_prototype(scope, "BarProp")?;
        let object = BarPropObjectDeclaration::new(prototype, cache).bind(scope)?;
        cache.set_index(scope, index, object.into());
        Ok(object)
    })();
    match result {
        Ok(object) => rv.set(object.into()),
        Err(error) => throw_error(scope, &format!("Failed to create BarProp: {error}")),
    }
}

fn bar_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((name, _)) =
        super::window_receiver::bound_callback_data_item(scope, &args, NAMES, "Window BarProp")
    else {
        return;
    };
    super::runtime_state::define_replaceable_window_property(scope, args.this(), name, args.get(0));
}

fn private_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<u64> {
    get_private_value(scope, object, slot)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map(|value| value.u64_value().0)
}

fn visible_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let visible = (|| {
        let owner = get_private_value(scope, args.this(), OWNER_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
        let host_ptr = context_host_ptr_from_global_bridge(scope)?;
        let host = unsafe { &*host_ptr };
        if let Some(popup_id) = private_id(scope, owner, POPUP_SLOT) {
            let local_window_id = private_id(scope, owner, POPUP_WINDOW_SLOT)?;
            return Some(
                host.current_lightweight_popup_local_window_id(popup_id)
                    .is_some_and(|current| current.as_u64() == local_window_id)
                    && host.lightweight_popup_bar_props_visible(popup_id),
            );
        }
        let context = owner.get_creation_context(scope)?;
        let identity = host.window_execution_context_identity_for_access_check(context)?;
        if !host.window_execution_context_identity_is_current(identity) {
            return Some(false);
        }
        let mut dispatch = identity.dispatch_scope();
        loop {
            match dispatch {
                OwnerDispatchScope::Top => return Some(true),
                OwnerDispatchScope::LightweightPopup(id) => {
                    return Some(host.lightweight_popup_bar_props_visible(id));
                }
                OwnerDispatchScope::Child(handle) => {
                    if !host.child_browsing_context_is_live(handle) {
                        return Some(false);
                    }
                    if let Some(id) = host.child_browsing_context_popup_owner_id(handle) {
                        return Some(host.lightweight_popup_bar_props_visible(id));
                    }
                    dispatch = host
                        .child_browsing_context_parent_handle(handle)
                        .map(OwnerDispatchScope::Child)
                        .unwrap_or(OwnerDispatchScope::Top);
                }
            }
        }
    })()
    .unwrap_or(false);
    rv.set_bool(visible);
}
