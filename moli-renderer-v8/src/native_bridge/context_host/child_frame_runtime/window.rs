use super::{
    super::{JsContextHost, PendingWindowMessageEndpoint, WindowExecutionContextIdentity},
    cross_origin_property_descriptor_map::realm_local_cross_origin_function,
};
use crate::{
    browsing_context_model::{
        BrowsingContextId, BrowsingContextKind, TopLevelWindowProxyEndpointId,
    },
    context_bootstrap::{
        CHILD_BROWSING_CONTEXT_HANDLE_SLOT, LocationNavigationKind, WINDOW_OPENER_SLOT,
        dispatch_cross_document_navigation_navigate_event_for_window,
        navigate_top_level_window_location_from_cross_origin,
        resolve_location_navigation_target_against_entered_base,
    },
    definitions::define_get_set_property,
    document_runtime::DomHandle,
    document_script_scheduler::FrameDocumentClassicScriptSchedulerWork,
    native_bridge::{
        bridge::throw_dom_exception,
        child_window_surface::bind_materialized_child_window_indexed_db_factory,
        helpers::set_object_slot,
    },
    util::{
        context_host_ptr_from_context_slot, context_host_ptr_from_global_bridge, get_private_value,
        new_null_prototype_object, serialize_v8_array, serialize_v8_iter_array, set_null_prototype,
        set_private_value, throw_type_error, v8_string, v8str,
    },
    webidl, window_host,
};
use moli_webapi_declare::WebApiObject;
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

#[derive(Clone, Copy)]
struct ChildWindowProxyFacadeContextHandle(DomHandle);

#[derive(Default)]
pub(in crate::native_bridge::context_host) struct ChildWindowProxyRecords {
    records: HashMap<BrowsingContextId, ChildWindowProxyRecord>,
    context_ids_by_owner_handle: HashMap<DomHandle, BrowsingContextId>,
    // Detached iframe wrappers are an identity lookup, not a native owner.
    // The iframe wrapper's private cache or another author reference keeps a
    // live value strong; these maps must not keep an otherwise unreachable
    // child realm alive through the page host.
    detached_content_window_wrappers: HashMap<DomHandle, v8::Weak<v8::Object>>,
    detached_content_document_wrappers: HashMap<DomHandle, v8::Weak<v8::Object>>,
}

#[derive(Default)]
struct ChildWindowProxyRecord {
    live_window_wrapper: Option<v8::Global<v8::Object>>,
    facade_context: Option<v8::Global<v8::Context>>,
    browsing_context_parent_window: Option<v8::Global<v8::Object>>,
    browsing_context_top_window: Option<v8::Global<v8::Object>>,
    cross_origin_endpoint_projections:
        HashMap<PendingWindowMessageEndpoint, v8::Global<v8::Object>>,
    realm_top_window_wrapper: Option<v8::Global<v8::Object>>,
    // Once any Realm can retain this browsing context's WindowProxy, later
    // navigation must rebind that exact shell instead of replacing it.
    window_proxy_exposed: bool,
    cross_origin_window_proxy: Option<v8::Global<v8::Object>>,
    cross_origin_access_surface: Option<v8::Global<v8::Object>>,
    default_execution_context_id: Option<i64>,
}

impl ChildWindowProxyRecords {
    pub(in crate::native_bridge::context_host) fn bind_nested_browsing_context(
        &mut self,
        handle: DomHandle,
        browsing_context_id: BrowsingContextId,
    ) {
        assert_eq!(
            browsing_context_id.kind(),
            BrowsingContextKind::Nested,
            "child WindowProxy adapter requires a nested browsing context"
        );
        self.context_ids_by_owner_handle
            .retain(|bound_handle, bound_id| {
                *bound_handle == handle || *bound_id != browsing_context_id
            });
        let previous_id = self
            .context_ids_by_owner_handle
            .insert(handle, browsing_context_id);
        if let Some(previous_id) = previous_id
            && previous_id != browsing_context_id
            && !self.records.contains_key(&browsing_context_id)
            && let Some(record) = self.records.remove(&previous_id)
        {
            // Preserve the current child observable behavior when an owner
            // element is rebound while giving the record its authoritative
            // context-identity key.
            self.records.insert(browsing_context_id, record);
        }
    }

    fn context_id(&self, handle: DomHandle) -> Option<BrowsingContextId> {
        self.context_ids_by_owner_handle.get(&handle).copied()
    }

    fn record(&self, handle: DomHandle) -> Option<&ChildWindowProxyRecord> {
        self.records.get(&self.context_id(handle)?)
    }

    fn existing_record_mut(&mut self, handle: DomHandle) -> Option<&mut ChildWindowProxyRecord> {
        let context_id = self.context_id(handle)?;
        self.records.get_mut(&context_id)
    }

    fn record_mut(&mut self, handle: DomHandle) -> &mut ChildWindowProxyRecord {
        let context_id = self
            .context_id(handle)
            .expect("child WindowProxy materialization requires a bound browsing-context identity");
        self.records.entry(context_id).or_default()
    }

    pub(in crate::native_bridge::context_host) fn shell<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let record = self.record(handle)?;
        record
            .live_window_wrapper
            .as_ref()
            .or(record.cross_origin_window_proxy.as_ref())
            .map(|window| v8::Local::new(scope, window))
    }

    pub(in crate::native_bridge::context_host) fn live_window<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.record(handle)?
            .live_window_wrapper
            .as_ref()
            .map(|window| v8::Local::new(scope, window))
    }

    pub(in crate::native_bridge::context_host) fn has_live_window(
        &self,
        handle: DomHandle,
    ) -> bool {
        self.record(handle)
            .is_some_and(|record| record.live_window_wrapper.is_some())
    }

    pub(in crate::native_bridge::context_host) fn set_facade_context(
        &mut self,
        handle: DomHandle,
        context: v8::Global<v8::Context>,
    ) {
        let record = self.record_mut(handle);
        record.facade_context = Some(context);
        // Endpoint projections are wrappers in the accessing child realm. A
        // replacement LocalWindow gets fresh wrappers even though the stable
        // target WindowProxy identities remain the same.
        record.cross_origin_endpoint_projections.clear();
    }

    pub(in crate::native_bridge::context_host) fn take_facade_context(
        &mut self,
        handle: DomHandle,
    ) -> Option<v8::Global<v8::Context>> {
        self.existing_record_mut(handle)?.facade_context.take()
    }

    pub(in crate::native_bridge::context_host) fn promote_shell_to_live(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        window: v8::Local<'_, v8::Object>,
    ) {
        let record = self.record_mut(handle);
        record.cross_origin_window_proxy = None;
        record.cross_origin_access_surface = None;
        record.live_window_wrapper = Some(v8::Global::new(scope, window));
    }

    pub(in crate::native_bridge::context_host) fn set_browsing_context_parent_top(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        parent: v8::Local<'_, v8::Object>,
        top: v8::Local<'_, v8::Object>,
    ) {
        let record = self.record_mut(handle);
        record
            .browsing_context_parent_window
            .get_or_insert_with(|| v8::Global::new(scope, parent));
        record
            .browsing_context_top_window
            .get_or_insert_with(|| v8::Global::new(scope, top));
    }

    pub(in crate::native_bridge::context_host) fn browsing_context_parent<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.record(handle)?
            .browsing_context_parent_window
            .as_ref()
            .map(|parent| v8::Local::new(scope, parent))
    }

    pub(in crate::native_bridge::context_host) fn browsing_context_top<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.record(handle)?
            .browsing_context_top_window
            .as_ref()
            .map(|top| v8::Local::new(scope, top))
    }

    fn cross_origin_endpoint_projection<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
        endpoint: PendingWindowMessageEndpoint,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.record(handle)?
            .cross_origin_endpoint_projections
            .get(&endpoint)
            .map(|projection| v8::Local::new(scope, projection))
    }

    fn set_cross_origin_endpoint_projection(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        endpoint: PendingWindowMessageEndpoint,
        projection: v8::Local<'_, v8::Object>,
    ) {
        self.record_mut(handle)
            .cross_origin_endpoint_projections
            .insert(endpoint, v8::Global::new(scope, projection));
    }

    pub(in crate::native_bridge::context_host) fn set_realm_top(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        top: v8::Local<'_, v8::Object>,
    ) {
        if moli_trace::window_message_trace_enabled() {
            let global = scope.get_current_context().global(scope);
            tracing::info!(
                target: "moli_window_message_trace",
                handle = handle.index(),
                top_is_target_global = top.strict_equals(global.into()),
                stage = "child_window_proxy_realm_top_installed",
            );
        }
        self.record_mut(handle).realm_top_window_wrapper = Some(v8::Global::new(scope, top));
    }

    pub(in crate::native_bridge::context_host) fn realm_top<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.record(handle)?
            .realm_top_window_wrapper
            .as_ref()
            .map(|top| v8::Local::new(scope, top))
    }

    pub(in crate::native_bridge::context_host) fn mark_window_proxy_exposed(
        &mut self,
        handle: DomHandle,
    ) {
        self.record_mut(handle).window_proxy_exposed = true;
    }

    pub(in crate::native_bridge::context_host) fn window_proxy_exposed(
        &self,
        handle: DomHandle,
    ) -> bool {
        self.record(handle)
            .is_some_and(|record| record.window_proxy_exposed)
    }

    pub(in crate::native_bridge::context_host) fn cross_origin_proxy<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.record(handle)?
            .cross_origin_window_proxy
            .as_ref()
            .map(|proxy| v8::Local::new(scope, proxy))
    }

    pub(in crate::native_bridge::context_host) fn has_cross_origin_proxy(
        &self,
        handle: DomHandle,
    ) -> bool {
        self.record(handle)
            .is_some_and(|record| record.cross_origin_window_proxy.is_some())
    }

    pub(in crate::native_bridge::context_host) fn set_cross_origin_proxy(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        proxy: v8::Local<'_, v8::Object>,
    ) {
        self.record_mut(handle).cross_origin_window_proxy = Some(v8::Global::new(scope, proxy));
    }

    fn set_cross_origin_access_surface(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        access_surface: v8::Local<'_, v8::Object>,
    ) {
        self.record_mut(handle).cross_origin_access_surface =
            Some(v8::Global::new(scope, access_surface));
    }

    fn cross_origin_handler_data<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>)> {
        let record = self.record(handle)?;
        let access_surface = record.cross_origin_access_surface.as_ref()?;
        let window_proxy = record
            .live_window_wrapper
            .as_ref()
            .or(record.cross_origin_window_proxy.as_ref())?;
        Some((
            v8::Local::new(scope, access_surface),
            v8::Local::new(scope, window_proxy),
        ))
    }

    pub(in crate::native_bridge::context_host) fn set_default_execution_context_id(
        &mut self,
        handle: DomHandle,
        execution_context_id: i64,
    ) {
        self.record_mut(handle).default_execution_context_id = Some(execution_context_id);
    }

    pub(in crate::native_bridge::context_host) fn clear_default_execution_context_id(
        &mut self,
        handle: DomHandle,
    ) {
        let Some(record) = self.existing_record_mut(handle) else {
            return;
        };
        record.default_execution_context_id = None;
        record.realm_top_window_wrapper = None;
    }

    pub(in crate::native_bridge::context_host) fn clear_default_execution_context_id_if_matches(
        &mut self,
        handle: DomHandle,
        expected_execution_context_id: i64,
    ) -> bool {
        // A replacement Document reuses the browsing-context handle. Its context
        // binding must survive delayed retirement of the previous LocalWindow.
        let Some(record) = self.existing_record_mut(handle) else {
            return false;
        };
        if record.default_execution_context_id != Some(expected_execution_context_id) {
            return false;
        }
        record.default_execution_context_id = None;
        record.realm_top_window_wrapper = None;
        true
    }

    pub(in crate::native_bridge::context_host) fn default_execution_context_id(
        &self,
        handle: DomHandle,
    ) -> Option<i64> {
        self.record(handle)?.default_execution_context_id
    }

    pub(in crate::native_bridge::context_host) fn clear_live_records(&mut self, handle: DomHandle) {
        if let Some(context_id) = self.context_ids_by_owner_handle.remove(&handle) {
            self.records.remove(&context_id);
        }
    }

    pub(in crate::native_bridge::context_host) fn retain_live_records(
        &mut self,
        live_handles: &HashSet<DomHandle>,
    ) {
        self.context_ids_by_owner_handle
            .retain(|handle, _| live_handles.contains(handle));
        let live_context_ids = self
            .context_ids_by_owner_handle
            .values()
            .copied()
            .collect::<HashSet<_>>();
        self.records
            .retain(|context_id, _| live_context_ids.contains(context_id));
    }

    pub(in crate::native_bridge::context_host) fn detached_content_document<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.detached_content_document_wrappers
            .get(&handle)
            .and_then(|wrapper| wrapper.to_local(scope))
    }

    pub(in crate::native_bridge::context_host) fn set_detached_content_document(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        document: v8::Local<'_, v8::Object>,
    ) {
        self.detached_content_document_wrappers
            .insert(handle, v8::Weak::new(scope, document));
    }

    pub(in crate::native_bridge::context_host) fn detached_content_window<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.detached_content_window_wrappers
            .get(&handle)
            .and_then(|wrapper| wrapper.to_local(scope))
    }

    pub(in crate::native_bridge::context_host) fn set_detached_content_window(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        window: v8::Local<'_, v8::Object>,
    ) {
        self.detached_content_window_wrappers
            .insert(handle, v8::Weak::new(scope, window));
    }

    pub(in crate::native_bridge::context_host) fn clear_detached_content_surfaces(
        &mut self,
        handle: DomHandle,
    ) {
        self.detached_content_window_wrappers.remove(&handle);
        self.detached_content_document_wrappers.remove(&handle);
    }
}

const CROSS_ORIGIN_WINDOW_LOCATION_SLOT: &str = "__moliCrossOriginWindowLocation";
const CROSS_ORIGIN_LOCATION_PROXY_SLOT: &str = "__moliCrossOriginLocationProxy";
const CROSS_ORIGIN_LOCATION_PROXY_SELF_SLOT: &str = "__moliCrossOriginLocationProxySelf";
const CROSS_ORIGIN_WINDOW_EXPOSED_PROPERTY_NAMES: &[&str] = &[
    "window",
    "self",
    "location",
    "close",
    "closed",
    "focus",
    "blur",
    "frames",
    "length",
    "top",
    "opener",
    "parent",
    "postMessage",
];
const DETACHED_CROSS_ORIGIN_WINDOW_PROXY_SLOT: &str = "__moliDetachedCrossOriginWindowProxy";
const CROSS_ORIGIN_TOP_WINDOW_PROXY_SLOT: &str = "__moliCrossOriginTopWindowProxy";
const CROSS_ORIGIN_RELATED_TOP_WINDOW_GROUP_SLOT: &str = "__moliCrossOriginRelatedTopWindowGroup";
const CROSS_ORIGIN_RELATED_TOP_WINDOW_GENERATION_SLOT: &str =
    "__moliCrossOriginRelatedTopWindowGeneration";
const CROSS_ORIGIN_REMOTE_FRAME_PROJECTION_SLOT: &str = "__moliCrossOriginRemoteFrameProjection";
const CROSS_ORIGIN_LOCAL_TOP_WINDOW_SLOT: &str = "__moliCrossOriginLocalTopWindow";
const CLOSED_TOP_LEVEL_WINDOW_ACCESS_SURFACE_SLOT: &str = "__moliClosedTopLevelWindowAccessSurface";
const CLOSED_TOP_LEVEL_WINDOW_MARKER_SLOT: &str = "__moliClosedTopLevelWindow";
const CROSS_ORIGIN_WINDOW_NAMED_CHILD_SLOTS: &str = "__moliCrossOriginWindowNamedChildSlots";

const CROSS_ORIGIN_ACCESS_ERROR: &str =
    "Blocked a frame with a different origin from accessing a cross-origin frame.";

const CROSS_ORIGIN_REALM_WINDOW_CLOSE_SLOT: &str = "__lmCrossOriginRealmWindowClose";
const CROSS_ORIGIN_REALM_WINDOW_FOCUS_SLOT: &str = "__lmCrossOriginRealmWindowFocus";
const CROSS_ORIGIN_REALM_WINDOW_BLUR_SLOT: &str = "__lmCrossOriginRealmWindowBlur";
const CROSS_ORIGIN_REALM_WINDOW_POST_MESSAGE_SLOT: &str = "__lmCrossOriginRealmWindowPostMessage";
const CROSS_ORIGIN_REALM_WINDOW_LOCATION_GETTER_SLOT: &str =
    "__lmCrossOriginRealmWindowLocationGetter";
const CROSS_ORIGIN_REALM_WINDOW_WINDOW_GETTER_SLOT: &str = "__lmCrossOriginRealmWindowWindowGetter";
const CROSS_ORIGIN_REALM_WINDOW_FRAMES_GETTER_SLOT: &str = "__lmCrossOriginRealmWindowFramesGetter";
const CROSS_ORIGIN_REALM_WINDOW_SELF_GETTER_SLOT: &str = "__lmCrossOriginRealmWindowSelfGetter";
const CROSS_ORIGIN_REALM_WINDOW_TOP_GETTER_SLOT: &str = "__lmCrossOriginRealmWindowTopGetter";
const CROSS_ORIGIN_REALM_WINDOW_PARENT_GETTER_SLOT: &str = "__lmCrossOriginRealmWindowParentGetter";
const CROSS_ORIGIN_REALM_WINDOW_OPENER_GETTER_SLOT: &str = "__lmCrossOriginRealmWindowOpenerGetter";
const CROSS_ORIGIN_REALM_WINDOW_CLOSED_GETTER_SLOT: &str = "__lmCrossOriginRealmWindowClosedGetter";
const CROSS_ORIGIN_REALM_WINDOW_LENGTH_GETTER_SLOT: &str = "__lmCrossOriginRealmWindowLengthGetter";
const CROSS_ORIGIN_REALM_WINDOW_LOCATION_SETTER_SLOT: &str =
    "__lmCrossOriginRealmWindowLocationSetter";
const CROSS_ORIGIN_REALM_LOCATION_REPLACE_SLOT: &str = "__lmCrossOriginRealmLocationReplace";
const CROSS_ORIGIN_REALM_LOCATION_HREF_SETTER_SLOT: &str = "__lmCrossOriginRealmLocationHrefSetter";

fn cross_origin_window_method_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Option<v8::Local<'s, v8::Function>> {
    match name {
        "close" => realm_local_cross_origin_function(
            scope,
            CROSS_ORIGIN_REALM_WINDOW_CLOSE_SLOT,
            "close",
            0,
            cross_origin_window_close_callback,
        ),
        "focus" => realm_local_cross_origin_function(
            scope,
            CROSS_ORIGIN_REALM_WINDOW_FOCUS_SLOT,
            "focus",
            0,
            cross_origin_window_focus_callback,
        ),
        "blur" => realm_local_cross_origin_function(
            scope,
            CROSS_ORIGIN_REALM_WINDOW_BLUR_SLOT,
            "blur",
            0,
            cross_origin_window_noop_callback,
        ),
        "postMessage" => realm_local_cross_origin_function(
            scope,
            CROSS_ORIGIN_REALM_WINDOW_POST_MESSAGE_SLOT,
            "postMessage",
            1,
            window_host::window_post_message_callback,
        ),
        _ => None,
    }
}

