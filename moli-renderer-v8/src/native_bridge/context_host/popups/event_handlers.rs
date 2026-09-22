use super::{
    JsContextHost, LIGHTWEIGHT_POPUP_EVENT_LISTENERS_SLOT, lightweight_popup_id_from_window,
};
use crate::{
    context_bootstrap::{
        WINDOW_EVENT_HANDLER_PROPERTIES, install_simple_event_target_ordered_handlers,
        simple_object_event_activate_uncompiled_handler, simple_object_event_set_ordered_handler,
    },
    definitions::define_function_accessor_property,
    document_runtime::DomHandle,
    native_bridge::OwnerDispatchScope,
    util::{context_host_ptr_from_global_bridge, get_private_value, set_private_value, v8str},
};
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct PopupWindowContentEventHandlers {
    next_revision: u64,
    attributes: HashMap<&'static str, PopupWindowContentEventHandler>,
}

struct PopupWindowContentEventHandler {
    revision: u64,
    uncompiled: Option<PopupWindowEventHandlerSource>,
}

struct PopupWindowEventHandlerSource {
    text: String,
    base_url: url::Url,
}

impl JsContextHost {
    pub(crate) fn lightweight_popup_event_handler_property_value<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        popup_id: u64,
        property_name: &str,
    ) -> Option<v8::Local<'s, v8::Value>> {
        let window = self.lightweight_popup_window(scope, popup_id)?;
        let pending = self
            .lightweight_popup_document_record_mut(popup_id)
            .and_then(|document| {
                document
                    .content_event_handlers
                    .attributes
                    .get_mut(property_name)
            })
            .and_then(|entry| {
                entry
                    .uncompiled
                    .take()
                    .map(|source| (source, entry.revision))
            });
        let Some((source, revision)) = pending else {
            return Some(lightweight_popup_event_handler_value(
                scope,
                window,
                property_name,
            ));
        };
        let target = self.current_popup_window_event_target(popup_id)?;
        let property_name = WINDOW_EVENT_HANDLER_PROPERTIES
            .iter()
            .copied()
            .find(|name| *name == property_name)?;
        if !self.ensure_lightweight_popup_execution_context(scope, popup_id) {
            return Some(v8::null(scope).into());
        }
        let context = window.get_creation_context(scope)?;
        let handler = {
            let scope = &mut v8::ContextScope::new(scope, context);
            let dispatch_scope = OwnerDispatchScope::LightweightPopup(popup_id);
            let previous = dispatch_scope.enter(scope);
            // The pending owner's slot is already null, but its listener is
            // still active. Error reporting may read it or replace it reentrantly.
            let handler = crate::native_bridge::element::compile_window_event_attribute_handler(
                scope,
                self as *mut JsContextHost,
                dispatch_scope,
                &source.base_url,
                &source.text,
                property_name
                    .strip_prefix("on")
                    .expect("Window handler name"),
            );
            let still_current = self.popup_window_event_target_is_current(target)
                && self
                    .lightweight_popup_document_record(popup_id)
                    .and_then(|document| {
                        document
                            .content_event_handlers
                            .attributes
                            .get(property_name)
                    })
                    .is_some_and(|entry| entry.revision == revision && entry.uncompiled.is_none());
            let handler = if let Some(handler) = handler.filter(|_| still_current) {
                self.lightweight_popup_document_record_mut(popup_id)
                    .expect("current popup Document")
                    .content_event_handlers
                    .attributes
                    .remove(property_name);
                set_lightweight_popup_event_handler_value(
                    scope,
                    window,
                    property_name,
                    handler.into(),
                );
                Some(v8::Global::new(scope, handler))
            } else {
                None
            };
            dispatch_scope.restore(scope, previous);
            handler
        };
        Some(
            handler
                .map(|handler| v8::Local::new(scope, &handler).into())
                .unwrap_or_else(|| v8::null(scope).into()),
        )
    }

    pub(crate) fn set_lightweight_popup_event_handler_property<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        popup_id: u64,
        property_name: &str,
        handler: Option<v8::Local<'s, v8::Object>>,
    ) {
        let Some(window) = self.lightweight_popup_window(scope, popup_id) else {
            return;
        };
        let Some(property_name) = WINDOW_EVENT_HANDLER_PROPERTIES
            .iter()
            .copied()
            .find(|name| *name == property_name)
        else {
            return;
        };
        if let Some(document) = self.lightweight_popup_document_record_mut(popup_id) {
            document
                .content_event_handlers
                .attributes
                .remove(property_name);
        }
        let value = handler
            .map(v8::Local::<v8::Value>::from)
            .unwrap_or_else(|| v8::null(scope).into());
        set_lightweight_popup_event_handler_value(scope, window, property_name, value);
    }

    pub(crate) fn set_lightweight_popup_event_handler_content_attribute<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        popup_id: u64,
        event_type: &str,
        owner: Option<DomHandle>,
    ) {
        let Some(property_name) = WINDOW_EVENT_HANDLER_PROPERTIES
            .iter()
            .copied()
            .find(|name| name.strip_prefix("on") == Some(event_type))
        else {
            return;
        };
        let Some(owner) = owner else {
            self.set_lightweight_popup_event_handler_property(scope, popup_id, property_name, None);
            return;
        };
        let Some(window) = self.lightweight_popup_window(scope, popup_id) else {
            return;
        };
        // Save the raw handler when its attribute changes. The source element
        // may be adopted and mutated before this Window first reads the value.
        let Some(text) = self.dom_host().get_attribute(owner, property_name) else {
            return;
        };
        let Some(document) = self.lightweight_popup_document_record_mut(popup_id) else {
            return;
        };
        let source = PopupWindowEventHandlerSource {
            text,
            base_url: document.state.base_url.clone(),
        };
        let handlers = &mut document.content_event_handlers;
        handlers.next_revision = handlers
            .next_revision
            .checked_add(1)
            .expect("popup content handler revision exhausted");
        handlers.attributes.insert(
            property_name,
            PopupWindowContentEventHandler {
                revision: handlers.next_revision,
                uncompiled: Some(source),
            },
        );
        let null = v8::null(scope).into();
        set_private_value(scope, window, property_name, null);
        simple_object_event_activate_uncompiled_handler(
            scope,
            window,
            LIGHTWEIGHT_POPUP_EVENT_LISTENERS_SLOT,
            event_type,
            property_name,
        );
    }
}

