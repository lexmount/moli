//! Native access-check handlers. Static members never retain a target Window
//! or an accessing realm; their functions are V8's per-context template instances.

use super::*;
use crate::context_bootstrap;
use moli_webapi_declare::{WebApiCrossOrigin, WebApiCrossOriginDeclaration};

// Fields are declaration metadata; no runtime object carries this state.
#[allow(dead_code)]
#[derive(WebApiCrossOrigin)]
#[webapi(plain, receiver = context_bootstrap::is_window_receiver)]
pub(super) struct WindowMembers {
    #[webapi(accessor_property, getter = context_bootstrap::window_self_value)]
    window: (),
    #[webapi(accessor_property, name = "self", getter = context_bootstrap::window_self_value)]
    self_window: (),
    #[webapi(accessor_property, getter = location::window_location, setter = location::set_window_location)]
    location: (),
    #[webapi(method, callback = context_bootstrap::window_close_callback)]
    close: (),
    #[webapi(accessor_property, getter = closed_value)]
    closed: (),
    #[webapi(method, callback = context_bootstrap::window_noop_callback)]
    focus: (),
    #[webapi(method, callback = context_bootstrap::window_noop_callback)]
    blur: (),
    #[webapi(accessor_property, getter = context_bootstrap::window_frames_value)]
    frames: (),
    #[webapi(accessor_property, getter = length_value)]
    length: (),
    #[webapi(accessor_property, getter = top_value)]
    top: (),
    #[webapi(accessor_property, getter = context_bootstrap::window_opener_value)]
    opener: (),
    #[webapi(accessor_property, getter = parent_value)]
    parent: (),
    #[webapi(method, length = 1, callback = window_host::window_post_message_callback)]
    post_message: (),
}

// Fields are declaration metadata; no runtime object carries this state.
#[allow(dead_code)]
#[derive(WebApiCrossOrigin)]
#[webapi(plain, receiver = is_location_receiver)]
pub(super) struct LocationMembers {
    #[webapi(accessor_property, setter = set_href_value)]
    href: (),
    #[webapi(method, length = 1, callback = cross_origin_location_replace_callback)]
    replace: (),
}

fn is_location_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    context_bootstrap::is_native_location(scope, receiver)
        || is_cross_origin_location_proxy(scope, receiver)
}

fn set_href_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) -> Option<()> {
    if !context_bootstrap::navigate_borrowed_location(
        scope,
        receiver,
        value,
        context_bootstrap::LocationNavigationKind::Assign,
    ) {
        cross_origin_location_navigate(scope, receiver, value);
    }
    Some(())
}

/// A parked proxy has no LocalWindow identity yet. Its browsing context still
/// supplies parent/top and children while a replacement document is pending.
struct WindowTarget<'s> {
    window: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Context>,
    host: *mut JsContextHost,
    child: Option<DomHandle>,
    closed: bool,
}

impl<'s> WindowTarget<'s> {
    fn resolve(
        scope: &mut v8::PinScope<'s, '_>,
        receiver: v8::Local<'s, v8::Object>,
    ) -> Option<Self> {
        let context = receiver.get_creation_context(scope)?;
        let host = crate::util::context_host_ptr_from_context_slot(context)?;
        let owner = unsafe { &*host };
        let identity = owner.window_execution_context_identity_for_access_check(context);
        let current = identity
            .filter(|identity| owner.window_execution_context_identity_is_current(*identity));
        if current.is_some_and(|identity| {
            !owner.window_execution_context_identity_is_default_world(identity)
        }) {
            return None;
        }
        let facade = context
            .get_slot::<ChildWindowProxyFacadeContextHandle>()
            .map(|handle| handle.0);
        let child = current
            .and_then(|identity| identity.dispatch_scope().child_window())
            .or(facade);
        let closed = if let Some(handle) = facade {
            !owner.child_browsing_context_is_live(handle)
        } else if current.is_some()
            || context
                .get_slot::<crate::util::MainDefaultWindowContext>()
                .is_some()
        {
            owner.browsing_context_is_closed()
        } else {
            true
        };
        Some(Self {
            window: context.global(scope),
            context,
            host,
            child,
            closed,
        })
    }

    fn sync_children(&self, scope: &mut v8::PinScope<'s, '_>) {
        let host = unsafe { &mut *self.host };
        let document = match self.child {
            Some(handle) => host.child_browsing_context_document_handle(handle),
            None => Some(host.document_handle()),
        };
        let scope = &mut v8::ContextScope::new(scope, self.context);
        if let Some(document) = document {
            host.sync_child_browsing_context_subtree(scope, document);
        }
    }