fn cross_origin_window_attribute_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Option<v8::Local<'s, v8::Function>> {
    match name {
        "location" => realm_local_cross_origin_function(
            scope,
            CROSS_ORIGIN_REALM_WINDOW_LOCATION_GETTER_SLOT,
            "get location",
            0,
            cross_origin_window_location_attribute_getter_callback,
        ),
        "window" => realm_local_cross_origin_function(
            scope,
            CROSS_ORIGIN_REALM_WINDOW_WINDOW_GETTER_SLOT,
            "get window",
            0,
            cross_origin_window_window_attribute_getter_callback,
        ),
        "frames" => realm_local_cross_origin_function(
            scope,
            CROSS_ORIGIN_REALM_WINDOW_FRAMES_GETTER_SLOT,
            "get frames",
            0,
            cross_origin_window_frames_attribute_getter_callback,
        ),
        "self" => realm_local_cross_origin_function(
            scope,
            CROSS_ORIGIN_REALM_WINDOW_SELF_GETTER_SLOT,
            "get self",
            0,
            cross_origin_window_self_attribute_getter_callback,
        ),
        "top" => realm_local_cross_origin_function(
            scope,
            CROSS_ORIGIN_REALM_WINDOW_TOP_GETTER_SLOT,
            "get top",
            0,
            cross_origin_window_top_attribute_getter_callback,
        ),
        "parent" => realm_local_cross_origin_function(
            scope,
            CROSS_ORIGIN_REALM_WINDOW_PARENT_GETTER_SLOT,
            "get parent",
            0,
            cross_origin_window_parent_attribute_getter_callback,
        ),
        "opener" => realm_local_cross_origin_function(
            scope,
            CROSS_ORIGIN_REALM_WINDOW_OPENER_GETTER_SLOT,
            "get opener",
            0,
            cross_origin_window_opener_attribute_getter_callback,
        ),
        "closed" => realm_local_cross_origin_function(
            scope,
            CROSS_ORIGIN_REALM_WINDOW_CLOSED_GETTER_SLOT,
            "get closed",
            0,
            cross_origin_window_closed_attribute_getter_callback,
        ),
        "length" => realm_local_cross_origin_function(
            scope,
            CROSS_ORIGIN_REALM_WINDOW_LENGTH_GETTER_SLOT,
            "get length",
            0,
            cross_origin_window_length_attribute_getter_callback,
        ),
        _ => None,
    }
}

fn cross_origin_window_location_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Function>> {
    realm_local_cross_origin_function(
        scope,
        CROSS_ORIGIN_REALM_WINDOW_LOCATION_SETTER_SLOT,
        "set location",
        1,
        cross_origin_window_location_attribute_setter_callback,
    )
}

fn cross_origin_location_replace_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Function>> {
    realm_local_cross_origin_function(
        scope,
        CROSS_ORIGIN_REALM_LOCATION_REPLACE_SLOT,
        "replace",
        1,
        cross_origin_location_replace_callback,
    )
}

fn cross_origin_location_href_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Function>> {
    realm_local_cross_origin_function(
        scope,
        CROSS_ORIGIN_REALM_LOCATION_HREF_SETTER_SLOT,
        "set href",
        1,
        cross_origin_location_href_attribute_setter_callback,
    )
}

fn cross_origin_window_attribute_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let getter = cross_origin_window_attribute_getter_function(scope, name)?;
    let setter = if name == "location" {
        cross_origin_window_location_setter_function(scope)?.into()
    } else {
        v8::undefined(scope).into()
    };
    CrossOriginAccessorPropertyDescriptorDeclaration::new(getter.into(), setter, false, true)
        .bind(scope)
        .ok()
}

fn cross_origin_window_method_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let function = cross_origin_window_method_function(scope, name)?;
    CrossOriginPropertyDescriptorDeclaration::new(function.into(), false, false, true)
        .bind(scope)
        .ok()
}

fn cross_origin_location_replace_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let function = cross_origin_location_replace_function(scope)?;
    CrossOriginPropertyDescriptorDeclaration::new(function.into(), false, false, true)
        .bind(scope)
        .ok()
}

fn cross_origin_location_href_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let setter = cross_origin_location_href_setter_function(scope)?;
    CrossOriginAccessorPropertyDescriptorDeclaration::new(
        v8::undefined(scope).into(),
        setter.into(),
        false,
        true,
    )
    .bind(scope)
    .ok()
}

pub(crate) fn install_child_window_proxy_access_check_handlers(
    global_template: v8::Local<'_, v8::ObjectTemplate>,
) {
    global_template.set_immutable_proto();
    global_template.set_security_token_access_check_and_handlers(
        window_access_check_callback,
        v8::NamedPropertyHandlerConfiguration::new()
            .getter(child_window_cross_origin_named_getter)
            .setter(child_window_cross_origin_named_setter)
            .query(child_window_cross_origin_named_query)
            .deleter(child_window_cross_origin_named_deleter)
            .enumerator(child_window_cross_origin_named_enumerator)
            .definer(child_window_cross_origin_named_definer)
            .descriptor(child_window_cross_origin_named_descriptor),
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(child_window_cross_origin_indexed_getter)
            .setter(child_window_cross_origin_indexed_setter)
            .query(child_window_cross_origin_indexed_query)
            .deleter(child_window_cross_origin_indexed_deleter)
            .enumerator(child_window_cross_origin_indexed_enumerator)
            .definer(child_window_cross_origin_indexed_definer)
            .descriptor(child_window_cross_origin_indexed_descriptor),
    );
}

unsafe extern "C" fn window_access_check_callback(
    accessing_context: v8::Local<'_, v8::Context>,
    accessed_object: v8::Local<'_, v8::Object>,
    _data: v8::Local<'_, v8::Value>,
) -> bool {
    let scope = std::pin::pin!(unsafe { v8::CallbackScope::new(accessed_object) });
    let scope = &mut scope.init();
    window_access_is_allowed(scope, accessing_context, accessed_object)
}

pub(super) fn window_access_is_allowed(
    scope: &mut v8::PinScope<'_, '_>,
    accessing_context: v8::Local<'_, v8::Context>,
    accessed_object: v8::Local<'_, v8::Object>,
) -> bool {
    let Some(accessed_context) = accessed_object.get_creation_context(scope) else {
        return false;
    };
    if crate::util::page_context_is_disconnected(accessing_context)
        || crate::util::page_context_is_disconnected(accessed_context)
    {
        return false;
    }
    if accessing_context == accessed_context {
        return true;
    }

    let Some(accessing_host_ptr) =
        crate::util::context_host_ptr_from_context_slot(accessing_context)
    else {
        return false;
    };
    let Some(accessed_host_ptr) = crate::util::context_host_ptr_from_context_slot(accessed_context)
    else {
        return false;
    };
    let accessing_host = unsafe { &*accessing_host_ptr };
    let accessed_host = unsafe { &*accessed_host_ptr };
    accessing_host.window_realms_can_access(accessing_context, accessed_host, accessed_context)
}

impl JsContextHost {
    pub(in crate::native_bridge::context_host) fn inform_about_canceled_child_navigation_before_detach(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
    ) {
        let Some(window) = self.child_window_proxy_records.live_window(scope, handle) else {
            return;
        };
        let Some(context) = window.get_creation_context(scope) else {
            return;
        };
        let window = v8::Global::new(scope, window);
        let context = v8::Global::new(scope, context);
        let context = v8::Local::new(scope, &context);
        let child_scope = &mut v8::ContextScope::new(scope, context);
        let window = v8::Local::new(child_scope, &window);
        crate::context_bootstrap::inform_about_canceled_navigation_for_window(child_scope, window);
    }

    pub(in crate::native_bridge::context_host) fn refresh_child_window_access_surfaces_after_origin_mutation(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
    ) {
        let handles = self.child_browsing_context_handles_in_document_order();
        for handle in handles {
            let dispatch_scope = super::super::OwnerDispatchScope::Child(handle);
            let Some(owner) = self.current_window_execution_context_owner(dispatch_scope) else {
                continue;
            };
            let Some((_, context)) = self.window_execution_context(scope, owner, dispatch_scope)
            else {
                continue;
            };
            let context = v8::Global::new(scope, context);
            let child_context = v8::Local::new(scope, &context);
            let child_scope = &mut v8::ContextScope::new(scope, child_context);
            let Some(window) = self
                .child_window_proxy_records
                .live_window(child_scope, handle)
            else {
                continue;
            };
            self.sync_child_browsing_context_window_parent_top_slots(child_scope, handle, window);
        }
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Location.replace")]
struct CrossOriginLocationReplaceArgs {
    #[webidl(required, converter = "usv_string")]
    url: String,
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct CrossOriginWindowMethodsDeclaration {
    #[webapi(
        method,
        length = 1,
        callback = window_host::window_post_message_callback,
        readonly
    )]
    post_message: (),
    #[webapi(method, callback = cross_origin_window_noop_callback, readonly)]
    blur: (),
    #[webapi(method, callback = cross_origin_window_close_callback, readonly)]
    close: (),
    #[webapi(method, callback = cross_origin_window_focus_callback, readonly)]
    focus: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct CrossOriginPropertyDescriptorDeclaration<'scope> {
    value: v8::Local<'scope, v8::Value>,
    writable: bool,
    enumerable: bool,
    configurable: bool,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct CrossOriginAccessorPropertyDescriptorDeclaration<'scope> {
    get: v8::Local<'scope, v8::Value>,
    set: v8::Local<'scope, v8::Value>,
    enumerable: bool,
    configurable: bool,
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct CrossOriginLocationMethodsDeclaration {
    #[webapi(
        method,
        length = 1,
        callback = cross_origin_location_replace_callback,
        readonly
    )]
    replace: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct CrossOriginLocationProxyHandlerDeclaration {
    #[webapi(method, length = 3, callback = cross_origin_location_proxy_get_callback)]
    get: (),
    #[webapi(method, length = 2, callback = cross_origin_location_proxy_has_callback)]
    has: (),
    #[webapi(method, length = 4, callback = cross_origin_location_proxy_set_callback)]
    set: (),
    #[webapi(
        method,
        length = 2,
        callback = cross_origin_location_proxy_get_own_property_descriptor_callback
    )]
    get_own_property_descriptor: (),
    #[webapi(method, length = 2, callback = cross_origin_window_denied_callback)]
    delete_property: (),
    #[webapi(method, length = 3, callback = cross_origin_window_denied_callback)]
    define_property: (),
    #[webapi(
        method,
        length = 2,
        callback = cross_origin_location_proxy_set_prototype_of_callback
    )]
    set_prototype_of: (),
    #[webapi(
        method,
        length = 1,
        callback = cross_origin_location_proxy_prevent_extensions_callback
    )]
    prevent_extensions: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct CrossOriginWindowProxyHandlerDeclaration {
    #[webapi(method, length = 3, callback = cross_origin_window_proxy_get_callback)]
    get: (),
    #[webapi(method, length = 2, callback = cross_origin_window_proxy_has_callback)]
    has: (),
    #[webapi(method, length = 4, callback = cross_origin_window_proxy_set_callback)]
    set: (),
    #[webapi(
        method,
        length = 2,
        callback = cross_origin_window_proxy_get_own_property_descriptor_callback
    )]
    get_own_property_descriptor: (),
    #[webapi(
        method,
        length = 1,
        callback = cross_origin_window_proxy_own_keys_callback
    )]
    own_keys: (),
    #[webapi(method, length = 2, callback = cross_origin_window_denied_callback)]
    delete_property: (),
    #[webapi(method, length = 3, callback = cross_origin_window_denied_callback)]
    define_property: (),
    #[webapi(
        method,
        length = 2,
        callback = cross_origin_window_proxy_set_prototype_of_callback
    )]
    set_prototype_of: (),
    #[webapi(
        method,
        length = 1,
        callback = cross_origin_window_proxy_prevent_extensions_callback
    )]
    prevent_extensions: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct CrossOriginWindowLiveAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = cross_origin_window_closed_getter_callback,
        setter = cross_origin_window_denied_callback
    )]
    closed: (),
    #[webapi(
        accessor_property,
        getter = cross_origin_window_length_getter_callback,
        setter = cross_origin_window_denied_callback
    )]
    length: (),
    #[webapi(
        accessor_property,
        getter = cross_origin_window_location_getter_callback,
        setter = cross_origin_location_navigate_setter_callback
    )]
    location: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct CrossOriginTopLevelWindowOpenerAccessorDeclaration {
    #[webapi(
        accessor_property,
        getter = cross_origin_top_level_window_opener_getter_callback,
        setter = cross_origin_window_denied_callback
    )]
    opener: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct CrossOriginWindowLocationAccessorDeclaration {
    #[webapi(
        accessor_property,
        getter = cross_origin_window_location_getter_callback,
        setter = cross_origin_location_navigate_setter_callback
    )]
    location: (),
}

impl JsContextHost {
    pub(in crate::native_bridge::context_host) fn install_default_world_state_for_child_window(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        window: v8::Local<'_, v8::Object>,
        document: v8::Local<'_, v8::Object>,
    ) {
        let _ = document;
        self.install_default_runtime_bindings_for_child_window(scope, handle, window);
    }

    pub(crate) fn request_child_frame_realm_materialization(&mut self, handle: DomHandle) {
        let Some(owner) = self
            .frame_owner_store
            .current_child_document_task_owner(handle)
        else {
            return;
        };
        let _ = self.request_child_frame_realm_materialization_for_owner(handle, owner);
    }

    pub(crate) fn request_child_frame_realm_materialization_for_owner(
        &mut self,
        handle: DomHandle,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> Option<crate::frame_owner_model::FrameRealmMaterializationRequest> {
        if self
            .frame_owner_store
            .current_child_document_task_owner(handle)
            != Some(owner)
        {
            return None;
        }
        self.frame_owner_store.ensure_child_realm(handle)?;
        if let Some(realm_id) = self
            .frame_owner_store
            .current_reserved_realm_id_for_document_task_owner(owner)
        {
            self.bind_pending_child_modulepreload_work_to_first_realm(handle, owner, realm_id);
        }
        let request = self
            .frame_owner_store
            .request_child_realm_materialization(handle, owner)?;
        if !matches!(
            request,
            crate::frame_owner_model::FrameRealmMaterializationRequest::NewlyQueued { .. }
        ) {
            return Some(request);
        }
        let target =
            crate::page_task_queue::RendererPageChildRealmMaterializationTarget::new(handle, owner);
        if self
            .page_child_frame_task_sender()
            .send_realm_materialization(target)
            .is_err()
        {
            let _ = self
                .frame_owner_store
                .rollback_child_realm_materialization_request(handle, owner, request.realm_id());
            return None;
        }
        Some(request)
    }

    pub(crate) fn child_current_document_is_initial_empty(&self, handle: DomHandle) -> bool {
        self.frame_owner_store
            .current_child_document_creation_kind(handle)
            .is_some_and(crate::frame_owner_model::DocumentCreationKind::is_initial_empty)
    }

    pub(crate) fn retire_child_frame_realm_materialization_request(
        &mut self,
        handle: DomHandle,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> bool {
        let retired = self
            .frame_owner_store
            .retire_child_realm_materialization_request(handle, owner);
        if retired {
            self.signal_page_child_realm_materialization_reconsideration_if_installed();
        }
        retired
    }

    pub(crate) fn has_child_frame_realm_materialization_request(
        &self,
        handle: DomHandle,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> bool {
        self.frame_owner_store
            .child_realm_materialization_is_queued(handle, owner)
    }

    pub(crate) fn has_pending_child_frame_realm_materialization(&self) -> bool {
        self.frame_owner_store
            .has_queued_child_realm_materialization()
    }

    pub(crate) fn fail_child_frame_realm_materialization(
        &mut self,
        handle: DomHandle,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> bool {
        let failed = self
            .frame_owner_store
            .fail_child_realm_materialization(handle, owner);
        if failed {
            self.child_window_proxy_records
                .clear_default_execution_context_id(handle);
        }
        failed
    }

    pub(in crate::native_bridge::context_host) fn clear_live_child_window_proxy_records(
        &mut self,
        handle: DomHandle,
    ) {
        self.child_window_proxy_records.clear_live_records(handle);
    }

    pub(in crate::native_bridge::context_host) fn retain_live_child_window_proxy_records(
        &mut self,
        live_handles: &HashSet<DomHandle>,
    ) {
        self.child_window_proxy_records
            .retain_live_records(live_handles);
    }

    pub(crate) fn take_child_window_proxy_shell_for_realm<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_, ()>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let shell = self.child_window_proxy_records.shell(scope, handle)?;
        if let Some(context) = self.child_window_proxy_records.take_facade_context(handle) {
            v8::Local::new(scope, &context).detach_global();
        }
        Some(shell)
    }

    pub(crate) fn promote_child_window_proxy_shell_to_realm<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
        shell: v8::Local<'s, v8::Object>,
    ) {
        self.child_window_proxy_records
            .promote_shell_to_live(scope, handle, shell);
    }

    pub(crate) fn preserve_child_window_proxy_between_realms<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_, ()>,
        handle: DomHandle,
    ) -> bool {
        // A same-origin caller can retain functions from the retired inner
        // global. Keep that proxy detached until the replacement LocalWindow
        // takes it over so those closures continue to resolve against their
        // old inner global. Cross-origin callers can observe only the stable
        // WindowProxy whitelist, so park that proxy on a restricted facade.
        if self.top_window_can_access_child(handle) {
            return true;
        }
        let Some(window_proxy) = self.child_window_proxy_records.live_window(scope, handle) else {
            return false;
        };
        let Some(context) = self
            .bridge
            .bindings
            .attach_window_proxy_shell_to_facade(scope, window_proxy)
        else {
            return false;
        };
        crate::util::install_context_host_pointer_slot(
            context,
            self as *mut Self,
            self.context_host_lifecycle_handle(),
        );
        let previous = context.set_slot(Rc::new(ChildWindowProxyFacadeContextHandle(handle)));
        debug_assert!(previous.is_none());
        // Keep V8's unique default security token. A facade is not a real
        // LocalWindow realm, so no external context may bypass the access
        // handlers merely because it shares the pending document's origin.
        let facade_context = v8::Global::new(scope, context);
        let child_frame_count = self.child_browsing_context_child_frame_count(handle);
        let named_indices = self.child_browsing_context_child_frame_named_indices(handle);
        let parent = self
            .child_window_proxy_records
            .browsing_context_parent(scope, handle)
            .unwrap_or(window_proxy);
        let top = self
            .child_window_proxy_records
            .browsing_context_top(scope, handle)
            .unwrap_or(parent);

        let facade_scope = &mut v8::ContextScope::new(scope, context);
        let window_proxy = context.global(facade_scope);
        install_live_cross_origin_child_window_surface(
            facade_scope,
            window_proxy,
            handle,
            parent,
            top,
            window_proxy,
            child_frame_count,
            &named_indices,
        );
        let access_surface = new_null_prototype_object(facade_scope);
        install_live_cross_origin_child_window_surface(
            facade_scope,
            access_surface,
            handle,
            parent,
            top,
            window_proxy,
            child_frame_count,
            &named_indices,
        );
        self.child_window_proxy_records
            .set_cross_origin_access_surface(facade_scope, handle, access_surface);
        self.child_window_proxy_records
            .set_facade_context(handle, facade_context);
        true
    }

    pub(crate) fn child_window_proxy_shell_is_exposed(&self, handle: DomHandle) -> bool {
        self.child_window_proxy_records.window_proxy_exposed(handle)
    }

    pub(crate) fn install_child_window_proxy_cross_origin_access_surface<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
        window_proxy: v8::Local<'s, v8::Object>,
        realm_parent: v8::Local<'s, v8::Object>,
        realm_top: v8::Local<'s, v8::Object>,
    ) {
        // parent/top on a WindowProxy are browsing-context identities. Do not
        // replace them with the caller-specific projections installed on a
        // replacement inner global.
        let parent = self
            .child_window_proxy_records
            .browsing_context_parent(scope, handle)
            .unwrap_or(realm_parent);
        let top = self
            .child_window_proxy_records
            .browsing_context_top(scope, handle)
            .unwrap_or(realm_top);
        let surface = new_null_prototype_object(scope);
        // A live Document projects children through its authoritative registry
        // in the access handlers. Snapshot slots are reserved for facades that
        // have no current LocalWindow/Document owner.
        install_live_cross_origin_child_window_surface(
            scope,
            surface,
            handle,
            parent,
            top,
            window_proxy,
            0,
            &[],
        );
        self.child_window_proxy_records
            .set_cross_origin_access_surface(scope, handle, surface);
    }