pub(super) fn install_lightweight_popup_event_handler_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) {
    install_simple_event_target_ordered_handlers(scope, window);
    for property_name in WINDOW_EVENT_HANDLER_PROPERTIES {
        let data = v8str(scope, property_name).into();
        define_function_accessor_property(
            scope,
            window,
            property_name,
            lightweight_popup_event_handler_getter,
            Some(data),
            lightweight_popup_event_handler_setter,
            Some(data),
            v8::PropertyAttribute::NONE,
        )
        .expect("lightweight popup Window event handler accessor should initialize");
    }
}

fn lightweight_popup_event_handler_name<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<&'static str> {
    let requested = data.to_string(scope)?.to_rust_string_lossy(scope);
    WINDOW_EVENT_HANDLER_PROPERTIES
        .iter()
        .copied()
        .find(|candidate| *candidate == requested)
}

fn lightweight_popup_event_handler_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(popup_id) = lightweight_popup_id_from_window(scope, args.this()) else {
        rv.set_null();
        return;
    };
    let Some(property_name) = lightweight_popup_event_handler_name(scope, args.data()) else {
        rv.set_null();
        return;
    };
    let value = context_host_ptr_from_global_bridge(scope).and_then(|host_ptr| {
        unsafe { &mut *host_ptr }.lightweight_popup_event_handler_property_value(
            scope,
            popup_id,
            property_name,
        )
    });
    rv.set(value.unwrap_or_else(|| v8::null(scope).into()));
}

fn lightweight_popup_event_handler_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    property_name: &str,
) -> v8::Local<'s, v8::Value> {
    get_private_value(scope, window, property_name)
        .filter(|value| value.is_object())
        .unwrap_or_else(|| v8::null(scope).into())
}

fn lightweight_popup_event_handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(popup_id) = lightweight_popup_id_from_window(scope, args.this()) else {
        rv.set_undefined();
        return;
    };
    let Some(property_name) = lightweight_popup_event_handler_name(scope, args.data()) else {
        rv.set_undefined();
        return;
    };
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let handler = v8::Local::<v8::Object>::try_from(args.get(0)).ok();
        unsafe { &mut *host_ptr }.set_lightweight_popup_event_handler_property(
            scope,
            popup_id,
            property_name,
            handler,
        );
    }
    rv.set_undefined();
}

fn set_lightweight_popup_event_handler_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    property_name: &'static str,
    value: v8::Local<'s, v8::Value>,
) {
    let stored = if value.is_object() {
        value
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, window, property_name, stored);
    simple_object_event_set_ordered_handler(
        scope,
        window,
        LIGHTWEIGHT_POPUP_EVENT_LISTENERS_SLOT,
        property_name.strip_prefix("on").unwrap_or(property_name),
        property_name,
        stored.is_object(),
    );
}

pub(super) fn clear_lightweight_popup_window_document_event_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) {
    if let Some(popup_id) = lightweight_popup_id_from_window(scope, window)
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(document) =
            unsafe { &mut *host_ptr }.lightweight_popup_document_record_mut(popup_id)
    {
        document.content_event_handlers.attributes.clear();
    }
    let undefined = v8::undefined(scope);
    set_private_value(
        scope,
        window,
        LIGHTWEIGHT_POPUP_EVENT_LISTENERS_SLOT,
        undefined.into(),
    );
    let null = v8::null(scope).into();
    for name in WINDOW_EVENT_HANDLER_PROPERTIES {
        // Reset the shared handler state even if script has replaced the
        // public accessor. Document retirement must not invoke author setters.
        set_private_value(scope, window, name, null);
    }
}