    fn children(&self, scope: &mut v8::PinScope<'s, '_>) -> Vec<DomHandle> {
        if self.closed {
            return Vec::new();
        }
        self.sync_children(scope);
        let host = unsafe { &*self.host };
        match self.child {
            Some(handle) => host
                .child_browsing_context_direct_host_handles(handle)
                .into_iter()
                .filter(|handle| host.child_browsing_context_is_live(*handle))
                .collect(),
            None => host.top_level_child_browsing_context_handles_in_frame_tree_order(),
        }
    }

    fn indexed(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        index: u32,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let handle = self.children(scope).get(index as usize).copied()?;
        unsafe { &mut *self.host }
            .child_browsing_context_window_proxy_for_accessing_context(scope, handle)
    }

    fn named(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        name: &str,
    ) -> Option<v8::Local<'s, v8::Object>> {
        if self.closed || name.is_empty() {
            return None;
        }
        self.sync_children(scope);
        let host = unsafe { &mut *self.host };
        let handle = host.child_browsing_context_named_child_handle(self.child, name)?;
        host.child_browsing_context_window_proxy_for_accessing_context(scope, handle)
    }
}

fn closed_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    if let Some(target) = WindowTarget::resolve(scope, receiver) {
        return Some(v8::Boolean::new(scope, target.closed).into());
    }
    context_bootstrap::window_closed_value(scope, receiver)
}

fn length_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    if let Some(context) = receiver.get_creation_context(scope)
        && let Some(handle) = context
            .get_slot::<ChildWindowProxyFacadeContextHandle>()
            .map(|handle| handle.0)
        && let Some(host) = crate::util::context_host_ptr_from_context_slot(context)
    {
        return Some(
            v8::Number::new(
                scope,
                unsafe { &mut *host }.child_browsing_context_child_frame_count(handle) as f64,
            )
            .into(),
        );
    }
    context_bootstrap::window_length_value(scope, receiver)
}

fn parent_top_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    top: bool,
) -> Option<v8::Local<'s, v8::Value>> {
    if let Some(context) = receiver.get_creation_context(scope)
        && let Some(handle) = context
            .get_slot::<ChildWindowProxyFacadeContextHandle>()
            .map(|handle| handle.0)
        && let Some(host) = crate::util::context_host_ptr_from_context_slot(context)
    {
        let records = &unsafe { &*host }.child_window_proxy_records;
        return if top {
            records.browsing_context_top(scope, handle)
        } else {
            records.browsing_context_parent(scope, handle)
        }
        .map(Into::into);
    }
    if top {
        context_bootstrap::window_top_value(scope, receiver)
    } else {
        context_bootstrap::window_parent_value(scope, receiver)
    }
}

fn parent_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    parent_top_value(scope, receiver, false)
}
fn top_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    parent_top_value(scope, receiver, true)
}

fn member_name(scope: &mut v8::PinScope<'_, '_>, key: v8::Local<'_, v8::Name>) -> Option<String> {
    v8::Local::<v8::String>::try_from(key)
        .ok()
        .map(|key| key.to_rust_string_lossy(scope))
}

pub(super) fn is_fallback_key(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
) -> bool {
    member_name(scope, key).as_deref() == Some("then")
        || [
            v8::Symbol::get_to_string_tag(scope),
            v8::Symbol::get_has_instance(scope),
            v8::Symbol::get_is_concat_spreadable(scope),
        ]
        .into_iter()
        .any(|symbol| key.strict_equals(symbol.into()))
}

fn value_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    enumerable: bool,
) -> Option<v8::Local<'s, v8::Value>> {
    CrossOriginPropertyDescriptorDeclaration::new(value, false, enumerable, true)
        .bind(scope)
        .ok()
        .map(Into::into)
}

pub(super) fn child_window_cross_origin_named_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(target) = WindowTarget::resolve(scope, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    let name = member_name(scope, key);
    if let Some(member) = name
        .as_deref()
        .and_then(WindowMembers::cross_origin_property)
    {
        let context = cross_origin_accessing_context(scope);
        let scope = &mut v8::ContextScope::new(scope, context);
        if let Some(value) = member.get(scope, target.window) {
            rv.set(value);
        }
        return v8::Intercepted::kYes;
    }
    if let Some(value) = name.as_deref().and_then(|name| target.named(scope, name)) {
        rv.set(value.into());
        return v8::Intercepted::kYes;
    }
    if is_fallback_key(scope, key) {
        rv.set_undefined();
        return v8::Intercepted::kYes;
    }
    v8::Intercepted::kNo
}