    pub(crate) fn install_top_level_window_proxy_cross_origin_access_surface<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        window_proxy: v8::Local<'s, v8::Object>,
        endpoint: Option<TopLevelWindowProxyEndpointId>,
    ) {
        let surface = new_null_prototype_object(scope);
        let opener = get_private_value(scope, window_proxy, WINDOW_OPENER_SLOT)
            .unwrap_or_else(|| v8::null(scope).into());
        install_live_cross_origin_top_level_window_surface(
            scope,
            surface,
            window_proxy,
            opener,
            endpoint,
        );
        self.top_level_cross_origin_window_access_surface = Some(v8::Global::new(scope, surface));
    }

    /// Materializes this script agent's restricted projection for a live
    /// related top-level endpoint. The logical browsing context remains owned
    /// by the group registry; this facade contains no target LocalWindow or
    /// target `JsContextHost` pointer.
    pub(crate) fn remote_top_level_window_proxy_for_endpoint<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        endpoint: TopLevelWindowProxyEndpointId,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let environment = self.page_script_environment.clone()?;
        self.remote_top_level_window_proxy_for_endpoint_with_environment(
            scope,
            &environment,
            endpoint,
        )
    }

    fn remote_top_level_window_proxy_for_endpoint_with_environment<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        environment: &crate::script_vm::RendererPageScriptEnvironment,
        endpoint: TopLevelWindowProxyEndpointId,
    ) -> Option<v8::Local<'s, v8::Object>> {
        if let Some(proxy) = environment.related_page_projected_window_proxy(scope, endpoint) {
            return Some(proxy);
        }
        environment.remote_top_level_target_snapshot(endpoint)?;

        let (proxy, facade_context) = self.bridge.bindings.instantiate_window_proxy_shell(scope);
        let facade_context_local = v8::Local::new(scope, &facade_context);
        crate::util::install_context_host_pointer_slot(
            facade_context_local,
            self as *mut Self,
            self.context_host_lifecycle_handle(),
        );
        let proxy_global;
        {
            let facade_scope = &mut v8::ContextScope::new(scope, facade_context_local);
            let facade_proxy = facade_context_local.global(facade_scope);
            debug_assert!(facade_proxy.strict_equals(proxy.into()));
            set_null_prototype(facade_scope, facade_proxy);
            let surface = new_null_prototype_object(facade_scope);
            let null_opener = v8::null(facade_scope).into();
            install_live_cross_origin_top_level_window_surface(
                facade_scope,
                surface,
                facade_proxy,
                null_opener,
                Some(endpoint),
            );
            set_private_value(
                facade_scope,
                facade_proxy,
                CLOSED_TOP_LEVEL_WINDOW_ACCESS_SURFACE_SLOT,
                surface.into(),
            );
            proxy_global = v8::Global::new(facade_scope, facade_proxy);
        }
        if let Err(error) = environment.install_remote_top_level_window_proxy_projection(
            endpoint,
            proxy_global,
            facade_context,
        ) {
            tracing::warn!(%error, "failed to install remote top-level WindowProxy projection");
            return None;
        }
        environment.related_page_projected_window_proxy(scope, endpoint)
    }

    /// Materializes a stable restricted WindowProxy for a nested context in a
    /// related Page hosted by another script agent. The facade stores only an
    /// observer-local projection id; every callback resolves that id back to a
    /// group/root-Document/frame token and consults the replicated tree.
    pub(crate) fn remote_frame_window_proxy_for_token<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        token: crate::script_vm::RendererRemoteFrameToken,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let environment = self.page_script_environment.clone()?;
        if let Some(proxy) = environment.projected_remote_frame_window_proxy(scope, token) {
            return Some(proxy);
        }
        let snapshot = environment.remote_frame_snapshot(token)?;
        let top = self.remote_top_level_window_proxy_for_endpoint(scope, token.endpoint)?;
        let parent = if let Some(parent_browsing_context_id) = snapshot.parent_browsing_context_id {
            self.remote_frame_window_proxy_for_token(
                scope,
                crate::script_vm::RendererRemoteFrameToken {
                    browsing_context_id: parent_browsing_context_id,
                    ..token
                },
            )?
        } else {
            top
        };

        let projection_id = environment.allocate_remote_frame_projection_id();
        let (proxy, facade_context) = self.bridge.bindings.instantiate_window_proxy_shell(scope);
        let facade_context_local = v8::Local::new(scope, &facade_context);
        crate::util::install_context_host_pointer_slot(
            facade_context_local,
            self as *mut Self,
            self.context_host_lifecycle_handle(),
        );
        let proxy_global;
        {
            let facade_scope = &mut v8::ContextScope::new(scope, facade_context_local);
            let facade_proxy = facade_context_local.global(facade_scope);
            debug_assert!(facade_proxy.strict_equals(proxy.into()));
            set_null_prototype(facade_scope, facade_proxy);
            let surface = new_null_prototype_object(facade_scope);
            install_live_cross_origin_remote_frame_window_surface(
                facade_scope,
                surface,
                facade_proxy,
                projection_id,
                parent,
                top,
            );
            set_private_value(
                facade_scope,
                facade_proxy,
                CLOSED_TOP_LEVEL_WINDOW_ACCESS_SURFACE_SLOT,
                surface.into(),
            );
            install_remote_frame_projection_marker(facade_scope, facade_proxy, projection_id);
            proxy_global = v8::Global::new(facade_scope, facade_proxy);
        }
        if let Err(error) = environment.install_remote_frame_window_proxy_projection(
            projection_id,
            token,
            proxy_global,
            facade_context,
        ) {
            tracing::warn!(%error, ?token, "failed to install remote-frame WindowProxy projection");
            return None;
        }
        environment.projected_remote_frame_window_proxy(scope, token)
    }

    pub(crate) fn restore_current_top_level_opener_projection<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        environment: &crate::script_vm::RendererPageScriptEnvironment,
    ) -> anyhow::Result<v8::Local<'s, v8::Value>> {
        let opener_endpoint = environment.top_level_opener_endpoint();
        if let Some(opener) = environment.top_level_opener_value(scope) {
            anyhow::ensure!(
                opener_endpoint.is_none() || !opener.is_null(),
                "committed logical opener endpoint {opener_endpoint:?} has a null agent-local projection"
            );
            return Ok(opener);
        }
        let Some(endpoint) = opener_endpoint else {
            return Ok(v8::null(scope).into());
        };
        let opener = self
            .remote_top_level_window_proxy_for_endpoint_with_environment(
                scope,
                environment,
                endpoint,
            )
            .map(v8::Local::<v8::Value>::from)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "failed to materialize the committed top-level opener endpoint {endpoint:?}"
                )
            })?;
        environment.set_top_level_opener_edge(scope, opener);
        Ok(opener)
    }

    pub(crate) fn related_top_level_opener_projection<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        target_endpoint: TopLevelWindowProxyEndpointId,
    ) -> Option<v8::Local<'s, v8::Value>> {
        let environment = self.page_script_environment.clone()?;
        let target = environment.remote_top_level_target_snapshot(target_endpoint)?;
        let Some(opener_endpoint) = target.opener_endpoint else {
            return Some(v8::null(scope).into());
        };
        Some(
            self.remote_top_level_window_proxy_for_endpoint(scope, opener_endpoint)
                .map(v8::Local::<v8::Value>::from)
                .unwrap_or_else(|| v8::null(scope).into()),
        )
    }

    /// Reattaches the stable top-level WindowProxy to a host-free facade after
    /// final Page discard.
    ///
    /// The facade deliberately retains only the cross-origin Window whitelist.
    /// Its access surface is stored on the V8 proxy itself so related Pages can
    /// keep observing the identity after this `JsContextHost` is destroyed.
    pub(crate) fn park_closed_top_level_window_proxy<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_, ()>,
        window_proxy: v8::Local<'s, v8::Object>,
        opener: v8::Local<'s, v8::Value>,
    ) -> bool {
        // Closing does not sever the auxiliary relationship. Chromium keeps
        // `closedWindow.opener` pointing at the original opener until an
        // explicit opener/group-severing policy says otherwise.
        let endpoint = self
            .top_level_window_proxy_endpoint_id()
            .expect("a parked top-level WindowProxy must retain its group endpoint");
        let Some(context) = self
            .bridge
            .bindings
            .attach_window_proxy_shell_to_facade(scope, window_proxy)
        else {
            return false;
        };
        let facade_scope = &mut v8::ContextScope::new(scope, context);
        let window_proxy = context.global(facade_scope);
        let surface = new_null_prototype_object(facade_scope);
        install_live_cross_origin_top_level_window_surface(
            facade_scope,
            surface,
            window_proxy,
            opener,
            Some(endpoint),
        );
        let closed: v8::Local<'s, v8::Value> = v8::Boolean::new(facade_scope, true).into();
        set_private_value(
            facade_scope,
            surface,
            CLOSED_TOP_LEVEL_WINDOW_MARKER_SLOT,
            closed,
        );
        let closed: v8::Local<'s, v8::Value> = v8::Boolean::new(facade_scope, true).into();
        set_private_value(
            facade_scope,
            window_proxy,
            CLOSED_TOP_LEVEL_WINDOW_MARKER_SLOT,
            closed,
        );
        set_private_value(
            facade_scope,
            window_proxy,
            CLOSED_TOP_LEVEL_WINDOW_ACCESS_SURFACE_SLOT,
            surface.into(),
        );
        crate::util::disconnect_page_context(facade_scope);
        true
    }

    /// Parks the old agent's stable outer proxy as a live remote facade while
    /// the same logical browsing context commits a LocalWindow in another
    /// script agent. Unlike final close/COOP sever this keeps the endpoint,
    /// opener edge, and `closed === false` surface live.
    pub(crate) fn park_remote_top_level_window_proxy<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_, ()>,
        window_proxy: v8::Local<'s, v8::Object>,
        opener: v8::Local<'s, v8::Value>,
    ) -> bool {
        let Some(environment) = self.page_script_environment.clone() else {
            return false;
        };
        let endpoint = environment.top_level_window_proxy_endpoint_id();
        let Some(context) = self
            .bridge
            .bindings
            .attach_window_proxy_shell_to_facade(scope, window_proxy)
        else {
            return false;
        };
        let facade_context = v8::Global::new(scope, context);
        let facade_scope = &mut v8::ContextScope::new(scope, context);
        let window_proxy = context.global(facade_scope);
        let surface = new_null_prototype_object(facade_scope);
        install_live_cross_origin_top_level_window_surface(
            facade_scope,
            surface,
            window_proxy,
            opener,
            Some(endpoint),
        );
        set_private_value(
            facade_scope,
            window_proxy,
            CLOSED_TOP_LEVEL_WINDOW_ACCESS_SURFACE_SLOT,
            surface.into(),
        );
        crate::util::disconnect_page_context(facade_scope);
        environment.retain_current_agent_top_level_facade_context(facade_context);
        true
    }

    fn top_level_cross_origin_window_handler_data<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        window_proxy: v8::Local<'s, v8::Object>,
    ) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>)> {
        let surface = self
            .top_level_cross_origin_window_access_surface
            .as_ref()
            .map(|surface| v8::Local::new(scope, surface))?;
        Some((surface, window_proxy))
    }

    pub(crate) fn child_browsing_context_window_proxy_for_top<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        if self.child_browsing_context_is_same_origin_with_top(handle) {
            return self.child_browsing_context_window_wrapper(scope, handle);
        }
        if let Some(proxy) = self.child_window_proxy_records.live_window(scope, handle) {
            return Some(proxy);
        }
        if self.child_window_proxy_records.window_proxy_exposed(handle)
            && let Some(proxy) = self.ensure_top_exposed_cross_origin_window_proxy(scope, handle)
        {
            return Some(proxy);
        }
        self.child_browsing_context_cross_origin_window_proxy(scope, handle)
    }

    pub(crate) fn mark_child_browsing_context_window_proxy_exposed(&mut self, handle: DomHandle) {
        self.child_window_proxy_records
            .mark_window_proxy_exposed(handle);
    }

    pub(crate) fn child_browsing_context_cross_origin_window_proxy<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        if let Some(work) = self.refresh_child_browsing_context(scope, handle) {
            self.push_child_document_script_ready_input(work);
        }
        let child_frame_count = self.child_browsing_context_child_frame_count(handle);
        if let Some(proxy) = self
            .child_window_proxy_records
            .cross_origin_proxy(scope, handle)
        {
            return Some(proxy);
        }
        if !self.child_browsing_contexts.contains_key(&handle) {
            return None;
        }

        let named_indices = self.child_browsing_context_child_frame_named_indices(handle);
        let (proxy, facade_context) = self.bridge.bindings.instantiate_window_proxy_shell(scope);
        let global = scope.get_current_context().global(scope);
        let top = self.child_browsing_context_root_window(scope, handle, global);
        let parent = self.child_browsing_context_parent_window(scope, handle, top);
        self.child_window_proxy_records
            .set_browsing_context_parent_top(scope, handle, parent, top);
        let facade_context_local = v8::Local::new(scope, &facade_context);
        crate::util::install_context_host_pointer_slot(
            facade_context_local,
            self as *mut Self,
            self.context_host_lifecycle_handle(),
        );
        let previous =
            facade_context_local.set_slot(Rc::new(ChildWindowProxyFacadeContextHandle(handle)));
        debug_assert!(previous.is_none());
        {
            let facade_scope = &mut v8::ContextScope::new(scope, facade_context_local);
            let facade_proxy = facade_context_local.global(facade_scope);
            debug_assert!(facade_proxy.strict_equals(proxy.into()));
            set_null_prototype(facade_scope, facade_proxy);
            install_live_cross_origin_child_window_surface(
                facade_scope,
                facade_proxy,
                handle,
                parent,
                top,
                facade_proxy,
                child_frame_count,
                &named_indices,
            );
            let access_surface = new_null_prototype_object(facade_scope);
            install_live_cross_origin_child_window_surface(
                facade_scope,
                access_surface,
                handle,
                parent,
                top,
                facade_proxy,
                child_frame_count,
                &named_indices,
            );
            self.child_window_proxy_records
                .set_cross_origin_access_surface(facade_scope, handle, access_surface);
            self.child_window_proxy_records.set_cross_origin_proxy(
                facade_scope,
                handle,
                facade_proxy,
            );
        }
        self.child_window_proxy_records
            .set_facade_context(handle, facade_context);
        self.child_window_proxy_records
            .cross_origin_proxy(scope, handle)
    }

    pub(crate) fn ensure_top_exposed_cross_origin_window_proxy<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        if let Some(proxy) = self.child_window_proxy_records.live_window(scope, handle) {
            return Some(proxy);
        }
        self.child_browsing_context_cross_origin_window_proxy(scope, handle)
    }

    pub(crate) fn child_browsing_context_window_wrapper<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let (wrapper, ready_work) =
            self.child_browsing_context_window_wrapper_with_ready_work(scope, handle);
        for work in ready_work {
            self.push_child_document_script_ready_input(work);
        }
        wrapper
    }

    pub(crate) fn child_browsing_context_window_wrapper_with_ready_work<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> (
        Option<v8::Local<'s, v8::Object>>,
        Vec<FrameDocumentClassicScriptSchedulerWork>,
    ) {
        self.child_browsing_context_window_wrapper_with_projection_authority(scope, handle, false)
    }

    fn child_browsing_context_window_wrapper_with_projection_authority<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
        may_promote_cross_origin_facade: bool,
    ) -> (
        Option<v8::Local<'s, v8::Object>>,
        Vec<FrameDocumentClassicScriptSchedulerWork>,
    ) {
        let mut ready_work = self
            .refresh_child_browsing_context(scope, handle)
            .into_iter()
            .collect::<Vec<_>>();
        if self.child_window_proxy_records.has_live_window(handle) {
            if self.child_default_execution_context_id(handle).is_none()
                && let Err(error) = self.ensure_prebootstrapped_child_default_context(scope, handle)
            {
                tracing::warn!(
                    %error,
                    child_handle = handle.index(),
                    "failed to attach the stable child WindowProxy to its current LocalWindow"
                );
                return (None, ready_work);
            }
            let (_, document_ready_work) =
                self.child_browsing_context_document_wrapper_with_ready_work(scope, handle);
            ready_work.extend(document_ready_work);
            let Some(wrapper) = self.child_window_proxy_records.live_window(scope, handle) else {
                return (None, ready_work);
            };
            self.sync_child_browsing_context_window_parent_top_slots(scope, handle, wrapper);
            bind_materialized_child_window_indexed_db_factory(scope, wrapper, handle);
            return (Some(wrapper), ready_work);
        }
        if self
            .child_window_proxy_records
            .has_cross_origin_proxy(handle)
            && !self.child_browsing_context_is_same_origin_with_top(handle)
            && !may_promote_cross_origin_facade
        {
            return (
                self.child_window_proxy_records
                    .cross_origin_proxy(scope, handle),
                ready_work,
            );
        }
        if !self.child_browsing_contexts.contains_key(&handle) {
            return (None, ready_work);
        }

        if let Err(error) = self.ensure_prebootstrapped_child_default_context(scope, handle) {
            tracing::warn!(
                %error,
                child_handle = handle.index(),
                "failed to bootstrap child LocalWindow context"
            );
            return (None, ready_work);
        }
        let (_, document_ready_work) =
            self.child_browsing_context_document_wrapper_with_ready_work(scope, handle);
        ready_work.extend(document_ready_work);
        let Some(wrapper) = self.child_window_proxy_records.live_window(scope, handle) else {
            return (None, ready_work);
        };
        self.sync_child_browsing_context_window_parent_top_slots(scope, handle, wrapper);
        bind_materialized_child_window_indexed_db_factory(scope, wrapper, handle);
        (Some(wrapper), ready_work)
    }

    fn child_browsing_context_window_wrapper_for_authorized_observer<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let (wrapper, ready_work) = self
            .child_browsing_context_window_wrapper_with_projection_authority(scope, handle, true);
        for work in ready_work {
            self.push_child_document_script_ready_input(work);
        }
        wrapper
    }

    pub(crate) fn child_browsing_context_window_for_navigation_observer<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
        observer_can_access: bool,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let window = if observer_can_access {
            self.child_browsing_context_window_wrapper_for_authorized_observer(scope, handle)
        } else {
            self.child_browsing_context_window_proxy_for_top(scope, handle)
        };
        if window.is_some() {
            self.mark_child_browsing_context_window_proxy_exposed(handle);
        }
        window
    }

    pub(crate) fn existing_child_browsing_context_window_wrapper<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.child_window_proxy_records.live_window(scope, handle)
    }

    pub(crate) fn child_browsing_context_top_window_for_current_realm<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.child_window_proxy_records.realm_top(scope, handle)
    }

    pub(in crate::native_bridge::context_host) fn child_browsing_context_parent_window<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
        top: v8::Local<'s, v8::Object>,
    ) -> v8::Local<'s, v8::Object> {
        self.child_browsing_context_parent_handle(handle)
            .and_then(|parent| self.existing_child_browsing_context_window_wrapper(scope, parent))
            .unwrap_or(top)
    }

    pub(in crate::native_bridge::context_host) fn sync_child_browsing_context_window_parent_top_slots<
        's,
    >(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
        window: v8::Local<'s, v8::Object>,
    ) {
        let global = scope.get_current_context().global(scope);
        let same_origin_with_top = self.child_browsing_context_is_same_origin_with_top(handle);
        let existing_parent =
            Self::child_window_non_cross_origin_object_slot(scope, window, "__moliWindowParent");
        let existing_top =
            Self::child_window_non_cross_origin_object_slot(scope, window, "__moliWindowTop");
        let stable_parent = self
            .child_window_proxy_records
            .browsing_context_parent(scope, handle);
        let stable_top = self
            .child_window_proxy_records
            .browsing_context_top(scope, handle);
        let root = stable_top
            .or(existing_top)
            .filter(|top| !is_cross_origin_window_proxy(scope, *top))
            .unwrap_or_else(|| self.child_browsing_context_root_window(scope, handle, global));
        let top = if same_origin_with_top {
            root
        } else {
            self.cross_origin_window_endpoint_projection_for_child(
                scope,
                handle,
                PendingWindowMessageEndpoint::TopWindow,
            )
            .unwrap_or(window)
        };
        let parent = if same_origin_with_top {
            stable_parent
                .or(existing_parent)
                .unwrap_or_else(|| self.child_browsing_context_parent_window(scope, handle, top))
        } else {
            self.child_browsing_context_parent_window(scope, handle, top)
        };
        set_object_slot(scope, window, "__moliWindowParent", parent.into());
        set_object_slot(scope, window, "__moliWindowTop", top.into());
        set_private_value(scope, window, "__moliWindowParent", parent.into());
        set_private_value(scope, window, "__moliWindowTop", top.into());
    }

    pub(in crate::native_bridge::context_host) fn child_window_object_slot<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        window: v8::Local<'s, v8::Object>,
        name: &str,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let key = v8_string(scope, name)?;
        window
            .get(scope, key.into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    }

    pub(in crate::native_bridge::context_host) fn child_window_non_cross_origin_object_slot<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        window: v8::Local<'s, v8::Object>,
        name: &str,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let object = Self::child_window_object_slot(scope, window, name)?;
        (!is_cross_origin_window_proxy(scope, object)).then_some(object)
    }

    pub(in crate::native_bridge::context_host) fn child_browsing_context_parent_top_for_realm_global<
        's,
    >(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
        global: v8::Local<'s, v8::Object>,
    ) -> (v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>) {
        let Some(window) = self.existing_child_browsing_context_window_wrapper(scope, handle)
        else {
            let top = self
                .child_window_proxy_records
                .browsing_context_top(scope, handle)
                .unwrap_or_else(|| self.child_browsing_context_root_window(scope, handle, global));
            let parent = self
                .child_window_proxy_records
                .browsing_context_parent(scope, handle)
                .unwrap_or_else(|| self.child_browsing_context_parent_window(scope, handle, top));
            return (parent, top);
        };

        if self.child_browsing_context_is_same_origin_with_top(handle) {
            let top = self
                .child_window_proxy_records
                .browsing_context_top(scope, handle)
                .or_else(|| {
                    Self::child_window_non_cross_origin_object_slot(
                        scope,
                        window,
                        "__moliWindowTop",
                    )
                })
                .unwrap_or_else(|| self.child_browsing_context_root_window(scope, handle, global));
            let parent = self
                .child_window_proxy_records
                .browsing_context_parent(scope, handle)
                .or_else(|| {
                    Self::child_window_non_cross_origin_object_slot(
                        scope,
                        window,
                        "__moliWindowParent",
                    )
                })
                .unwrap_or_else(|| self.child_browsing_context_parent_window(scope, handle, top));
            return (parent, top);
        }

        let existing_top = Self::child_window_object_slot(scope, window, "__moliWindowTop");
        let top = existing_top
            .filter(|top| is_cross_origin_window_proxy(scope, *top))
            .or_else(|| {
                self.cross_origin_window_endpoint_projection_for_child(
                    scope,
                    handle,
                    PendingWindowMessageEndpoint::TopWindow,
                )
            })
            .unwrap_or(global);
        let parent = self.child_browsing_context_parent_window(scope, handle, top);
        (parent, top)
    }

    pub(in crate::native_bridge::context_host) fn child_browsing_context_root_window<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
        fallback: v8::Local<'s, v8::Object>,
    ) -> v8::Local<'s, v8::Object> {
        if let Some(parent_handle) = self.child_browsing_context_parent_handle(handle)
            && let Some(parent_top) = self
                .child_window_proxy_records
                .browsing_context_top(scope, parent_handle)
        {
            return parent_top;
        }
        let top_scope = super::super::OwnerDispatchScope::Top;
        if let Some(top_owner) = self.current_window_execution_context_owner(top_scope)
            && let Some((_, top_context)) =
                self.window_execution_context(scope, top_owner, top_scope)
        {
            return top_context.global(scope);
        }
        fallback
    }

    fn cross_origin_window_endpoint_projection_for_child<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        accessing_handle: DomHandle,
        endpoint: PendingWindowMessageEndpoint,
    ) -> Option<v8::Local<'s, v8::Object>> {
        match endpoint {
            PendingWindowMessageEndpoint::TopWindow => {
                let top_scope = super::super::OwnerDispatchScope::Top;
                if let Some(top_owner) = self.current_window_execution_context_owner(top_scope)
                    && let Some((_, top_context)) =
                        self.window_execution_context(scope, top_owner, top_scope)
                {
                    return Some(top_context.global(scope));
                }
                if let Some(projection) = self
                    .child_window_proxy_records
                    .cross_origin_endpoint_projection(scope, accessing_handle, endpoint)
                {
                    return Some(projection);
                }
                let projection = build_cross_origin_top_window_proxy(scope);
                self.child_window_proxy_records
                    .set_cross_origin_endpoint_projection(
                        scope,
                        accessing_handle,
                        endpoint,
                        projection,
                    );
                let storage = cross_origin_proxy_storage_object(scope, projection);
                set_cross_origin_object_slot(scope, storage, "opener", v8::null(scope).into());
                Some(projection)
            }
            PendingWindowMessageEndpoint::ChildWindow(handle) => {
                self.child_browsing_context_cross_origin_window_proxy(scope, handle)
            }
        }
    }
}

impl JsContextHost {
    pub(crate) fn child_performance_navigation_type(&self, handle: DomHandle) -> String {
        self.child_browsing_contexts
            .get(&handle)
            .map(|entry| entry.performance_navigation_type())
            .unwrap_or("navigate")
            .to_owned()
    }

    pub(crate) fn child_performance_time_origin(&self, handle: DomHandle) -> f64 {
        self.child_browsing_contexts
            .get(&handle)
            .map(|entry| entry.performance_time_origin_millis())
            .unwrap_or_else(moli_time::unix_epoch_millis)
    }
}

fn install_cross_origin_window_index_slots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    count: usize,
    parent: v8::Local<'s, v8::Object>,
    top: v8::Local<'s, v8::Object>,
) {
    for index in 0..count.min(u32::MAX as usize) {
        let index_name = index.to_string();
        let Some(key) = v8_string(scope, &index_name) else {
            continue;
        };
        if window.has_own_property(scope, key.into()).unwrap_or(false) {
            continue;
        }
        let child = build_detached_cross_origin_window_index_proxy(scope, parent, top);
        let _ = window.define_own_property(
            scope,
            key.into(),
            child.into(),
            cross_origin_index_property_attributes(),
        );
    }
}

fn install_cross_origin_window_named_slots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    named_indices: &[(usize, String)],
    parent: v8::Local<'s, v8::Object>,
    top: v8::Local<'s, v8::Object>,
) {
    let current_names = named_indices
        .iter()
        .filter_map(|(_, name)| {
            is_cross_origin_named_child_slot_name(name).then_some(name.as_str())
        })
        .collect::<std::collections::HashSet<_>>();
    remove_stale_cross_origin_window_named_slots(scope, window, &current_names);

    for (index, name) in named_indices {
        if !is_cross_origin_named_child_slot_name(name) {
            continue;
        }
        let Some(key) = v8_string(scope, name) else {
            continue;
        };
        if name != "then" && window.has_own_property(scope, key.into()).unwrap_or(false) {
            continue;
        }
        let value = window.get_index(scope, *index as u32).unwrap_or_else(|| {
            build_detached_cross_origin_window_index_proxy(scope, parent, top).into()
        });
        let _ = window.define_own_property(
            scope,
            key.into(),
            value,
            cross_origin_named_property_attributes(),
        );
    }
    set_cross_origin_window_named_slot_registry(scope, window, &current_names);
}

fn is_cross_origin_named_child_slot_name(name: &str) -> bool {
    cross_origin_named_child_can_shadow(name)
}

fn remove_stale_cross_origin_window_named_slots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    current_names: &std::collections::HashSet<&str>,
) {
    let Some(previous_names) = cross_origin_window_named_slot_registry(scope, window) else {
        return;
    };
    for name in previous_names {
        if current_names.contains(name.as_str()) {
            continue;
        }
        let Some(key) = v8_string(scope, &name) else {
            continue;
        };
        if name == "then" {
            set_cross_origin_object_slot(scope, window, "then", v8::undefined(scope).into());
        } else {
            let _ = window.delete(scope, key.into());
        }
    }
}

fn cross_origin_window_named_slot_registry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<Vec<String>> {
    let value = get_private_value(scope, window, CROSS_ORIGIN_WINDOW_NAMED_CHILD_SLOTS)?;
    let array = v8::Local::<v8::Array>::try_from(value).ok()?;
    let mut names = Vec::new();
    for index in 0..array.length() {
        if let Some(name) = array
            .get_index(scope, index)
            .and_then(|value| value.to_string(scope))
        {
            names.push(name.to_rust_string_lossy(scope));
        }
    }
    Some(names)
}

fn set_cross_origin_window_named_slot_registry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    names: &std::collections::HashSet<&str>,
) {
    let names = names.iter().copied().collect::<Vec<_>>();
    let array =
        serialize_v8_array(scope, names.as_slice()).unwrap_or_else(|| v8::Array::new(scope, 0));
    set_private_value(
        scope,
        window,
        CROSS_ORIGIN_WINDOW_NAMED_CHILD_SLOTS,
        array.into(),
    );
}

fn install_live_cross_origin_child_window_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    handle: DomHandle,
    parent: v8::Local<'s, v8::Object>,
    top: v8::Local<'s, v8::Object>,
    indexed_parent: v8::Local<'s, v8::Object>,
    child_frame_count: usize,
    named_indices: &[(usize, String)],
) {
    install_cross_origin_window_identity_slots(scope, window, handle, parent, top);
    install_cross_origin_window_index_slots(scope, window, child_frame_count, indexed_parent, top);
    install_cross_origin_symbol_slots(scope, window, "Window");
    let location = build_cross_origin_location_proxy(scope, handle);
    set_private_value(
        scope,
        window,
        CROSS_ORIGIN_WINDOW_LOCATION_SLOT,
        location.into(),
    );
    CrossOriginWindowLiveAccessorsDeclaration::default()
        .initialize(scope, window)
        .expect("cross-origin Window accessors declaration should initialize");
    set_cross_origin_object_slot(scope, window, "opener", v8::null(scope).into());
    set_cross_origin_object_slot(scope, window, "then", v8::undefined(scope).into());
    install_cross_origin_window_methods(scope, window);
    install_cross_origin_window_named_slots(scope, window, named_indices, indexed_parent, top);
}

fn install_cross_origin_related_top_window_endpoint<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    endpoint: TopLevelWindowProxyEndpointId,
) {
    set_private_value(
        scope,
        object,
        CROSS_ORIGIN_RELATED_TOP_WINDOW_GROUP_SLOT,
        v8::BigInt::new_from_u64(scope, endpoint.browsing_context_group_id().value()).into(),
    );
    set_private_value(
        scope,
        object,
        CROSS_ORIGIN_RELATED_TOP_WINDOW_GENERATION_SLOT,
        v8::BigInt::new_from_u64(scope, endpoint.generation()).into(),
    );
}

fn install_remote_frame_projection_marker<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    projection_id: u64,
) {
    set_private_value(
        scope,
        object,
        CROSS_ORIGIN_REMOTE_FRAME_PROJECTION_SLOT,
        v8::BigInt::new_from_u64(scope, projection_id).into(),
    );
}

fn install_live_cross_origin_remote_frame_window_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    surface: v8::Local<'s, v8::Object>,
    window_proxy: v8::Local<'s, v8::Object>,
    projection_id: u64,
    parent: v8::Local<'s, v8::Object>,
    top: v8::Local<'s, v8::Object>,
) {
    install_remote_frame_projection_marker(scope, surface, projection_id);
    set_cross_origin_object_slot(scope, surface, "self", window_proxy.into());
    set_cross_origin_object_slot(scope, surface, "window", window_proxy.into());
    set_cross_origin_object_slot(scope, surface, "frames", window_proxy.into());
    set_cross_origin_object_slot(scope, surface, "parent", parent.into());
    set_cross_origin_object_slot(scope, surface, "top", top.into());
    set_cross_origin_object_slot(scope, surface, "opener", v8::null(scope).into());
    set_cross_origin_object_slot(scope, surface, "then", v8::undefined(scope).into());
    install_cross_origin_symbol_slots(scope, surface, "Window");
    let location = build_detached_cross_origin_location_proxy(scope);
    let location_storage = cross_origin_proxy_storage_object(scope, location);
    install_remote_frame_projection_marker(scope, location_storage, projection_id);
    set_private_value(
        scope,
        surface,
        CROSS_ORIGIN_WINDOW_LOCATION_SLOT,
        location.into(),
    );
    set_private_value(
        scope,
        window_proxy,
        CROSS_ORIGIN_WINDOW_LOCATION_SLOT,
        location.into(),
    );
    CrossOriginWindowLiveAccessorsDeclaration::default()
        .initialize(scope, surface)
        .expect("cross-origin remote-frame Window accessors should initialize");
    install_cross_origin_window_methods(scope, surface);
}

fn install_cross_origin_local_top_window_marker<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        object,
        CROSS_ORIGIN_LOCAL_TOP_WINDOW_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
}

fn install_live_cross_origin_top_level_window_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    surface: v8::Local<'s, v8::Object>,
    window_proxy: v8::Local<'s, v8::Object>,
    opener: v8::Local<'s, v8::Value>,
    endpoint: Option<TopLevelWindowProxyEndpointId>,
) {
    if let Some(endpoint) = endpoint {
        install_cross_origin_related_top_window_endpoint(scope, surface, endpoint);
        install_cross_origin_related_top_window_endpoint(scope, window_proxy, endpoint);
    } else {
        // Standalone ScriptVm owners intentionally have no Page-group
        // identity. They still need the local cross-origin Window surface,
        // but must never fabricate a group-qualified remote endpoint.
        install_cross_origin_local_top_window_marker(scope, surface);
        install_cross_origin_local_top_window_marker(scope, window_proxy);
    }
    set_cross_origin_object_slot(scope, surface, "self", window_proxy.into());
    set_cross_origin_object_slot(scope, surface, "window", window_proxy.into());
    set_cross_origin_object_slot(scope, surface, "parent", window_proxy.into());
    set_cross_origin_object_slot(scope, surface, "top", window_proxy.into());
    set_cross_origin_object_slot(scope, surface, "frames", window_proxy.into());
    // Live top-level indexed and named access is projected directly from the
    // target Page's child registry by the WindowProxy handlers below. Keeping
    // snapshots on this surface would leave stale indices after removal and
    // incorrectly expose named children through [[OwnPropertyKeys]].
    install_cross_origin_symbol_slots(scope, surface, "Window");
    let location = build_detached_cross_origin_location_proxy(scope);
    let location_storage = cross_origin_proxy_storage_object(scope, location);
    if let Some(endpoint) = endpoint {
        install_cross_origin_related_top_window_endpoint(scope, location_storage, endpoint);
    } else {
        install_cross_origin_local_top_window_marker(scope, location_storage);
    }
    set_private_value(
        scope,
        surface,
        CROSS_ORIGIN_WINDOW_LOCATION_SLOT,
        location.into(),
    );
    set_private_value(
        scope,
        window_proxy,
        CROSS_ORIGIN_WINDOW_LOCATION_SLOT,
        location.into(),
    );
    CrossOriginWindowLiveAccessorsDeclaration::default()
        .initialize(scope, surface)
        .expect("cross-origin top-level Window accessors declaration should initialize");
    set_private_value(scope, surface, WINDOW_OPENER_SLOT, opener);
    CrossOriginTopLevelWindowOpenerAccessorDeclaration::default()
        .initialize(scope, surface)
        .expect("cross-origin top-level Window opener accessor should initialize");
    set_cross_origin_object_slot(scope, surface, "then", v8::undefined(scope).into());
    install_cross_origin_window_methods(scope, surface);
}