pub(super) fn child_window_cross_origin_named_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    value: v8::Local<'s, v8::Value>,
    args: v8::PropertyCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let Some(member) = member_name(scope, key)
        .as_deref()
        .and_then(WindowMembers::cross_origin_property)
        .filter(|member| member.has_setter())
    else {
        return v8::Intercepted::kNo;
    };
    let Some(target) = WindowTarget::resolve(scope, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    let _ = member.set(scope, target.window, value);
    v8::Intercepted::kYes
}

pub(super) fn child_window_cross_origin_named_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    let Some(target) = WindowTarget::resolve(scope, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    let name = member_name(scope, key);
    if name
        .as_deref()
        .and_then(WindowMembers::cross_origin_property)
        .is_some()
        || is_fallback_key(scope, key)
        || name.and_then(|name| target.named(scope, &name)).is_some()
    {
        rv.set_int32(v8::PropertyAttribute::DONT_ENUM.as_u32() as i32);
        return v8::Intercepted::kYes;
    }
    v8::Intercepted::kNo
}

pub(super) fn child_window_cross_origin_named_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    if WindowTarget::resolve(scope, args.holder()).is_none() {
        return;
    }
    let mut names: Vec<v8::Local<v8::Value>> = WindowMembers::cross_origin_properties()
        .iter()
        .map(|member| v8str(scope, member.name()).into())
        .collect();
    // Named child access is dynamic, and never creates own properties on the
    // target Window. The cross-origin own-key list contains static members.
    names.push(v8str(scope, "then").into());
    for symbol in [
        v8::Symbol::get_to_string_tag(scope),
        v8::Symbol::get_has_instance(scope),
        v8::Symbol::get_is_concat_spreadable(scope),
    ] {
        names.push(symbol.into());
    }
    let context = cross_origin_accessing_context(scope);
    let scope = &mut v8::ContextScope::new(scope, context);
    rv.set(v8::Array::new_with_elements(scope, &names));
}

pub(super) fn child_window_cross_origin_named_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(target) = WindowTarget::resolve(scope, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    let name = member_name(scope, key);
    let context = cross_origin_accessing_context(scope);
    let scope = &mut v8::ContextScope::new(scope, context);
    if let Some(member) = name
        .as_deref()
        .and_then(WindowMembers::cross_origin_property)
    {
        if let Some(descriptor) = member.descriptor(scope) {
            rv.set(descriptor.into());
        }
        return v8::Intercepted::kYes;
    }
    let value = name
        .as_deref()
        .and_then(|name| target.named(scope, name))
        .map(Into::into)
        .or_else(|| is_fallback_key(scope, key).then(|| v8::undefined(scope).into()));
    if let Some(value) = value {
        if let Some(descriptor) = value_descriptor(scope, value, false) {
            rv.set(descriptor);
        }
    } else {
        throw_cross_origin_location_security_error(scope);
    }
    v8::Intercepted::kYes
}

pub(super) fn child_window_cross_origin_indexed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    if let Some(value) =
        WindowTarget::resolve(scope, args.holder()).and_then(|target| target.indexed(scope, index))
    {
        rv.set(value.into());
        return v8::Intercepted::kYes;
    }
    v8::Intercepted::kNo
}

pub(super) fn child_window_cross_origin_indexed_setter<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _index: u32,
    _value: v8::Local<'s, v8::Value>,
    _args: v8::PropertyCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    v8::Intercepted::kNo
}

pub(super) fn child_window_cross_origin_indexed_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    if let Some(target) = WindowTarget::resolve(scope, args.holder())
        && (index as usize) < target.children(scope).len()
    {
        rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
        return v8::Intercepted::kYes;
    }
    v8::Intercepted::kNo
}

pub(super) fn child_window_cross_origin_indexed_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let Some(target) = WindowTarget::resolve(scope, args.holder()) else {
        return;
    };
    let count = target.children(scope).len() as u32;
    if let Some(array) = serialize_v8_iter_array(scope, 0..count) {
        rv.set(array);
    }
}

pub(super) fn child_window_cross_origin_indexed_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(target) = WindowTarget::resolve(scope, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    let Some(handle) = target.children(scope).get(index as usize).copied() else {
        throw_cross_origin_location_security_error(scope);
        return v8::Intercepted::kYes;
    };
    if let Some(value) = unsafe { &mut *target.host }
        .child_browsing_context_window_proxy_for_accessing_context(scope, handle)
    {
        let context = cross_origin_accessing_context(scope);
        let scope = &mut v8::ContextScope::new(scope, context);
        if let Some(descriptor) = value_descriptor(scope, value.into(), true) {
            rv.set(descriptor);
        }
    }
    v8::Intercepted::kYes
}