fn build_detached_cross_origin_window_index_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    top: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let window = new_null_prototype_object(scope);
    set_private_value(
        scope,
        window,
        DETACHED_CROSS_ORIGIN_WINDOW_PROXY_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    set_cross_origin_object_slot(scope, window, "parent", parent.into());
    set_cross_origin_object_slot(scope, window, "top", top.into());
    set_cross_origin_object_slot(
        scope,
        window,
        "closed",
        v8::Boolean::new(scope, false).into(),
    );
    set_cross_origin_object_slot(scope, window, "opener", v8::null(scope).into());
    set_cross_origin_object_slot(scope, window, "then", v8::undefined(scope).into());
    set_cross_origin_object_slot(scope, window, "length", v8::Number::new(scope, 0.0).into());
    let location = build_detached_cross_origin_location_proxy(scope);
    set_private_value(
        scope,
        window,
        CROSS_ORIGIN_WINDOW_LOCATION_SLOT,
        location.into(),
    );
    CrossOriginWindowLocationAccessorDeclaration::default()
        .initialize(scope, window)
        .expect("cross-origin Window location accessor declaration should initialize");
    install_cross_origin_window_methods(scope, window);
    install_cross_origin_symbol_slots(scope, window, "Window");
    let Some(proxy) = wrap_detached_cross_origin_window(scope, window) else {
        return window;
    };
    set_cross_origin_object_slot(scope, window, "self", proxy.into());
    set_cross_origin_object_slot(scope, window, "window", proxy.into());
    set_cross_origin_object_slot(scope, window, "frames", proxy.into());
    proxy
}

fn build_cross_origin_top_window_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    build_cross_origin_top_level_window_proxy(scope)
}

fn build_cross_origin_top_level_window_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    let window = new_null_prototype_object(scope);
    set_private_value(
        scope,
        window,
        window_host::TOP_WINDOW_MESSAGE_ENDPOINT_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    set_private_value(
        scope,
        window,
        DETACHED_CROSS_ORIGIN_WINDOW_PROXY_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    set_private_value(
        scope,
        window,
        CROSS_ORIGIN_TOP_WINDOW_PROXY_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    set_cross_origin_object_slot(scope, window, "then", v8::undefined(scope).into());
    let location = build_detached_cross_origin_location_proxy(scope);
    set_private_value(
        scope,
        window,
        CROSS_ORIGIN_WINDOW_LOCATION_SLOT,
        location.into(),
    );
    CrossOriginWindowLiveAccessorsDeclaration::default()
        .initialize(scope, window)
        .expect("cross-origin top Window accessors declaration should initialize");
    install_cross_origin_window_methods(scope, window);
    install_cross_origin_symbol_slots(scope, window, "Window");
    let Some(proxy) = wrap_detached_cross_origin_window(scope, window) else {
        return window;
    };
    set_cross_origin_window_self_identity_slots(scope, window, proxy);
    proxy
}

fn is_cross_origin_window_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    get_cross_origin_proxy_private_value(scope, object, CROSS_ORIGIN_WINDOW_LOCATION_SLOT).is_some()
        || get_cross_origin_proxy_private_value(
            scope,
            object,
            DETACHED_CROSS_ORIGIN_WINDOW_PROXY_SLOT,
        )
        .is_some()
        || get_cross_origin_proxy_private_value(scope, object, CROSS_ORIGIN_TOP_WINDOW_PROXY_SLOT)
            .is_some()
}

fn cross_origin_proxy_storage_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let value: v8::Local<'s, v8::Value> = object.into();
    let Ok(proxy) = v8::Local::<v8::Proxy>::try_from(value) else {
        return object;
    };
    let Ok(target) = v8::Local::<v8::Object>::try_from(proxy.get_target(scope)) else {
        return object;
    };
    let is_moli_cross_origin_proxy =
        get_private_value(scope, target, CROSS_ORIGIN_LOCATION_PROXY_SLOT).is_some()
            || get_private_value(scope, target, DETACHED_CROSS_ORIGIN_WINDOW_PROXY_SLOT).is_some()
            || get_private_value(scope, target, CROSS_ORIGIN_TOP_WINDOW_PROXY_SLOT).is_some();
    if is_moli_cross_origin_proxy {
        target
    } else {
        object
    }
}

fn get_cross_origin_proxy_private_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let storage = cross_origin_proxy_storage_object(scope, object);
    get_private_value(scope, storage, slot)
}

fn set_cross_origin_window_self_identity_slots(
    scope: &mut v8::PinScope<'_, '_>,
    window: v8::Local<'_, v8::Object>,
    identity: v8::Local<'_, v8::Object>,
) {
    set_cross_origin_object_slot(scope, window, "self", identity.into());
    set_cross_origin_object_slot(scope, window, "window", identity.into());
    set_cross_origin_object_slot(scope, window, "parent", identity.into());
    set_cross_origin_object_slot(scope, window, "top", identity.into());
    set_cross_origin_object_slot(scope, window, "frames", identity.into());
}

fn wrap_detached_cross_origin_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let handler = CrossOriginWindowProxyHandlerDeclaration {
        get: (),
        has: (),
        set: (),
        get_own_property_descriptor: (),
        own_keys: (),
        delete_property: (),
        define_property: (),
        set_prototype_of: (),
        prevent_extensions: (),
    }
    .bind(scope)
    .ok()?;
    let proxy = v8::Proxy::new(scope, target, handler)?;
    let proxy: v8::Local<'s, v8::Value> = proxy.into();
    v8::Local::<v8::Object>::try_from(proxy).ok()
}

fn install_cross_origin_window_methods<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    CrossOriginWindowMethodsDeclaration::default()
        .initialize(scope, object)
        .expect("cross-origin Window methods declaration should initialize");
}

fn install_cross_origin_window_identity_slots(
    scope: &mut v8::PinScope<'_, '_>,
    window: v8::Local<'_, v8::Object>,
    handle: DomHandle,
    parent: v8::Local<'_, v8::Object>,
    top: v8::Local<'_, v8::Object>,
) {
    let handle_value = v8::Number::new(scope, handle.index() as f64);
    set_private_value(
        scope,
        window,
        CHILD_BROWSING_CONTEXT_HANDLE_SLOT,
        handle_value.into(),
    );
    set_cross_origin_object_slot(scope, window, "self", window.into());
    set_cross_origin_object_slot(scope, window, "window", window.into());
    set_cross_origin_object_slot(scope, window, "parent", parent.into());
    set_cross_origin_object_slot(scope, window, "top", top.into());
    set_cross_origin_object_slot(scope, window, "frames", window.into());
}

pub(in crate::native_bridge::context_host::child_frame_runtime) fn install_child_window_identity_slots<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    handle: DomHandle,
    parent: v8::Local<'s, v8::Object>,
    top: v8::Local<'s, v8::Object>,
) {
    let handle_value = v8::Number::new(scope, handle.index() as f64);
    set_private_value(
        scope,
        window,
        CHILD_BROWSING_CONTEXT_HANDLE_SLOT,
        handle_value.into(),
    );
    set_object_slot(scope, window, "__moliWindowSelf", window.into());
    set_object_slot(scope, window, "__moliWindowParent", parent.into());
    set_object_slot(scope, window, "__moliWindowTop", top.into());
    set_object_slot(scope, window, "__moliWindowFrames", window.into());
    set_private_value(scope, window, "__moliWindowSelf", window.into());
    set_private_value(scope, window, "__moliWindowParent", parent.into());
    set_private_value(scope, window, "__moliWindowTop", top.into());
    set_private_value(scope, window, "__moliWindowFrames", window.into());
}

fn set_cross_origin_object_slot(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    let _ = object.define_own_property(
        scope,
        v8str(scope, name).into(),
        value,
        cross_origin_property_attributes(),
    );
}

pub(super) fn build_cross_origin_location_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: DomHandle,
) -> v8::Local<'s, v8::Object> {
    let target = new_cross_origin_location_proxy_target(scope);
    set_private_value(
        scope,
        target,
        CROSS_ORIGIN_LOCATION_PROXY_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    set_private_value(
        scope,
        target,
        CHILD_BROWSING_CONTEXT_HANDLE_SLOT,
        v8::Number::new(scope, handle.index() as f64).into(),
    );
    install_cross_origin_symbol_slots(scope, target, "Location");
    install_cross_origin_location_href(scope, target);
    install_cross_origin_location_methods(scope, target);
    set_cross_origin_object_slot(scope, target, "then", v8::undefined(scope).into());
    let location = wrap_cross_origin_location_proxy(scope, target).unwrap_or(target);
    set_private_value(
        scope,
        target,
        CROSS_ORIGIN_LOCATION_PROXY_SELF_SLOT,
        location.into(),
    );
    location
}

fn build_detached_cross_origin_location_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    let target = new_cross_origin_location_proxy_target(scope);
    set_private_value(
        scope,
        target,
        CROSS_ORIGIN_LOCATION_PROXY_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    install_cross_origin_symbol_slots(scope, target, "Location");
    install_cross_origin_location_href(scope, target);
    install_cross_origin_location_methods(scope, target);
    set_cross_origin_object_slot(scope, target, "then", v8::undefined(scope).into());
    let location = wrap_cross_origin_location_proxy(scope, target).unwrap_or(target);
    set_private_value(
        scope,
        target,
        CROSS_ORIGIN_LOCATION_PROXY_SELF_SLOT,
        location.into(),
    );
    location
}

fn new_cross_origin_location_proxy_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    new_null_prototype_object(scope)
}

fn wrap_cross_origin_location_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let handler = CrossOriginLocationProxyHandlerDeclaration {
        get: (),
        has: (),
        set: (),
        get_own_property_descriptor: (),
        delete_property: (),
        define_property: (),
        set_prototype_of: (),
        prevent_extensions: (),
    }
    .bind(scope)
    .ok()?;
    let proxy = v8::Proxy::new(scope, target, handler)?;
    let proxy: v8::Local<'s, v8::Value> = proxy.into();
    v8::Local::<v8::Object>::try_from(proxy).ok()
}

fn install_cross_origin_location_href<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
) {
    let Some(setter) = v8::Function::builder(cross_origin_location_navigate_setter_callback)
        .length(1)
        .data(location.into())
        .build(scope)
    else {
        return;
    };
    if let Some(setter_name) = v8_string(scope, "set href") {
        setter.set_name(setter_name);
    }
    let _ = define_get_set_property(
        scope,
        location,
        v8str(scope, "href").into(),
        v8::undefined(scope).into(),
        setter.into(),
        cross_origin_property_attributes(),
        "href",
    );
}

fn install_cross_origin_location_methods<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
) {
    CrossOriginLocationMethodsDeclaration::default()
        .initialize(scope, location)
        .expect("cross-origin Location methods declaration should initialize");
}

fn install_cross_origin_symbol_slots(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    _to_string_tag: &'static str,
) {
    let undefined = v8::undefined(scope).into();
    let _ = object.define_own_property(
        scope,
        v8::Symbol::get_to_string_tag(scope).into(),
        undefined,
        cross_origin_property_attributes(),
    );
    for symbol in [
        v8::Symbol::get_has_instance(scope),
        v8::Symbol::get_is_concat_spreadable(scope),
    ] {
        let _ = object.define_own_property(
            scope,
            symbol.into(),
            undefined,
            cross_origin_property_attributes(),
        );
    }
}

fn child_window_cross_origin_access_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    child_window_cross_origin_handler_data(scope, holder).map(|(surface, _)| surface)
}

fn child_window_cross_origin_proxy_self<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    child_window_cross_origin_handler_data(scope, holder)
        .map(|(_, window_proxy)| window_proxy)
        .unwrap_or(holder)
}

/// Ephemeral observer authority resolved from the incumbent Context slot for
/// one WindowProxy callback. It is never cached beyond that callback.
#[derive(Clone, Copy)]
struct CrossOriginWindowObserver {
    host_ptr: *mut JsContextHost,
    identity: WindowExecutionContextIdentity,
}

impl CrossOriginWindowObserver {
    fn resolve(scope: &mut v8::PinScope<'_, '_>) -> Option<Self> {
        let context = cross_origin_accessing_context(scope);
        let host_ptr = context_host_ptr_from_context_slot(context)?;
        let identity =
            unsafe { &*host_ptr }.window_execution_context_identity_for_access_check(context)?;
        Some(Self { host_ptr, identity })
    }

    fn can_access_child(self, target_host_ptr: *mut JsContextHost, handle: DomHandle) -> bool {
        let target_scope = super::super::OwnerDispatchScope::Child(handle);
        let accessing_host = unsafe { &*self.host_ptr };
        if self.host_ptr == target_host_ptr {
            return accessing_host
                .window_execution_context_can_access_dispatch_scope(self.identity, target_scope);
        }
        let target_host = unsafe { &*target_host_ptr };
        accessing_host.window_execution_context_can_access_related_page_dispatch_scope(
            self.identity,
            target_host,
            target_scope,
        )
    }

    fn can_navigate(
        self,
        scope: &mut v8::PinScope<'_, '_>,
        target_host_ptr: *mut JsContextHost,
        target_scope: super::super::OwnerDispatchScope,
        destination_url: &url::Url,
    ) -> Result<(), super::super::BrowsingContextNavigationDenial> {
        let accessing_host = unsafe { &*self.host_ptr };
        let target_host = unsafe { &*target_host_ptr };
        accessing_host.can_navigate_browsing_context(
            scope,
            self.identity,
            target_host,
            target_scope,
            destination_url,
        )
    }

    fn navigation_api_base_url(self, scope: &mut v8::PinScope<'_, '_>) -> Option<url::Url> {
        unsafe { &*self.host_ptr }.navigation_api_base_url_for_identity(scope, self.identity)
    }

    fn can_navigate_remote_top_level(
        self,
        target: &crate::script_vm::RendererRemoteTopLevelWindowProxyTarget,
        destination_url: &url::Url,
    ) -> Result<(), super::super::BrowsingContextNavigationDenial> {
        unsafe { &*self.host_ptr }.can_navigate_remote_top_level_browsing_context(
            self.identity,
            target,
            destination_url,
        )
    }

    fn can_navigate_remote_frame(
        self,
        target: &crate::script_vm::RendererRemoteFrameSnapshot,
        destination_url: &url::Url,
    ) -> Result<(), super::super::BrowsingContextNavigationDenial> {
        unsafe { &*self.host_ptr }.can_navigate_remote_frame_browsing_context(
            self.identity,
            target,
            destination_url,
        )
    }

    fn top_level_navigation_source(self) -> Option<crate::RendererTopLevelNavigationSource> {
        unsafe { &*self.host_ptr }.renderer_top_level_navigation_source_for_dispatch_scope(
            self.identity.dispatch_scope(),
            false,
        )
    }

    fn admitted_remote_javascript_url_source(
        self,
        scope: &mut v8::PinScope<'_, '_>,
        target_url: &url::Url,
    ) -> Option<crate::runtime::RendererRemoteJavaScriptUrlSource> {
        if target_url.scheme() != "javascript" {
            return None;
        }
        let source = crate::native_bridge::javascript_url_csp_source(target_url);
        let host = unsafe { &mut *self.host_ptr };
        if !host.allows_inline_javascript_navigation_by_csp(
            scope,
            self.identity.dispatch_scope(),
            &source,
        ) {
            return None;
        }
        host.renderer_remote_javascript_url_source(self.identity, false)
    }

    fn remote_frame_navigation_request(
        self,
        target_url: &url::Url,
    ) -> Option<super::super::ChildBrowsingContextNavigationRequest> {
        let source = self.top_level_navigation_source()?;
        Some(
            super::super::ChildBrowsingContextNavigationRequest::get_from_top_level_source(
                target_url.clone(),
                Some(&source),
            ),
        )
    }

    fn append_remote_command(
        self,
        command: anyhow::Result<crate::runtime::RendererRemoteWindowProxyCommand>,
    ) -> bool {
        let command = match command {
            Ok(command) => command,
            Err(error) => {
                tracing::warn!(%error, "rejected cross-origin RemoteWindowProxy command");
                return false;
            }
        };
        unsafe { &*self.host_ptr }.append_live_turn_owner_action(
            crate::runtime::RendererOwnerAction::RemoteWindowProxy(command),
        )
    }

    fn admits_remote_focus(
        self,
        target: &crate::script_vm::RendererRemoteTopLevelWindowProxyTarget,
    ) -> bool {
        if target.active {
            return false;
        }
        let source_endpoint = unsafe { &*self.host_ptr }.top_level_window_proxy_endpoint_id();
        let opener_exemption = source_endpoint == target.opener_endpoint;
        let consumed_interaction =
            unsafe { &mut *self.host_ptr }.consume_transient_user_activation_for_window_focus();
        consumed_interaction || opener_exemption
    }
}

/// Ephemeral target registry authority resolved for one WindowProxy callback.
/// The observer is resolved separately because incumbent and target Contexts
/// may belong to different related Page hosts in the same script agent.
#[derive(Clone, Copy)]
enum CrossOriginWindowChildRegistryOwner {
    Local {
        host_ptr: *mut JsContextHost,
        parent: Option<DomHandle>,
        observer: Option<CrossOriginWindowObserver>,
    },
    Remote {
        observer: CrossOriginWindowObserver,
        endpoint: TopLevelWindowProxyEndpointId,
        parent: Option<crate::script_vm::RendererRemoteFrameToken>,
    },
}

#[derive(Clone, Copy)]
enum CrossOriginWindowChildTarget {
    Local(DomHandle),
    Remote(crate::script_vm::RendererRemoteFrameToken),
}

impl CrossOriginWindowChildRegistryOwner {
    fn sync(self, scope: &mut v8::PinScope<'_, '_>) -> bool {
        match self {
            Self::Local {
                host_ptr, parent, ..
            } => {
                let host = unsafe { &mut *host_ptr };
                let root = match parent {
                    Some(parent) => {
                        let Some(document) = host.child_browsing_context_document_handle(parent)
                        else {
                            return false;
                        };
                        document
                    }
                    None => host.document_handle(),
                };
                host.sync_child_browsing_context_subtree(scope, root);
                true
            }
            Self::Remote {
                observer,
                endpoint,
                parent,
            } => unsafe { &*observer.host_ptr }
                .page_script_environment
                .as_ref()
                .and_then(|environment| environment.remote_frame_direct_children(endpoint, parent))
                .is_some(),
        }
    }

    fn child_count(self) -> usize {
        match self {
            Self::Local {
                host_ptr, parent, ..
            } => {
                let host = unsafe { &*host_ptr };
                match parent {
                    Some(parent) => host
                        .child_browsing_context_child_frame_handles(parent)
                        .len(),
                    None => host.child_browsing_context_count(),
                }
            }
            Self::Remote {
                observer,
                endpoint,
                parent,
            } => unsafe { &*observer.host_ptr }
                .page_script_environment
                .as_ref()
                .and_then(|environment| environment.remote_frame_direct_children(endpoint, parent))
                .map_or(0, |children| children.len()),
        }
    }

    fn child_by_index(self, index: usize) -> Option<CrossOriginWindowChildTarget> {
        match self {
            Self::Local {
                host_ptr, parent, ..
            } => {
                let host = unsafe { &*host_ptr };
                match parent {
                    Some(parent) => {
                        host.child_browsing_context_child_frame_handle_by_index(parent, index)
                    }
                    None => host.child_browsing_context_handle_by_index(index),
                }
                .map(CrossOriginWindowChildTarget::Local)
            }
            Self::Remote {
                observer,
                endpoint,
                parent,
            } => unsafe { &*observer.host_ptr }
                .page_script_environment
                .as_ref()?
                .remote_frame_direct_children(endpoint, parent)?
                .get(index)
                .map(|snapshot| CrossOriginWindowChildTarget::Remote(snapshot.token)),
        }
    }

    fn named_child(self, name: &str) -> Option<CrossOriginWindowChildTarget> {
        match self {
            Self::Local {
                host_ptr, parent, ..
            } => unsafe { &*host_ptr }
                .child_browsing_context_named_child_handle(parent, name)
                .map(CrossOriginWindowChildTarget::Local),
            Self::Remote {
                observer,
                endpoint,
                parent,
            } => unsafe { &*observer.host_ptr }
                .page_script_environment
                .as_ref()?
                .remote_frame_direct_children(endpoint, parent)?
                .into_iter()
                .find(|snapshot| snapshot.name == name)
                .map(|snapshot| CrossOriginWindowChildTarget::Remote(snapshot.token)),
        }
    }

    fn child_window<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        child: CrossOriginWindowChildTarget,
    ) -> Option<v8::Local<'s, v8::Object>> {
        match (self, child) {
            (
                Self::Local {
                    host_ptr, observer, ..
                },
                CrossOriginWindowChildTarget::Local(handle),
            ) => {
                let observer_can_access =
                    observer.is_some_and(|observer| observer.can_access_child(host_ptr, handle));
                let host = unsafe { &mut *host_ptr };
                let window = if observer_can_access {
                    host.child_browsing_context_window_wrapper_for_authorized_observer(
                        scope, handle,
                    )
                } else {
                    host.child_browsing_context_window_proxy_for_top(scope, handle)
                };
                if window.is_some() {
                    host.mark_child_browsing_context_window_proxy_exposed(handle);
                }
                window
            }
            (Self::Remote { observer, .. }, CrossOriginWindowChildTarget::Remote(token)) => {
                unsafe { &mut *observer.host_ptr }.remote_frame_window_proxy_for_token(scope, token)
            }
            _ => None,
        }
    }
}

fn cross_origin_window_child_registry_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<'s, v8::Object>,
) -> Option<CrossOriginWindowChildRegistryOwner> {
    let observer = CrossOriginWindowObserver::resolve(scope);
    if let Some(parent) = cross_origin_remote_frame_token(scope, holder) {
        return Some(CrossOriginWindowChildRegistryOwner::Remote {
            observer: observer?,
            endpoint: parent.endpoint,
            parent: Some(parent),
        });
    }
    if cross_origin_related_top_window_endpoint(scope, holder).is_some() {
        return match resolve_cross_origin_live_top_window(scope, holder)? {
            ResolvedCrossOriginRelatedTopWindow::Local { host_ptr, .. } => {
                Some(CrossOriginWindowChildRegistryOwner::Local {
                    host_ptr,
                    parent: None,
                    observer,
                })
            }
            ResolvedCrossOriginRelatedTopWindow::Remote(target) => {
                Some(CrossOriginWindowChildRegistryOwner::Remote {
                    observer: observer?,
                    endpoint: target.endpoint,
                    parent: None,
                })
            }
        };
    }

    let parent = child_handle_from_object(scope, holder)?;
    let holder_context = holder.get_creation_context(scope)?;
    let host_ptr = context_host_ptr_from_context_slot(holder_context)?;
    let host = unsafe { &*host_ptr };
    if !host.child_browsing_context_is_live(parent)
        || host
            .child_browsing_context_document_handle(parent)
            .is_none()
    {
        return None;
    }
    Some(CrossOriginWindowChildRegistryOwner::Local {
        host_ptr,
        parent: Some(parent),
        observer,
    })
}

fn cross_origin_window_child_by_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<'s, v8::Object>,
    index: u32,
) -> Option<Option<v8::Local<'s, v8::Object>>> {
    let owner = cross_origin_window_child_registry_owner(scope, holder)?;
    if !owner.sync(scope) {
        return None;
    }
    let Some(child) = owner.child_by_index(index as usize) else {
        return Some(None);
    };
    Some(owner.child_window(scope, child))
}

fn cross_origin_window_child_count<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<'s, v8::Object>,
) -> Option<usize> {
    let owner = cross_origin_window_child_registry_owner(scope, holder)?;
    owner.sync(scope).then(|| owner.child_count())
}

fn cross_origin_named_child_can_shadow(name: &str) -> bool {
    !name.is_empty()
        && name.parse::<u32>().is_err()
        && !CROSS_ORIGIN_WINDOW_EXPOSED_PROPERTY_NAMES.contains(&name)
}

enum CrossOriginWindowNamedChildLookup<'s> {
    Fallback,
    Missing,
    Value(v8::Local<'s, v8::Object>),
}

fn cross_origin_window_named_child<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<'s, v8::Object>,
    key: v8::Local<'s, v8::Name>,
) -> CrossOriginWindowNamedChildLookup<'s> {
    let Ok(key) = v8::Local::<v8::String>::try_from(key) else {
        return CrossOriginWindowNamedChildLookup::Fallback;
    };
    let name = key.to_rust_string_lossy(scope);
    if !cross_origin_named_child_can_shadow(&name) {
        return CrossOriginWindowNamedChildLookup::Fallback;
    }
    let Some(owner) = cross_origin_window_child_registry_owner(scope, holder) else {
        return CrossOriginWindowNamedChildLookup::Fallback;
    };
    if !owner.sync(scope) {
        return CrossOriginWindowNamedChildLookup::Fallback;
    }
    let Some(child) = owner.named_child(&name) else {
        return if name == "then" {
            CrossOriginWindowNamedChildLookup::Fallback
        } else {
            CrossOriginWindowNamedChildLookup::Missing
        };
    };
    owner
        .child_window(scope, child)
        .map(CrossOriginWindowNamedChildLookup::Value)
        .unwrap_or(CrossOriginWindowNamedChildLookup::Missing)
}

fn child_window_cross_origin_handler_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>)> {
    if let Some(surface) =
        get_private_value(scope, holder, CLOSED_TOP_LEVEL_WINDOW_ACCESS_SURFACE_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Some((surface, holder));
    }
    let holder_context = holder.get_creation_context(scope)?;
    let host_ptr = crate::util::context_host_ptr_from_context_slot(holder_context)?;
    let host = unsafe { &*host_ptr };
    if let Some(identity) = host.window_execution_context_identity_for_access_check(holder_context)
        && host.window_execution_context_identity_is_current(identity)
    {
        if !host.window_execution_context_identity_is_default_world(identity) {
            return None;
        }
        return match identity.dispatch_scope() {
            super::super::OwnerDispatchScope::Top => {
                host.top_level_cross_origin_window_handler_data(scope, holder)
            }
            super::super::OwnerDispatchScope::Child(handle) => host
                .child_window_proxy_records
                .cross_origin_handler_data(scope, handle),
        };
    }

    // A WindowProxy belongs to the browsing context, not to one LocalWindow
    // generation. A parked facade context keeps the stable proxy attached
    // while the previous realm is retired and the replacement is pending.
    let handle = holder_context
        .get_slot::<ChildWindowProxyFacadeContextHandle>()?
        .0;
    if !host.child_browsing_context_is_live(handle) {
        return None;
    }
    host.child_window_proxy_records
        .cross_origin_handler_data(scope, handle)
}

fn child_window_cross_origin_identity_name(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<'_, v8::Name>,
) -> bool {
    v8::Local::<v8::String>::try_from(key)
        .ok()
        .map(|key| key.to_rust_string_lossy(scope))
        .is_some_and(|key| matches!(key.as_str(), "self" | "window" | "frames"))
}

fn child_window_cross_origin_named_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let holder = args.holder();
    if child_window_cross_origin_identity_name(scope, key) {
        rv.set(child_window_cross_origin_proxy_self(scope, holder).into());
        return v8::Intercepted::kYes;
    }
    match cross_origin_window_named_child(scope, holder, key) {
        CrossOriginWindowNamedChildLookup::Value(child) => {
            rv.set(child.into());
            return v8::Intercepted::kYes;
        }
        CrossOriginWindowNamedChildLookup::Missing => {
            throw_cross_origin_location_security_error(scope);
            return v8::Intercepted::kYes;
        }
        CrossOriginWindowNamedChildLookup::Fallback => {}
    }
    let Some(surface) = child_window_cross_origin_access_surface(scope, holder) else {
        throw_cross_origin_location_security_error(scope);
        return v8::Intercepted::kYes;
    };
    if let Ok(key_string) = v8::Local::<v8::String>::try_from(key)
        && surface.has_own_property(scope, key).unwrap_or(false)
    {
        let name = key_string.to_rust_string_lossy(scope);
        if let Some(function) = cross_origin_window_method_function(scope, &name) {
            rv.set(function.into());
            return v8::Intercepted::kYes;
        }
    }
    if surface.has_own_property(scope, key).unwrap_or(false)
        && let Some(value) = surface.get(scope, key.into())
    {
        rv.set(value);
        return v8::Intercepted::kYes;
    }
    throw_cross_origin_location_security_error(scope);
    v8::Intercepted::kYes
}

fn child_window_cross_origin_named_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    value: v8::Local<'s, v8::Value>,
    args: v8::PropertyCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    let Some(surface) = child_window_cross_origin_access_surface(scope, args.holder()) else {
        throw_cross_origin_location_security_error(scope);
        return v8::Intercepted::kYes;
    };
    let is_location = v8::Local::<v8::String>::try_from(key)
        .ok()
        .map(|key| key.to_rust_string_lossy(scope))
        .as_deref()
        == Some("location");
    if is_location {
        let _ = surface.set(scope, key.into(), value);
        return v8::Intercepted::kYes;
    }
    throw_cross_origin_location_security_error(scope);
    v8::Intercepted::kYes
}

fn child_window_cross_origin_named_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    if child_window_cross_origin_identity_name(scope, key) {
        rv.set_int32(cross_origin_property_attributes().as_u32() as i32);
        return v8::Intercepted::kYes;
    }
    match cross_origin_window_named_child(scope, args.holder(), key) {
        CrossOriginWindowNamedChildLookup::Value(_) => {
            rv.set_int32(cross_origin_named_property_attributes().as_u32() as i32);
            return v8::Intercepted::kYes;
        }
        CrossOriginWindowNamedChildLookup::Missing => {
            throw_cross_origin_location_security_error(scope);
            return v8::Intercepted::kYes;
        }
        CrossOriginWindowNamedChildLookup::Fallback => {}
    }
    let Some(surface) = child_window_cross_origin_access_surface(scope, args.holder()) else {
        throw_cross_origin_location_security_error(scope);
        return v8::Intercepted::kYes;
    };
    if surface.has_own_property(scope, key).unwrap_or(false) {
        rv.set_int32(cross_origin_property_attributes().as_u32() as i32);
        return v8::Intercepted::kYes;
    }
    throw_cross_origin_location_security_error(scope);
    v8::Intercepted::kYes
}

fn child_window_cross_origin_named_deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _key: v8::Local<'s, v8::Name>,
    _args: v8::PropertyCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    throw_cross_origin_location_security_error(scope);
    v8::Intercepted::kYes
}

fn child_window_cross_origin_named_definer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _key: v8::Local<'s, v8::Name>,
    _descriptor: &v8::PropertyDescriptor,
    _args: v8::PropertyCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    throw_cross_origin_location_security_error(scope);
    v8::Intercepted::kYes
}

fn child_window_cross_origin_named_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback_args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let Some(surface) = child_window_cross_origin_access_surface(scope, callback_args.holder())
    else {
        rv.set(v8::Array::new(scope, 0));
        return;
    };
    rv.set(ordered_cross_origin_window_own_keys(scope, surface, false));
}

fn ordered_cross_origin_window_own_keys<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    include_indices: bool,
) -> v8::Local<'s, v8::Array> {
    let mut property_names = v8::GetPropertyNamesArgsBuilder::new();
    property_names.property_filter(v8::PropertyFilter::ALL_PROPERTIES);
    property_names.key_conversion(v8::KeyConversionMode::ConvertToString);
    let names = target
        .get_own_property_names(scope, property_names.build())
        .unwrap_or_else(|| v8::Array::new(scope, 0));
    let mut indices = Vec::new();
    let mut exposed_names = Vec::new();
    let mut then = None;
    let mut symbols = Vec::new();
    for index in 0..names.length() {
        let Some(key) = names.get_index(scope, index) else {
            continue;
        };
        if let Ok(name) = v8::Local::<v8::String>::try_from(key) {
            let name = name.to_rust_string_lossy(scope);
            let array_index = name
                .parse::<u32>()
                .ok()
                .filter(|index| *index != u32::MAX)
                .is_some_and(|index| index.to_string() == name);
            if array_index {
                if include_indices {
                    indices.push(key);
                }
                continue;
            }
            if name == "then" {
                then = Some(key);
            } else if CROSS_ORIGIN_WINDOW_EXPOSED_PROPERTY_NAMES.contains(&name.as_str()) {
                exposed_names.push(key);
            }
        } else if key.is_symbol() {
            symbols.push(key);
        }
    }
    indices.append(&mut exposed_names);
    if let Some(then) = then {
        indices.push(then);
    }
    indices.append(&mut symbols);
    v8::Array::new_with_elements(scope, &indices)
}

fn child_window_cross_origin_named_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    match cross_origin_window_named_child(scope, args.holder(), key) {
        CrossOriginWindowNamedChildLookup::Value(child) => {
            let Ok(descriptor) =
                CrossOriginPropertyDescriptorDeclaration::new(child.into(), false, false, true)
                    .bind(scope)
            else {
                return v8::Intercepted::kNo;
            };
            rv.set(descriptor.into());
            return v8::Intercepted::kYes;
        }
        CrossOriginWindowNamedChildLookup::Missing => {
            throw_cross_origin_location_security_error(scope);
            return v8::Intercepted::kYes;
        }
        CrossOriginWindowNamedChildLookup::Fallback => {}
    }
    let Some(surface) = child_window_cross_origin_access_surface(scope, args.holder()) else {
        throw_cross_origin_location_security_error(scope);
        return v8::Intercepted::kYes;
    };
    if !surface.has_own_property(scope, key).unwrap_or(false) {
        throw_cross_origin_location_security_error(scope);
        return v8::Intercepted::kYes;
    }
    if let Ok(key_string) = v8::Local::<v8::String>::try_from(key) {
        let name = key_string.to_rust_string_lossy(scope);
        if let Some(descriptor) = cross_origin_window_method_descriptor(scope, &name) {
            rv.set(descriptor.into());
            return v8::Intercepted::kYes;
        }
        if let Some(descriptor) = cross_origin_window_attribute_descriptor(scope, &name) {
            rv.set(descriptor.into());
            return v8::Intercepted::kYes;
        }
    }
    let Some(descriptor) = surface.get_own_property_descriptor(scope, key) else {
        throw_cross_origin_location_security_error(scope);
        return v8::Intercepted::kYes;
    };
    rv.set(descriptor);
    v8::Intercepted::kYes
}

fn child_window_cross_origin_indexed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    match cross_origin_window_child_by_index(scope, args.holder(), index) {
        Some(Some(child)) => {
            rv.set(child.into());
            return v8::Intercepted::kYes;
        }
        Some(None) => {
            throw_cross_origin_location_security_error(scope);
            return v8::Intercepted::kYes;
        }
        None => {}
    }
    let Some(surface) = child_window_cross_origin_access_surface(scope, args.holder()) else {
        throw_cross_origin_location_security_error(scope);
        return v8::Intercepted::kYes;
    };
    let Some(key) = v8_string(scope, &index.to_string()) else {
        throw_cross_origin_location_security_error(scope);
        return v8::Intercepted::kYes;
    };
    if !surface.has_own_property(scope, key.into()).unwrap_or(false) {
        throw_cross_origin_location_security_error(scope);
        return v8::Intercepted::kYes;
    }
    match surface.get(scope, key.into()) {
        Some(value) => rv.set(value),
        None => throw_cross_origin_location_security_error(scope),
    }
    v8::Intercepted::kYes
}

fn child_window_cross_origin_indexed_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _index: u32,
    _value: v8::Local<'s, v8::Value>,
    _args: v8::PropertyCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    throw_cross_origin_location_security_error(scope);
    v8::Intercepted::kYes
}

fn child_window_cross_origin_indexed_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    match cross_origin_window_child_by_index(scope, args.holder(), index) {
        Some(Some(_)) => {
            rv.set_int32(cross_origin_index_property_attributes().as_u32() as i32);
            return v8::Intercepted::kYes;
        }
        Some(None) => {
            throw_cross_origin_location_security_error(scope);
            return v8::Intercepted::kYes;
        }
        None => {}
    }
    let Some(surface) = child_window_cross_origin_access_surface(scope, args.holder()) else {
        throw_cross_origin_location_security_error(scope);
        return v8::Intercepted::kYes;
    };
    let Some(key) = v8_string(scope, &index.to_string()) else {
        throw_cross_origin_location_security_error(scope);
        return v8::Intercepted::kYes;
    };
    if !surface.has_own_property(scope, key.into()).unwrap_or(false) {
        throw_cross_origin_location_security_error(scope);
        return v8::Intercepted::kYes;
    }
    rv.set_int32(cross_origin_index_property_attributes().as_u32() as i32);
    v8::Intercepted::kYes
}

fn child_window_cross_origin_indexed_deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _index: u32,
    _args: v8::PropertyCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    throw_cross_origin_location_security_error(scope);
    v8::Intercepted::kYes
}

fn child_window_cross_origin_indexed_definer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _index: u32,
    _descriptor: &v8::PropertyDescriptor,
    _args: v8::PropertyCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    throw_cross_origin_location_security_error(scope);
    v8::Intercepted::kYes
}

fn child_window_cross_origin_indexed_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let count = cross_origin_window_child_count(scope, args.holder()).unwrap_or_else(|| {
        child_window_cross_origin_access_surface(scope, args.holder())
            .and_then(|surface| child_handle_from_object(scope, surface))
            .and_then(|handle| {
                context_host_ptr_from_global_bridge(scope).map(|host_ptr| {
                    unsafe { &mut *host_ptr }.child_browsing_context_child_frame_count(handle)
                })
            })
            .unwrap_or(0)
    });
    let array = serialize_v8_iter_array(
        scope,
        (0..count.min(u32::MAX as usize)).map(|index| index as u32),
    )
    .unwrap_or_else(|| v8::Array::new(scope, 0));
    rv.set(array);
}

fn child_window_cross_origin_indexed_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    match cross_origin_window_child_by_index(scope, args.holder(), index) {
        Some(Some(child)) => {
            let Ok(descriptor) =
                CrossOriginPropertyDescriptorDeclaration::new(child.into(), false, true, true)
                    .bind(scope)
            else {
                return v8::Intercepted::kNo;
            };
            rv.set(descriptor.into());
            return v8::Intercepted::kYes;
        }
        Some(None) => {
            throw_cross_origin_location_security_error(scope);
            return v8::Intercepted::kYes;
        }
        None => {}
    }
    let Some(surface) = child_window_cross_origin_access_surface(scope, args.holder()) else {
        throw_cross_origin_location_security_error(scope);
        return v8::Intercepted::kYes;
    };
    let Some(key) = v8_string(scope, &index.to_string()) else {
        throw_cross_origin_location_security_error(scope);
        return v8::Intercepted::kYes;
    };
    if !surface.has_own_property(scope, key.into()).unwrap_or(false) {
        throw_cross_origin_location_security_error(scope);
        return v8::Intercepted::kYes;
    }
    let Some(value) = surface.get(scope, key.into()) else {
        throw_cross_origin_location_security_error(scope);
        return v8::Intercepted::kYes;
    };
    let Ok(descriptor) =
        CrossOriginPropertyDescriptorDeclaration::new(value, false, true, true).bind(scope)
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}

fn cross_origin_window_denied_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    throw_cross_origin_location_security_error(scope);
}

fn cross_origin_window_proxy_get_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_cross_origin_location_security_error(scope);
        return;
    };
    let Ok(key) = v8::Local::<v8::Name>::try_from(args.get(1)) else {
        throw_cross_origin_location_security_error(scope);
        return;
    };
    if !target.has_own_property(scope, key).unwrap_or(false) {
        throw_cross_origin_location_security_error(scope);
        return;
    }
    if let Ok(key_string) = v8::Local::<v8::String>::try_from(key) {
        let name = key_string.to_rust_string_lossy(scope);
        if let Some(function) = cross_origin_window_method_function(scope, &name) {
            rv.set(function.into());
            return;
        }
    }
    let receiver = v8::Local::<v8::Object>::try_from(args.get(2)).unwrap_or(target);
    match target.get_with_receiver(scope, key.into(), receiver) {
        Some(value) => rv.set(value),
        None => rv.set_undefined(),
    }
}

fn cross_origin_window_proxy_has_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_cross_origin_location_security_error(scope);
        return;
    };
    let Ok(key) = v8::Local::<v8::Name>::try_from(args.get(1)) else {
        throw_cross_origin_location_security_error(scope);
        return;
    };
    if target.has_own_property(scope, key).unwrap_or(false) {
        rv.set_bool(true);
        return;
    }
    throw_cross_origin_location_security_error(scope);
}

fn cross_origin_window_proxy_set_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_cross_origin_location_security_error(scope);
        return;
    };
    let Ok(key) = v8::Local::<v8::String>::try_from(args.get(1)) else {
        throw_cross_origin_location_security_error(scope);
        return;
    };
    if key.to_rust_string_lossy(scope) != "location"
        || !target.has_own_property(scope, key.into()).unwrap_or(false)
    {
        throw_cross_origin_location_security_error(scope);
        return;
    }
    let receiver = v8::Local::<v8::Object>::try_from(args.get(3)).unwrap_or(target);
    let assigned = target
        .set_with_receiver(scope, key.into(), args.get(2), receiver)
        .unwrap_or(false);
    rv.set_bool(assigned);
}

fn cross_origin_window_proxy_get_own_property_descriptor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_cross_origin_location_security_error(scope);
        return;
    };
    let Ok(key) = v8::Local::<v8::Name>::try_from(args.get(1)) else {
        throw_cross_origin_location_security_error(scope);
        return;
    };
    if !target.has_own_property(scope, key).unwrap_or(false) {
        throw_cross_origin_location_security_error(scope);
        return;
    }
    if let Ok(key_string) = v8::Local::<v8::String>::try_from(key) {
        let name = key_string.to_rust_string_lossy(scope);
        if let Some(descriptor) = cross_origin_window_method_descriptor(scope, &name) {
            rv.set(descriptor.into());
            return;
        }
        if let Some(descriptor) = cross_origin_window_attribute_descriptor(scope, &name) {
            rv.set(descriptor.into());
            return;
        }
    }
    let Some(descriptor) = target.get_own_property_descriptor(scope, key) else {
        throw_cross_origin_location_security_error(scope);
        return;
    };
    rv.set(descriptor);
}

fn cross_origin_window_proxy_own_keys_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_cross_origin_location_security_error(scope);
        return;
    };
    rv.set(ordered_cross_origin_window_own_keys(scope, target, true).into());
}

fn cross_origin_window_proxy_set_prototype_of_callback<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(args.get(1).is_null());
}

fn cross_origin_window_proxy_prevent_extensions_callback<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(false);
}

pub(crate) fn is_cross_origin_location_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    get_cross_origin_proxy_private_value(scope, object, CROSS_ORIGIN_LOCATION_PROXY_SLOT).is_some()
}

pub(crate) fn is_cross_origin_top_window_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    get_cross_origin_proxy_private_value(scope, object, CROSS_ORIGIN_TOP_WINDOW_PROXY_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

pub(crate) fn top_level_window_proxy_is_finally_closed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    get_cross_origin_proxy_private_value(scope, object, CLOSED_TOP_LEVEL_WINDOW_MARKER_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

fn cross_origin_accessing_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Context> {
    // The access surface lives in the target realm, while V8 exposes the
    // initiating Realm as the incumbent context for cross-origin callbacks.
    // Engine entry points without an incumbent script fall back to the
    // currently entered Context.
    scope
        .get_incumbent_context()
        .unwrap_or_else(|| scope.get_current_context())
}

pub(crate) fn throw_cross_origin_location_security_error(scope: &mut v8::PinScope<'_, '_>) {
    let accessing_context = {
        let context = cross_origin_accessing_context(scope);
        v8::Global::new(scope, context)
    };
    let accessing_context = v8::Local::new(scope, &accessing_context);
    if accessing_context == scope.get_current_context() {
        throw_dom_exception(scope, "SecurityError", 18, CROSS_ORIGIN_ACCESS_ERROR);
        return;
    }
    let accessing_scope = &mut v8::ContextScope::new(scope, accessing_context);
    throw_dom_exception(
        accessing_scope,
        "SecurityError",
        18,
        CROSS_ORIGIN_ACCESS_ERROR,
    );
}

fn is_cross_origin_location_href_key_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Value>,
) -> bool {
    let Ok(key) = v8::Local::<v8::String>::try_from(key) else {
        return false;
    };
    key.to_rust_string_lossy(scope) == "href"
}

fn cross_origin_location_proxy_get_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if is_cross_origin_location_href_key_value(scope, args.get(1)) {
        throw_cross_origin_location_security_error(scope);
        return;
    }
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set_undefined();
        return;
    };
    let Ok(key) = v8::Local::<v8::Name>::try_from(args.get(1)) else {
        throw_cross_origin_location_security_error(scope);
        return;
    };
    if !target.has_own_property(scope, key).unwrap_or(false) {
        throw_cross_origin_location_security_error(scope);
        return;
    }
    if let Ok(key_string) = v8::Local::<v8::String>::try_from(key)
        && key_string.to_rust_string_lossy(scope) == "replace"
        && let Some(function) = cross_origin_location_replace_function(scope)
    {
        rv.set(function.into());
        return;
    }
    let receiver = v8::Local::<v8::Object>::try_from(args.get(2)).unwrap_or(target);
    match target.get_with_receiver(scope, key.into(), receiver) {
        Some(value) => rv.set(value),
        None => rv.set_undefined(),
    }
}

fn cross_origin_location_proxy_has_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_cross_origin_location_security_error(scope);
        return;
    };
    let Ok(key) = v8::Local::<v8::Name>::try_from(args.get(1)) else {
        throw_cross_origin_location_security_error(scope);
        return;
    };
    if target.has_own_property(scope, key).unwrap_or(false) {
        rv.set_bool(true);
        return;
    }
    throw_cross_origin_location_security_error(scope);
}

fn cross_origin_location_proxy_set_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    receiver: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(expected) = get_private_value(scope, target, CROSS_ORIGIN_LOCATION_PROXY_SELF_SLOT)
        && receiver.strict_equals(expected)
    {
        return Some(target);
    }
    let receiver = v8::Local::<v8::Object>::try_from(receiver).ok()?;
    is_cross_origin_location_proxy(scope, receiver).then_some(receiver)
}

fn cross_origin_location_proxy_set_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    if !is_cross_origin_location_href_key_value(scope, args.get(1)) {
        throw_cross_origin_location_security_error(scope);
        return;
    }
    let Some(receiver) = cross_origin_location_proxy_set_receiver(scope, target, args.get(3))
    else {
        throw_cross_origin_illegal_invocation(scope);
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let navigated = cross_origin_location_navigate(scope, receiver, args.get(2));
    rv.set(v8::Boolean::new(scope, navigated).into());
}

fn cross_origin_location_proxy_get_own_property_descriptor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_cross_origin_location_security_error(scope);
        return;
    };
    let Ok(key) = v8::Local::<v8::Name>::try_from(args.get(1)) else {
        throw_cross_origin_location_security_error(scope);
        return;
    };
    if !target.has_own_property(scope, key).unwrap_or(false) {
        throw_cross_origin_location_security_error(scope);
        return;
    }
    if let Ok(key_string) = v8::Local::<v8::String>::try_from(key) {
        match key_string.to_rust_string_lossy(scope).as_str() {
            "replace" => {
                if let Some(descriptor) = cross_origin_location_replace_descriptor(scope) {
                    rv.set(descriptor.into());
                    return;
                }
            }
            "href" => {
                if let Some(descriptor) = cross_origin_location_href_descriptor(scope) {
                    rv.set(descriptor.into());
                    return;
                }
            }
            _ => {}
        }
    }
    let Some(descriptor) = target.get_own_property_descriptor(scope, key) else {
        throw_cross_origin_location_security_error(scope);
        return;
    };
    rv.set(descriptor);
}

fn cross_origin_location_proxy_set_prototype_of_callback<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(args.get(1).is_null());
}

fn cross_origin_location_proxy_prevent_extensions_callback<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(false);
}

fn is_cross_origin_window_function_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> bool {
    child_handle_from_object(scope, receiver).is_some()
        || cross_origin_related_top_window_endpoint(scope, receiver).is_some()
        || is_cross_origin_local_top_window(scope, receiver)
        || get_cross_origin_proxy_private_value(
            scope,
            receiver,
            DETACHED_CROSS_ORIGIN_WINDOW_PROXY_SLOT,
        )
        .is_some()
        || get_cross_origin_proxy_private_value(
            scope,
            receiver,
            CLOSED_TOP_LEVEL_WINDOW_ACCESS_SURFACE_SLOT,
        )
        .is_some()
}

fn cross_origin_window_function_receiver_surface<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if !is_cross_origin_window_function_receiver(scope, receiver) {
        return None;
    }
    let storage = cross_origin_proxy_storage_object(scope, receiver);
    if get_private_value(scope, storage, DETACHED_CROSS_ORIGIN_WINDOW_PROXY_SLOT).is_some() {
        return Some(storage);
    }
    child_window_cross_origin_access_surface(scope, receiver)
}

fn cross_origin_window_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    name: &'static str,
) {
    let receiver = args.this();
    let Some(surface) = cross_origin_window_function_receiver_surface(scope, receiver) else {
        throw_cross_origin_illegal_invocation(scope);
        return;
    };
    if matches!(name, "window" | "self" | "frames") {
        rv.set(receiver.into());
        return;
    }
    let Some(surface_context) = surface.get_creation_context(scope) else {
        throw_cross_origin_illegal_invocation(scope);
        return;
    };
    let surface = v8::Global::new(scope, surface);
    let surface_context = v8::Global::new(scope, surface_context);
    let surface_context = v8::Local::new(scope, &surface_context);
    let surface_scope = &mut v8::ContextScope::new(scope, surface_context);
    let surface = v8::Local::new(surface_scope, &surface);
    match surface.get(surface_scope, v8str(surface_scope, name).into()) {
        Some(value) => rv.set(value),
        None => throw_cross_origin_illegal_invocation(surface_scope),
    }
}

fn cross_origin_window_location_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    cross_origin_window_attribute_getter_callback(scope, args, rv, "location");
}

fn cross_origin_window_window_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    cross_origin_window_attribute_getter_callback(scope, args, rv, "window");
}

fn cross_origin_window_frames_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    cross_origin_window_attribute_getter_callback(scope, args, rv, "frames");
}

fn cross_origin_window_self_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    cross_origin_window_attribute_getter_callback(scope, args, rv, "self");
}

fn cross_origin_window_top_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    cross_origin_window_attribute_getter_callback(scope, args, rv, "top");
}

fn cross_origin_window_parent_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    cross_origin_window_attribute_getter_callback(scope, args, rv, "parent");
}

fn cross_origin_window_opener_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    cross_origin_window_attribute_getter_callback(scope, args, rv, "opener");
}

fn cross_origin_window_closed_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    cross_origin_window_attribute_getter_callback(scope, args, rv, "closed");
}

fn cross_origin_window_length_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    cross_origin_window_attribute_getter_callback(scope, args, rv, "length");
}

fn cross_origin_window_noop_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_cross_origin_window_function_receiver(scope, args.this()) {
        throw_cross_origin_illegal_invocation(scope);
    }
}

fn cross_origin_window_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    if cross_origin_related_top_window_endpoint(scope, args.this()).is_some()
        || is_cross_origin_local_top_window(scope, args.this())
    {
        if let Some(target) = resolve_cross_origin_live_top_window(scope, args.this()) {
            match target {
                ResolvedCrossOriginRelatedTopWindow::Local {
                    context, host_ptr, ..
                } => {
                    let target_scope = &mut v8::ContextScope::new(scope, context);
                    let _ = crate::context_bootstrap::request_top_level_browsing_context_close(
                        target_scope,
                        host_ptr,
                        crate::runtime::RendererTopLevelCloseSource::Window,
                    );
                }
                ResolvedCrossOriginRelatedTopWindow::Remote(target) => {
                    if let Some(observer) = CrossOriginWindowObserver::resolve(scope) {
                        let _ = observer.append_remote_command(
                            crate::runtime::RendererRemoteWindowProxyCommand::close(
                                target.endpoint,
                                target.residence,
                                target.channel,
                            ),
                        );
                    }
                }
            }
        }
        return;
    }
    cross_origin_window_noop_callback(scope, args, rv);
}

fn cross_origin_window_focus_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    if cross_origin_related_top_window_endpoint(scope, args.this()).is_some()
        || is_cross_origin_local_top_window(scope, args.this())
    {
        if let Some(target) = resolve_cross_origin_live_top_window(scope, args.this()) {
            match target {
                ResolvedCrossOriginRelatedTopWindow::Local {
                    context, host_ptr, ..
                } => {
                    let target_scope = &mut v8::ContextScope::new(scope, context);
                    let _ = crate::context_bootstrap::request_top_level_browsing_context_focus(
                        target_scope,
                        host_ptr,
                    );
                }
                ResolvedCrossOriginRelatedTopWindow::Remote(target) => {
                    if let Some(observer) = CrossOriginWindowObserver::resolve(scope)
                        && observer.admits_remote_focus(&target)
                    {
                        let _ = observer.append_remote_command(
                            crate::runtime::RendererRemoteWindowProxyCommand::focus(
                                target.endpoint,
                                target.residence,
                                target.channel,
                            ),
                        );
                    }
                }
            }
        }
        return;
    }
    cross_origin_window_noop_callback(scope, args, rv);
}

fn cross_origin_window_closed_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(token) = cross_origin_remote_frame_token(scope, args.this()) {
        let live = CrossOriginWindowObserver::resolve(scope).is_some_and(|observer| {
            unsafe { &*observer.host_ptr }
                .page_script_environment
                .as_ref()
                .is_some_and(|environment| environment.remote_frame_snapshot(token).is_some())
        });
        rv.set_bool(!live);
        return;
    }
    if get_cross_origin_proxy_private_value(scope, args.this(), CLOSED_TOP_LEVEL_WINDOW_MARKER_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
    {
        rv.set_bool(true);
        return;
    }
    if cross_origin_related_top_window_endpoint(scope, args.this()).is_some()
        || is_cross_origin_local_top_window(scope, args.this())
    {
        let closed = resolve_cross_origin_live_top_window(scope, args.this()).is_none();
        rv.set_bool(closed);
        return;
    }
    rv.set_bool(false);
}

fn cross_origin_top_level_window_opener_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    if let Some(endpoint) = cross_origin_related_top_window_endpoint(scope, receiver)
        && let Some(observer) = CrossOriginWindowObserver::resolve(scope)
        && let Some(opener) =
            unsafe { &mut *observer.host_ptr }.related_top_level_opener_projection(scope, endpoint)
    {
        rv.set(opener);
        return;
    }
    if let Some(ResolvedCrossOriginRelatedTopWindow::Local { host_ptr, .. }) =
        resolve_cross_origin_live_top_window(scope, receiver)
        && let Some(opener) = unsafe { &*host_ptr }.top_level_opener_value(scope)
    {
        rv.set(opener);
        return;
    }
    let opener = get_cross_origin_proxy_private_value(scope, receiver, WINDOW_OPENER_SLOT)
        .unwrap_or_else(|| v8::null(scope).into());
    if let Ok(opener_window) = v8::Local::<v8::Object>::try_from(opener)
        && top_level_window_proxy_is_finally_closed(scope, opener_window)
    {
        rv.set_null();
        return;
    }
    rv.set(opener);
}

fn cross_origin_window_length_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(count) = cross_origin_window_child_count(scope, args.this()) {
        rv.set_uint32(count as u32);
        return;
    }
    if cross_origin_related_top_window_endpoint(scope, args.this()).is_some() {
        // Disconnected group endpoints have no remotely projectable child
        // tree. They remain valid cross-origin Window receivers.
        rv.set_uint32(0);
        return;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_int32(0);
        return;
    };
    if get_cross_origin_proxy_private_value(scope, args.this(), CROSS_ORIGIN_TOP_WINDOW_PROXY_SLOT)
        .is_some()
    {
        let count = unsafe { &*host_ptr }.child_browsing_context_count();
        rv.set_uint32(count as u32);
        return;
    }
    let Some(handle) = child_handle_from_object(scope, args.this()) else {
        throw_cross_origin_illegal_invocation(scope);
        return;
    };
    let count = unsafe { &mut *host_ptr }.child_browsing_context_child_frame_count(handle);
    rv.set_uint32(count as u32);
}

fn cross_origin_window_location_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(location) =
        get_cross_origin_proxy_private_value(scope, args.this(), CROSS_ORIGIN_WINDOW_LOCATION_SLOT)
    {
        rv.set(location);
    } else {
        throw_cross_origin_illegal_invocation(scope);
    }
}

fn cross_origin_location_navigate_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Ok(expected_receiver) = v8::Local::<v8::Object>::try_from(args.data()) {
        let receiver = cross_origin_proxy_storage_object(scope, args.this());
        if !receiver.strict_equals(expected_receiver.into()) {
            throw_cross_origin_illegal_invocation(scope);
            return;
        }
    }
    let _ = cross_origin_location_navigate(scope, args.this(), args.get(0));
}

fn cross_origin_window_location_attribute_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_cross_origin_window_function_receiver(scope, args.this()) {
        throw_cross_origin_illegal_invocation(scope);
        return;
    }
    let _ = cross_origin_location_navigate(scope, args.this(), args.get(0));
}

fn cross_origin_location_href_attribute_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_cross_origin_location_proxy(scope, args.this()) {
        throw_cross_origin_illegal_invocation(scope);
        return;
    }
    let _ = cross_origin_location_navigate(scope, args.this(), args.get(0));
}

fn cross_origin_location_navigate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) -> bool {
    let raw = match webidl::convert::<webidl::UsvString>(
        scope,
        value,
        webidl::Context::member("Location", "href"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return false;
        }
    };
    cross_origin_location_navigate_raw(scope, receiver, LocationNavigationKind::Assign, raw)
}

fn cross_origin_location_navigate_raw<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    kind: LocationNavigationKind,
    raw: String,
) -> bool {
    let Some(observer) = CrossOriginWindowObserver::resolve(scope) else {
        return true;
    };
    let related_endpoint = cross_origin_related_top_window_endpoint(scope, receiver);
    let local_top_window = is_cross_origin_local_top_window(scope, receiver);
    let child_handle = child_handle_from_object(scope, receiver);
    let remote_frame_token = cross_origin_remote_frame_token(scope, receiver);
    let remote_frame_target = cross_origin_remote_frame_window_target(scope, receiver);
    if related_endpoint.is_none()
        && !local_top_window
        && child_handle.is_none()
        && remote_frame_token.is_none()
    {
        throw_cross_origin_illegal_invocation(scope);
        return false;
    }
    if remote_frame_token.is_some() && remote_frame_target.is_none() {
        // A retained facade for a removed frame or replaced root Document is
        // still a valid Window receiver, but its qualified route is stale.
        return true;
    }
    if let Some((token, frame, top)) = remote_frame_target {
        let source_base_url = observer.navigation_api_base_url(scope);
        let Some(target_url) = resolve_location_navigation_target_against_entered_base(
            &frame.current_url,
            kind,
            Some(raw),
            source_base_url.as_ref(),
        ) else {
            return false;
        };
        if let Err(denial) = observer.can_navigate_remote_frame(&frame, &target_url) {
            if denial.is_sandbox_violation() {
                throw_cross_origin_location_security_error(scope);
                return false;
            }
            return true;
        }
        let javascript_source = if target_url.scheme() == "javascript" {
            let Some(source) = observer.admitted_remote_javascript_url_source(scope, &target_url)
            else {
                return true;
            };
            Some(source)
        } else {
            None
        };
        let Some(request) = observer.remote_frame_navigation_request(&target_url) else {
            return true;
        };
        let kind = match kind {
            LocationNavigationKind::Assign => {
                crate::runtime::RendererRemoteWindowProxyNavigationKind::Assign
            }
            LocationNavigationKind::Replace | LocationNavigationKind::Reload => {
                crate::runtime::RendererRemoteWindowProxyNavigationKind::Replace
            }
        };
        let command = if let Some(source) = javascript_source {
            crate::runtime::RendererRemoteWindowProxyCommand::navigate_frame_javascript_url(
                token,
                top.residence,
                top.channel,
                kind,
                request,
                source,
                None,
            )
        } else {
            crate::runtime::RendererRemoteWindowProxyCommand::navigate_frame(
                token,
                top.residence,
                top.channel,
                kind,
                request,
                None,
            )
        };
        let _ = observer.append_remote_command(command);
        return true;
    }
    let resolved_top_target = if related_endpoint.is_some() || local_top_window {
        let Some(target) = resolve_cross_origin_live_top_window(scope, receiver) else {
            // A disconnected WindowProxy endpoint remains safely callable but
            // cannot route into a replacement browsing-context group.
            return true;
        };
        match target {
            ResolvedCrossOriginRelatedTopWindow::Remote(target) => {
                let source_base_url = observer.navigation_api_base_url(scope);
                let Some(target_url) = resolve_location_navigation_target_against_entered_base(
                    &target.current_url,
                    kind,
                    Some(raw.clone()),
                    source_base_url.as_ref(),
                ) else {
                    return false;
                };
                if let Err(denial) = observer.can_navigate_remote_top_level(&target, &target_url) {
                    if denial.is_sandbox_violation() {
                        throw_cross_origin_location_security_error(scope);
                        return false;
                    }
                    return true;
                }
                let javascript_source = if target_url.scheme() == "javascript" {
                    let Some(source) =
                        observer.admitted_remote_javascript_url_source(scope, &target_url)
                    else {
                        return true;
                    };
                    Some(source)
                } else {
                    None
                };
                let Some(source) = observer.top_level_navigation_source() else {
                    return true;
                };
                let kind = match kind {
                    LocationNavigationKind::Assign => {
                        crate::runtime::RendererRemoteWindowProxyNavigationKind::Assign
                    }
                    LocationNavigationKind::Replace | LocationNavigationKind::Reload => {
                        crate::runtime::RendererRemoteWindowProxyNavigationKind::Replace
                    }
                };
                let command = if let Some(javascript_source) = javascript_source {
                    crate::runtime::RendererRemoteWindowProxyCommand::navigate_javascript_url(
                        target.endpoint,
                        target.residence,
                        target.channel,
                        kind,
                        target_url.to_string(),
                        javascript_source,
                    )
                } else {
                    crate::runtime::RendererRemoteWindowProxyCommand::navigate(
                        target.endpoint,
                        target.residence,
                        target.channel,
                        kind,
                        target_url.to_string(),
                        source,
                    )
                };
                let _ = observer.append_remote_command(command);
                return true;
            }
            local @ ResolvedCrossOriginRelatedTopWindow::Local { .. } => Some(local),
        }
    } else {
        None
    };
    let (related_window, target_context, host_ptr) =
        if related_endpoint.is_some() || local_top_window {
            let Some(ResolvedCrossOriginRelatedTopWindow::Local {
                window_proxy,
                context,
                host_ptr,
            }) = resolved_top_target
            else {
                unreachable!("remote top-level navigation returned before local routing")
            };
            (
                Some(v8::Global::new(scope, window_proxy)),
                v8::Global::new(scope, context),
                host_ptr,
            )
        } else {
            let Some(context) = receiver.get_creation_context(scope) else {
                return false;
            };
            let Some(host_ptr) = context_host_ptr_from_context_slot(context) else {
                return false;
            };
            (None, v8::Global::new(scope, context), host_ptr)
        };
    let target_context = v8::Local::new(scope, &target_context);
    let navigation_target_scope = child_handle.map_or(
        super::super::OwnerDispatchScope::Top,
        super::super::OwnerDispatchScope::Child,
    );
    let current_href = if let Some(handle) = child_handle {
        unsafe { &*host_ptr }
            .document_url_for_child_context(handle)
            .to_string()
    } else {
        unsafe { &*host_ptr }.document_url().to_string()
    };
    // Resolve while the source/entered realm is still current. Blink's
    // Location setter uses the entered Window's API base URL, not the target
    // Location's Document base URL. Passing the absolute URL through to the
    // target context also keeps the policy decision and queued navigation on
    // the same destination.
    let source_base_url = observer.navigation_api_base_url(scope);
    let Some(target_url) = resolve_location_navigation_target_against_entered_base(
        &current_href,
        kind,
        Some(raw.clone()),
        source_base_url.as_ref(),
    ) else {
        return false;
    };
    if let Err(denial) =
        observer.can_navigate(scope, host_ptr, navigation_target_scope, &target_url)
    {
        if denial.is_sandbox_violation() {
            throw_cross_origin_location_security_error(scope);
            return false;
        }
        return true;
    }
    let target_scope = &mut v8::ContextScope::new(scope, target_context);
    if let Some(window) = related_window {
        let window = v8::Local::new(target_scope, &window);
        return navigate_top_level_window_location_from_cross_origin(
            target_scope,
            window,
            kind,
            target_url.to_string(),
        );
    }

    let handle = child_handle.expect("cross-origin child navigation must retain its handle");
    let target = target_url;
    match kind {
        LocationNavigationKind::Assign => {
            let same_document = cross_origin_location_target_is_same_document(
                unsafe { &*host_ptr },
                handle,
                &target,
            );
            let target_window = if same_document {
                unsafe { &mut *host_ptr }
                    .existing_child_browsing_context_window_wrapper(target_scope, handle)
            } else {
                None
            };
            if let Some(window) = target_window
                && !dispatch_cross_document_navigation_navigate_event_for_window(
                    target_scope,
                    window,
                    target.as_str(),
                    None,
                    false,
                    None,
                )
            {
                return false;
            }
            // The target NavigateEvent can synchronously execute native DOM
            // callbacks in this host. Reborrow only after author code returns.
            let _ = unsafe { &mut *host_ptr }.navigate_child_browsing_context_to_url(
                target_scope,
                handle,
                target.as_str(),
            );
        }
        LocationNavigationKind::Replace => {
            let _ = unsafe { &mut *host_ptr }
                .queue_child_browsing_context_navigation_from_existing_seed(
                    handle,
                    target.as_str(),
                    true,
                );
        }
        LocationNavigationKind::Reload => return false,
    }
    true
}

fn cross_origin_location_target_is_same_document(
    host: &JsContextHost,
    handle: DomHandle,
    target: &url::Url,
) -> bool {
    host.child_browsing_context_current_url(handle)
        .is_some_and(|current| {
            let mut current = current;
            current.set_fragment(None);
            let mut target = target.clone();
            target.set_fragment(None);
            current == target
        })
}

fn cross_origin_location_replace_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !is_cross_origin_location_proxy(scope, args.this()) {
        throw_cross_origin_illegal_invocation(scope);
        return;
    }
    let Some(parsed) = webidl::parse_args::<CrossOriginLocationReplaceArgs>(scope, &args) else {
        return;
    };
    let _ = cross_origin_location_navigate_raw(
        scope,
        args.this(),
        LocationNavigationKind::Replace,
        parsed.url,
    );
}

fn child_handle_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    get_cross_origin_proxy_private_value(scope, object, CHILD_BROWSING_CONTEXT_HANDLE_SLOT)
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
        .map(|value| DomHandle::new(value as usize))
}

fn private_bigint_u64<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<u64> {
    let value = get_cross_origin_proxy_private_value(scope, object, slot)?;
    let value = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (value, lossless) = value.u64_value();
    lossless.then_some(value)
}

fn cross_origin_related_top_window_endpoint<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<TopLevelWindowProxyEndpointId> {
    TopLevelWindowProxyEndpointId::from_wire_parts(
        private_bigint_u64(scope, object, CROSS_ORIGIN_RELATED_TOP_WINDOW_GROUP_SLOT)?,
        private_bigint_u64(
            scope,
            object,
            CROSS_ORIGIN_RELATED_TOP_WINDOW_GENERATION_SLOT,
        )?,
    )
}

fn cross_origin_remote_frame_token<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<crate::script_vm::RendererRemoteFrameToken> {
    let projection_id =
        private_bigint_u64(scope, object, CROSS_ORIGIN_REMOTE_FRAME_PROJECTION_SLOT)?;
    let observer = CrossOriginWindowObserver::resolve(scope)?;
    unsafe { &*observer.host_ptr }
        .page_script_environment
        .as_ref()?
        .remote_frame_token_for_projection_id(projection_id)
}

fn is_cross_origin_local_top_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    get_cross_origin_proxy_private_value(scope, object, CROSS_ORIGIN_LOCAL_TOP_WINDOW_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

enum ResolvedCrossOriginRelatedTopWindow<'s> {
    Local {
        window_proxy: v8::Local<'s, v8::Object>,
        context: v8::Local<'s, v8::Context>,
        host_ptr: *mut JsContextHost,
    },
    Remote(crate::script_vm::RendererRemoteTopLevelWindowProxyTarget),
}

fn resolve_cross_origin_related_top_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<ResolvedCrossOriginRelatedTopWindow<'s>> {
    let endpoint = cross_origin_related_top_window_endpoint(scope, object)?;
    let observer = CrossOriginWindowObserver::resolve(scope)?;
    match unsafe { &*observer.host_ptr }
        .related_page_target_for_window_proxy_endpoint(scope, endpoint)?
    {
        crate::script_vm::RendererRelatedTopLevelWindowProxyResolution::Local {
            window_proxy,
            context,
            ..
        } => {
            let host_ptr = context_host_ptr_from_context_slot(context)?;
            Some(ResolvedCrossOriginRelatedTopWindow::Local {
                window_proxy,
                context,
                host_ptr,
            })
        }
        crate::script_vm::RendererRelatedTopLevelWindowProxyResolution::Remote(target) => {
            Some(ResolvedCrossOriginRelatedTopWindow::Remote(target))
        }
    }
}

fn resolve_cross_origin_live_top_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<ResolvedCrossOriginRelatedTopWindow<'s>> {
    if cross_origin_related_top_window_endpoint(scope, object).is_some() {
        // Never fall back through an endpoint that became stale. In
        // particular, a disconnected COOP proxy cannot resolve against the
        // incumbent Page's creation Context.
        return resolve_cross_origin_related_top_window(scope, object);
    }
    if !is_cross_origin_local_top_window(scope, object) {
        return None;
    }
    let context = object.get_creation_context(scope)?;
    let host_ptr = context_host_ptr_from_context_slot(context)?;
    Some(ResolvedCrossOriginRelatedTopWindow::Local {
        window_proxy: context.global(scope),
        context,
        host_ptr,
    })
}

pub(crate) fn is_cross_origin_related_top_window_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    cross_origin_related_top_window_endpoint(scope, object).is_some()
}

pub(crate) fn is_cross_origin_remote_frame_window_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    cross_origin_remote_frame_token(scope, object).is_some()
}

pub(crate) fn cross_origin_window_target_host_ptr<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<*mut JsContextHost> {
    // This non-owning pointer is callback-scoped. The Context slot performs
    // the shared liveness check; callers must never cache the returned value.
    if cross_origin_related_top_window_endpoint(scope, object).is_some() {
        return match resolve_cross_origin_related_top_window(scope, object)? {
            ResolvedCrossOriginRelatedTopWindow::Local { host_ptr, .. } => Some(host_ptr),
            ResolvedCrossOriginRelatedTopWindow::Remote(_) => None,
        };
    }
    if cross_origin_remote_frame_token(scope, object).is_some() {
        return None;
    }
    let context = object.get_creation_context(scope)?;
    context_host_ptr_from_context_slot(context)
}

pub(crate) fn cross_origin_remote_top_window_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<crate::script_vm::RendererRemoteTopLevelWindowProxyTarget> {
    match resolve_cross_origin_related_top_window(scope, object)? {
        ResolvedCrossOriginRelatedTopWindow::Remote(target) => Some(target),
        ResolvedCrossOriginRelatedTopWindow::Local { .. } => None,
    }
}

pub(crate) fn cross_origin_remote_frame_window_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<(
    crate::script_vm::RendererRemoteFrameToken,
    crate::script_vm::RendererRemoteFrameSnapshot,
    crate::script_vm::RendererRemoteTopLevelWindowProxyTarget,
)> {
    let token = cross_origin_remote_frame_token(scope, object)?;
    let observer = CrossOriginWindowObserver::resolve(scope)?;
    let environment = unsafe { &*observer.host_ptr }
        .page_script_environment
        .as_ref()?;
    let frame = environment.remote_frame_snapshot(token)?;
    let top = environment.remote_top_level_target_snapshot(token.endpoint)?;
    Some((token, frame, top))
}

pub(crate) fn throw_cross_origin_type_error(scope: &mut v8::PinScope<'_, '_>, message: &str) {
    let accessing_context = {
        let context = cross_origin_accessing_context(scope);
        v8::Global::new(scope, context)
    };
    let accessing_context = v8::Local::new(scope, &accessing_context);
    if accessing_context == scope.get_current_context() {
        throw_type_error(scope, message);
        return;
    }
    let accessing_scope = &mut v8::ContextScope::new(scope, accessing_context);
    throw_type_error(accessing_scope, message);
}

fn throw_cross_origin_illegal_invocation(scope: &mut v8::PinScope<'_, '_>) {
    throw_cross_origin_type_error(
        scope,
        "Failed to execute cross-origin Window operation: Illegal invocation.",
    );
}

fn cross_origin_property_attributes() -> v8::PropertyAttribute {
    v8::PropertyAttribute::DONT_ENUM | v8::PropertyAttribute::READ_ONLY
}

fn cross_origin_index_property_attributes() -> v8::PropertyAttribute {
    v8::PropertyAttribute::READ_ONLY
}

fn cross_origin_named_property_attributes() -> v8::PropertyAttribute {
    v8::PropertyAttribute::DONT_ENUM | v8::PropertyAttribute::READ_ONLY
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_window_proxy_record_is_keyed_by_browsing_context_identity() {
        let handle = DomHandle::new(41);
        let first_id = BrowsingContextId::nested(7);
        let replacement_id = BrowsingContextId::nested(8);
        let mut records = ChildWindowProxyRecords::default();

        records.bind_nested_browsing_context(handle, first_id);
        records.record_mut(handle).window_proxy_exposed = true;
        assert!(records.records.contains_key(&first_id));

        records.bind_nested_browsing_context(handle, replacement_id);
        assert_eq!(records.context_id(handle), Some(replacement_id));
        assert!(records.records.contains_key(&replacement_id));
        assert!(!records.records.contains_key(&first_id));
        assert!(records.window_proxy_exposed(handle));
    }
}
