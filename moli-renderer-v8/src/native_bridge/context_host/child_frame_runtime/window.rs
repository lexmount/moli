use super::super::{JsContextHost, PendingWindowMessageEndpoint};
use crate::{
    context_bootstrap::{
        CHILD_BROWSING_CONTEXT_HANDLE_SLOT,
        dispatch_cross_document_navigation_navigate_event_for_window,
    },
    definitions::{define_function_accessor_property, define_get_set_property},
    document_runtime::DomHandle,
    document_script_scheduler::FrameDocumentClassicScriptSchedulerWork,
    native_bridge::{
        bridge::throw_dom_exception,
        child_window_surface::bind_materialized_child_window_indexed_db_factory,
        helpers::set_object_slot,
    },
    util::{
        context_host_ptr_from_global_bridge, get_private_value, new_null_prototype_object,
        serialize_v8_array, serialize_v8_iter_array, set_null_prototype, set_private_value,
        throw_type_error, v8_string, v8str,
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
    records: HashMap<DomHandle, ChildWindowProxyRecord>,
    detached_content_window_wrappers: HashMap<DomHandle, v8::Global<v8::Object>>,
    detached_content_document_wrappers: HashMap<DomHandle, v8::Global<v8::Object>>,
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
    live_window_exposed_to_top: bool,
    cross_origin_window_proxy: Option<v8::Global<v8::Object>>,
    cross_origin_access_surface: Option<v8::Global<v8::Object>>,
    default_execution_context_id: Option<i64>,
}

impl ChildWindowProxyRecords {
    fn record_mut(&mut self, handle: DomHandle) -> &mut ChildWindowProxyRecord {
        self.records.entry(handle).or_default()
    }

    pub(in crate::native_bridge::context_host) fn shell<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let record = self.records.get(&handle)?;
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
        self.records
            .get(&handle)?
            .live_window_wrapper
            .as_ref()
            .map(|window| v8::Local::new(scope, window))
    }

    pub(in crate::native_bridge::context_host) fn has_live_window(
        &self,
        handle: DomHandle,
    ) -> bool {
        self.records
            .get(&handle)
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
        self.records.get_mut(&handle)?.facade_context.take()
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
        self.records
            .get(&handle)?
            .browsing_context_parent_window
            .as_ref()
            .map(|parent| v8::Local::new(scope, parent))
    }

    pub(in crate::native_bridge::context_host) fn browsing_context_top<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.records
            .get(&handle)?
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
        self.records
            .get(&handle)?
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
        self.records
            .get(&handle)?
            .realm_top_window_wrapper
            .as_ref()
            .map(|top| v8::Local::new(scope, top))
    }

    pub(in crate::native_bridge::context_host) fn mark_live_window_exposed_to_top(
        &mut self,
        handle: DomHandle,
    ) {
        self.record_mut(handle).live_window_exposed_to_top = true;
    }

    pub(in crate::native_bridge::context_host) fn live_window_exposed_to_top(
        &self,
        handle: DomHandle,
    ) -> bool {
        self.records
            .get(&handle)
            .is_some_and(|record| record.live_window_exposed_to_top)
    }

    pub(in crate::native_bridge::context_host) fn cross_origin_proxy<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.records
            .get(&handle)?
            .cross_origin_window_proxy
            .as_ref()
            .map(|proxy| v8::Local::new(scope, proxy))
    }

    pub(in crate::native_bridge::context_host) fn has_cross_origin_proxy(
        &self,
        handle: DomHandle,
    ) -> bool {
        self.records
            .get(&handle)
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
        let record = self.records.get(&handle)?;
        let access_surface = record.cross_origin_access_surface.as_ref()?;
        let window_proxy = record.live_window_wrapper.as_ref()?;
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
        let Some(record) = self.records.get_mut(&handle) else {
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
        let Some(record) = self.records.get_mut(&handle) else {
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
        self.records.get(&handle)?.default_execution_context_id
    }

    pub(in crate::native_bridge::context_host) fn clear_live_records(&mut self, handle: DomHandle) {
        self.records.remove(&handle);
    }

    pub(in crate::native_bridge::context_host) fn retain_live_records(
        &mut self,
        live_handles: &HashSet<DomHandle>,
    ) {
        self.records
            .retain(|handle, _| live_handles.contains(handle));
    }

    pub(in crate::native_bridge::context_host) fn detached_content_document<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.detached_content_document_wrappers
            .get(&handle)
            .map(|wrapper| v8::Local::new(scope, wrapper))
    }

    pub(in crate::native_bridge::context_host) fn set_detached_content_document(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        document: v8::Local<'_, v8::Object>,
    ) {
        self.detached_content_document_wrappers
            .insert(handle, v8::Global::new(scope, document));
    }

    pub(in crate::native_bridge::context_host) fn detached_content_window<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.detached_content_window_wrappers
            .get(&handle)
            .map(|wrapper| v8::Local::new(scope, wrapper))
    }

    pub(in crate::native_bridge::context_host) fn set_detached_content_window(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        window: v8::Local<'_, v8::Object>,
    ) {
        self.detached_content_window_wrappers
            .insert(handle, v8::Global::new(scope, window));
    }

    pub(in crate::native_bridge::context_host) fn clear_detached_content_surfaces(
        &mut self,
        handle: DomHandle,
    ) {
        self.detached_content_window_wrappers.remove(&handle);
        self.detached_content_document_wrappers.remove(&handle);
    }
}

const CROSS_ORIGIN_DENIED_WINDOW_PROPERTIES: &[&str] = &[
    "customElements",
    "document",
    "external",
    "frameElement",
    "history",
    "indexedDB",
    "localStorage",
    "locationbar",
    "menubar",
    "name",
    "navigation",
    "sessionStorage",
    "navigator",
    "performance",
    "console",
    "screen",
    "visualViewport",
    "crypto",
    "caches",
    "clientInformation",
    "cookieStore",
    "credentialless",
    "crossOriginIsolated",
    "documentPictureInPicture",
    "fetch",
    "isSecureContext",
    "origin",
    "originAgentCluster",
    "scheduler",
    "speechSynthesis",
    "structuredClone",
    "trustedTypes",
    "queueMicrotask",
    "setTimeout",
    "clearTimeout",
    "setInterval",
    "clearInterval",
    "clearImmediate",
    "requestAnimationFrame",
    "cancelAnimationFrame",
    "requestIdleCallback",
    "cancelIdleCallback",
    "addEventListener",
    "removeEventListener",
    "dispatchEvent",
    "getComputedStyle",
    "getSelection",
    "matchMedia",
    "event",
    "onabort",
    "onafterprint",
    "onbeforeprint",
    "onbeforeunload",
    "onblur",
    "oncancel",
    "oncanplay",
    "oncanplaythrough",
    "onchange",
    "onclick",
    "onclose",
    "oncontextmenu",
    "ondblclick",
    "ondrag",
    "ondragend",
    "ondragenter",
    "ondragleave",
    "ondragover",
    "ondragstart",
    "ondrop",
    "ondurationchange",
    "onemptied",
    "onended",
    "onerror",
    "onfocus",
    "onhashchange",
    "oninput",
    "oninvalid",
    "onkeydown",
    "onkeypress",
    "onkeyup",
    "onload",
    "onloadeddata",
    "onloadedmetadata",
    "onloadstart",
    "onmessage",
    "onmousedown",
    "onmousemove",
    "onmouseout",
    "onmouseover",
    "onmouseup",
    "onmousewheel",
    "onoffline",
    "ononline",
    "onpagehide",
    "onpageshow",
    "onpause",
    "onplay",
    "onplaying",
    "onpopstate",
    "onprogress",
    "onratechange",
    "onreset",
    "onresize",
    "onscroll",
    "onseeked",
    "onseeking",
    "onselect",
    "onstalled",
    "onstorage",
    "onsubmit",
    "onsuspend",
    "ontimeupdate",
    "onunhandledrejection",
    "onunload",
    "onvolumechange",
    "onwaiting",
    "onrejectionhandled",
    "innerWidth",
    "innerHeight",
    "outerWidth",
    "outerHeight",
    "devicePixelRatio",
    "scrollX",
    "scrollY",
    "pageXOffset",
    "pageYOffset",
    "personalbar",
    "scrollbars",
    "statusbar",
    "status",
    "screenX",
    "screenY",
    "toolbar",
    "scroll",
    "scrollTo",
    "scrollBy",
    "moveBy",
    "moveTo",
    "resizeBy",
    "resizeTo",
    "open",
    "stop",
    "print",
    "find",
    "alert",
    "confirm",
    "prompt",
    "reportError",
    "btoa",
    "atob",
];

const CROSS_ORIGIN_LOCATION_DENIED_PROPERTIES: &[&str] = &[
    "ancestorOrigins",
    "assign",
    "hash",
    "host",
    "hostname",
    "origin",
    "pathname",
    "port",
    "protocol",
    "reload",
    "search",
    "toString",
];

const CROSS_ORIGIN_WINDOW_NOOP_METHODS: &[&str] = &["blur", "close", "focus"];

const CROSS_ORIGIN_WINDOW_LOCATION_SLOT: &str = "__moliCrossOriginWindowLocation";
const CROSS_ORIGIN_WINDOW_SELF_SLOT: &str = "__moliCrossOriginWindowSelf";
const CROSS_ORIGIN_WINDOW_PARENT_SLOT: &str = "__moliCrossOriginWindowParent";
const CROSS_ORIGIN_WINDOW_TOP_SLOT: &str = "__moliCrossOriginWindowTop";
const CROSS_ORIGIN_WINDOW_CLOSED_SLOT: &str = "__moliCrossOriginWindowClosed";
const CROSS_ORIGIN_WINDOW_OPENER_SLOT: &str = "__moliCrossOriginWindowOpener";
const CROSS_ORIGIN_LOCATION_PROXY_SLOT: &str = "__moliCrossOriginLocationProxy";
const CROSS_ORIGIN_LOCATION_PROXY_SELF_SLOT: &str = "__moliCrossOriginLocationProxySelf";
const DETACHED_CROSS_ORIGIN_WINDOW_PROXY_SLOT: &str = "__moliDetachedCrossOriginWindowProxy";
const CROSS_ORIGIN_TOP_WINDOW_PROXY_SLOT: &str = "__moliCrossOriginTopWindowProxy";
const CROSS_ORIGIN_LIGHTWEIGHT_POPUP_ID_SLOT: &str = "__moliCrossOriginLightweightPopupId";
const CROSS_ORIGIN_WINDOW_NAMED_CHILD_SLOTS: &str = "__moliCrossOriginWindowNamedChildSlots";

const CROSS_ORIGIN_ACCESS_ERROR: &str =
    "Blocked a frame with a different origin from accessing a cross-origin frame.";

pub(crate) fn install_child_window_proxy_access_check_handlers(
    global_template: v8::Local<'_, v8::ObjectTemplate>,
) {
    global_template.set_security_token_access_check_and_handlers(
        window_access_check_callback,
        v8::NamedPropertyHandlerConfiguration::new()
            .getter(child_window_cross_origin_named_getter)
            .setter(child_window_cross_origin_named_setter)
            .query(child_window_cross_origin_named_query)
            .enumerator(child_window_cross_origin_named_enumerator)
            .descriptor(child_window_cross_origin_named_descriptor),
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(child_window_cross_origin_indexed_getter)
            .setter(child_window_cross_origin_indexed_setter)
            .query(child_window_cross_origin_indexed_query)
            .enumerator(child_window_cross_origin_indexed_enumerator)
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
    let Some(accessed_context) = accessed_object.get_creation_context(scope) else {
        return false;
    };
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
    if accessing_host_ptr != accessed_host_ptr {
        return false;
    }

    let host = unsafe { &*accessing_host_ptr };
    let Some(accessing_identity) =
        host.window_execution_context_identity_for_access_check(accessing_context)
    else {
        return false;
    };
    if let Some(accessed_identity) =
        host.window_execution_context_identity_for_access_check(accessed_context)
        && host.window_execution_context_can_access(accessing_identity, accessed_identity)
    {
        return true;
    }
    if !host.window_execution_context_identity_is_current(accessing_identity)
        || accessed_context
            .get_slot::<super::super::RuntimeObservableContextToken>()
            .is_none()
    {
        return false;
    }

    // Removing a same-origin iframe retires its LocalWindow registration, but
    // JavaScript can still retain that inner global long enough to clean up
    // listeners and other realm-owned state. The context's internalized
    // security token is the last live effective-origin snapshot, so equal
    // tokens preserve that access without reopening retired cross-origin or
    // opaque realms.
    accessing_context
        .get_security_token(scope)
        .strict_equals(accessed_context.get_security_token(scope))
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
        readonly,
        dont_delete
    )]
    post_message: (),
    #[webapi(method, callback = cross_origin_window_noop_callback, readonly, dont_delete)]
    blur: (),
    #[webapi(method, callback = cross_origin_window_noop_callback, readonly, dont_delete)]
    close: (),
    #[webapi(method, callback = cross_origin_window_noop_callback, readonly, dont_delete)]
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

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct CrossOriginLocationMethodsDeclaration {
    #[webapi(
        method,
        length = 1,
        callback = cross_origin_location_replace_callback,
        readonly,
        dont_delete
    )]
    replace: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct CrossOriginLocationProxyHandlerDeclaration {
    #[webapi(method, length = 3, callback = cross_origin_location_proxy_get_callback)]
    get: (),
    #[webapi(method, length = 4, callback = cross_origin_location_proxy_set_callback)]
    set: (),
    #[webapi(method, length = 2, callback = cross_origin_window_denied_callback)]
    delete_property: (),
    #[webapi(method, length = 3, callback = cross_origin_window_denied_callback)]
    define_property: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct CrossOriginWindowProxyHandlerDeclaration {
    #[webapi(method, length = 2, callback = cross_origin_window_proxy_has_callback)]
    has: (),
    #[webapi(method, length = 2, callback = cross_origin_window_denied_callback)]
    delete_property: (),
    #[webapi(method, length = 3, callback = cross_origin_window_denied_callback)]
    define_property: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", scope_lifetime = 'scope)]
struct CrossOriginWindowLiveAccessorsDeclaration<'scope> {
    storage: v8::Local<'scope, v8::Value>,
    #[webapi(
        accessor_property,
        getter = cross_origin_window_self_getter_callback,
        data = self.storage
    )]
    window: (),
    #[webapi(
        accessor_property = "self",
        getter = cross_origin_window_self_getter_callback,
        data = self.storage
    )]
    self_value: (),
    #[webapi(
        accessor_property,
        getter = cross_origin_window_location_getter_callback,
        setter = cross_origin_location_navigate_setter_callback
    )]
    location: (),
    #[webapi(
        accessor_property,
        getter = cross_origin_window_closed_getter_callback,
        data = self.storage
    )]
    closed: (),
    #[webapi(
        accessor_property,
        getter = cross_origin_window_self_getter_callback,
        data = self.storage
    )]
    frames: (),
    #[webapi(
        accessor_property,
        getter = cross_origin_window_length_getter_callback
    )]
    length: (),
    #[webapi(
        accessor_property,
        getter = cross_origin_window_top_getter_callback,
        data = self.storage
    )]
    top: (),
    #[webapi(
        accessor_property,
        getter = cross_origin_window_opener_getter_callback,
        data = self.storage
    )]
    opener: (),
    #[webapi(
        accessor_property,
        getter = cross_origin_window_parent_getter_callback,
        data = self.storage
    )]
    parent: (),
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
        crate::util::install_context_host_pointer_slot(context, self as *mut Self);
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
        self.child_window_proxy_records
            .live_window_exposed_to_top(handle)
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
        let child_frame_count = self.child_browsing_context_child_frame_count(handle);
        let named_indices = self.child_browsing_context_child_frame_named_indices(handle);
        install_live_cross_origin_child_window_surface(
            scope,
            surface,
            handle,
            parent,
            top,
            window_proxy,
            child_frame_count,
            &named_indices,
        );
        self.child_window_proxy_records
            .set_cross_origin_access_surface(scope, handle, surface);
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
        if self
            .child_window_proxy_records
            .live_window_exposed_to_top(handle)
            && let Some(proxy) = self.ensure_top_exposed_cross_origin_window_proxy(scope, handle)
        {
            return Some(proxy);
        }
        self.child_browsing_context_cross_origin_window_proxy(scope, handle)
    }

    pub(crate) fn mark_child_browsing_context_window_wrapper_exposed_to_top(
        &mut self,
        handle: DomHandle,
    ) {
        self.child_window_proxy_records
            .mark_live_window_exposed_to_top(handle);
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
            let top =
                get_cross_origin_proxy_private_value(scope, proxy, CROSS_ORIGIN_WINDOW_TOP_SLOT)
                    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                    .unwrap_or_else(|| scope.get_current_context().global(scope));
            install_cross_origin_window_index_slots(scope, proxy, child_frame_count, proxy, top);
            let named_indices = self.child_browsing_context_child_frame_named_indices(handle);
            install_cross_origin_window_named_slots(scope, proxy, &named_indices, proxy, top);
            return Some(proxy);
        }
        if !self.child_browsing_contexts.contains_key(&handle) {
            return None;
        }

        let (proxy, facade_context) = self.bridge.bindings.instantiate_window_proxy_shell(scope);
        self.child_window_proxy_records
            .set_facade_context(handle, facade_context);
        set_null_prototype(scope, proxy);
        let global = scope.get_current_context().global(scope);
        let top = self.child_browsing_context_root_window(scope, handle, global);
        let parent = self.child_browsing_context_parent_window(scope, handle, top);
        self.child_window_proxy_records
            .set_browsing_context_parent_top(scope, handle, parent, top);
        let named_indices = self.child_browsing_context_child_frame_named_indices(handle);
        install_live_cross_origin_child_window_surface(
            scope,
            proxy,
            handle,
            parent,
            top,
            proxy,
            child_frame_count,
            &named_indices,
        );
        self.child_window_proxy_records
            .set_cross_origin_proxy(scope, handle, proxy);
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
        if let Some(window) =
            self.child_browsing_context_popup_owner_window_for_realm(scope, handle)
        {
            return window;
        }
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
        let popup_owner = self.child_browsing_context_popup_owner_window_for_realm(scope, handle);
        let top = if let Some(window) = popup_owner {
            window
        } else if same_origin_with_top {
            root
        } else {
            self.cross_origin_window_endpoint_projection_for_child(
                scope,
                handle,
                PendingWindowMessageEndpoint::TopWindow,
            )
            .unwrap_or(window)
        };
        let parent = if let Some(window) = popup_owner {
            window
        } else if same_origin_with_top {
            stable_parent
                .or(existing_parent)
                .unwrap_or_else(|| self.child_browsing_context_parent_window(scope, handle, top))
        } else {
            self.child_browsing_context_parent_window(scope, handle, top)
        };
        set_object_slot(scope, window, "__moliWindowParent", parent.into());
        set_object_slot(scope, window, "__moliWindowTop", top.into());
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

        if let Some(popup_owner) =
            self.child_browsing_context_popup_owner_window_for_realm(scope, handle)
        {
            return (popup_owner, popup_owner);
        }

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
        if let Some(window) =
            self.child_browsing_context_popup_owner_window_for_realm(scope, handle)
        {
            return window;
        }
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

    fn child_browsing_context_popup_owner_window_for_realm<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let popup_id = self.child_browsing_context_popup_owner_id(handle)?;
        if self.child_window_can_access_lightweight_popup(handle, popup_id) {
            return self.lightweight_popup_window(scope, popup_id);
        }
        self.cross_origin_window_endpoint_projection_for_child(
            scope,
            handle,
            PendingWindowMessageEndpoint::LightweightPopup(popup_id),
        )
    }

    fn cross_origin_window_endpoint_projection_for_child<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        accessing_handle: DomHandle,
        endpoint: PendingWindowMessageEndpoint,
    ) -> Option<v8::Local<'s, v8::Object>> {
        if let Some(projection) = self
            .child_window_proxy_records
            .cross_origin_endpoint_projection(scope, accessing_handle, endpoint)
        {
            return Some(projection);
        }
        match endpoint {
            PendingWindowMessageEndpoint::TopWindow => {
                let projection = build_cross_origin_top_window_proxy(scope);
                self.child_window_proxy_records
                    .set_cross_origin_endpoint_projection(
                        scope,
                        accessing_handle,
                        endpoint,
                        projection,
                    );
                set_cross_origin_window_backing_slot(
                    scope,
                    projection,
                    CROSS_ORIGIN_WINDOW_OPENER_SLOT,
                    v8::null(scope).into(),
                );
                Some(projection)
            }
            PendingWindowMessageEndpoint::ChildWindow(handle) => {
                self.child_browsing_context_cross_origin_window_proxy(scope, handle)
            }
            PendingWindowMessageEndpoint::LightweightPopup(popup_id) => {
                let projection = build_cross_origin_popup_window_proxy(scope, popup_id);
                self.child_window_proxy_records
                    .set_cross_origin_endpoint_projection(
                        scope,
                        accessing_handle,
                        endpoint,
                        projection,
                    );
                let opener = if let Some(opener_endpoint) =
                    self.lightweight_popup_opener_endpoint(popup_id)
                    && opener_endpoint != endpoint
                {
                    self.cross_origin_window_endpoint_projection_for_child(
                        scope,
                        accessing_handle,
                        opener_endpoint,
                    )
                    .map(v8::Local::<v8::Value>::from)
                    .unwrap_or_else(|| v8::null(scope).into())
                } else {
                    v8::null(scope).into()
                };
                set_cross_origin_window_backing_slot(
                    scope,
                    projection,
                    CROSS_ORIGIN_WINDOW_OPENER_SLOT,
                    opener,
                );
                Some(projection)
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
        if window.has_own_property(scope, key.into()).unwrap_or(false) {
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
    !name.is_empty()
        && name.parse::<u32>().is_err()
        && !CROSS_ORIGIN_DENIED_WINDOW_PROPERTIES.contains(&name)
        && !CROSS_ORIGIN_WINDOW_NOOP_METHODS.contains(&name)
        && !matches!(
            name,
            "window"
                | "self"
                | "globalThis"
                | "top"
                | "parent"
                | "frames"
                | "length"
                | "location"
                | "closed"
                | "opener"
                | "postMessage"
                | "then"
        )
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
        let _ = window.delete(scope, key.into());
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
    install_cross_origin_window_identity_slots(scope, window, handle, indexed_parent, parent, top);
    install_cross_origin_window_index_slots(scope, window, child_frame_count, indexed_parent, top);
    install_cross_origin_symbol_slots(scope, window, "Window");
    set_cross_origin_window_backing_slot(
        scope,
        window,
        CROSS_ORIGIN_WINDOW_CLOSED_SLOT,
        v8::Boolean::new(scope, false).into(),
    );
    let location = build_cross_origin_location_proxy(scope, handle);
    set_private_value(
        scope,
        window,
        CROSS_ORIGIN_WINDOW_LOCATION_SLOT,
        location.into(),
    );
    set_cross_origin_window_backing_slot(
        scope,
        window,
        CROSS_ORIGIN_WINDOW_OPENER_SLOT,
        v8::null(scope).into(),
    );
    install_cross_origin_window_accessors(scope, window);
    set_cross_origin_object_slot(scope, window, "then", v8::undefined(scope).into());
    install_cross_origin_window_methods(scope, window);
    install_cross_origin_denied_accessors(scope, window, CROSS_ORIGIN_DENIED_WINDOW_PROPERTIES);
    install_cross_origin_window_named_slots(scope, window, named_indices, indexed_parent, top);
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
    set_cross_origin_window_backing_slot(
        scope,
        window,
        CROSS_ORIGIN_WINDOW_PARENT_SLOT,
        parent.into(),
    );
    set_cross_origin_window_backing_slot(scope, window, CROSS_ORIGIN_WINDOW_TOP_SLOT, top.into());
    set_cross_origin_window_backing_slot(
        scope,
        window,
        CROSS_ORIGIN_WINDOW_CLOSED_SLOT,
        v8::Boolean::new(scope, false).into(),
    );
    set_cross_origin_window_backing_slot(
        scope,
        window,
        CROSS_ORIGIN_WINDOW_OPENER_SLOT,
        v8::null(scope).into(),
    );
    set_cross_origin_object_slot(scope, window, "then", v8::undefined(scope).into());
    let location = build_detached_cross_origin_location_proxy(scope);
    set_private_value(
        scope,
        window,
        CROSS_ORIGIN_WINDOW_LOCATION_SLOT,
        location.into(),
    );
    install_cross_origin_window_accessors(scope, window);
    install_cross_origin_window_methods(scope, window);
    install_cross_origin_denied_accessors(scope, window, CROSS_ORIGIN_DENIED_WINDOW_PROPERTIES);
    install_cross_origin_symbol_slots(scope, window, "Window");
    let Some(proxy) = wrap_cross_origin_window_with_has_trap(scope, window) else {
        return window;
    };
    set_cross_origin_window_backing_slot(
        scope,
        window,
        CROSS_ORIGIN_WINDOW_SELF_SLOT,
        proxy.into(),
    );
    proxy
}

fn build_cross_origin_top_window_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    build_cross_origin_top_level_window_proxy(scope, PendingWindowMessageEndpoint::TopWindow)
}

fn build_cross_origin_popup_window_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    popup_id: u64,
) -> v8::Local<'s, v8::Object> {
    build_cross_origin_top_level_window_proxy(
        scope,
        PendingWindowMessageEndpoint::LightweightPopup(popup_id),
    )
}

fn build_cross_origin_top_level_window_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    endpoint: PendingWindowMessageEndpoint,
) -> v8::Local<'s, v8::Object> {
    let window = new_null_prototype_object(scope);
    match endpoint {
        PendingWindowMessageEndpoint::TopWindow => set_private_value(
            scope,
            window,
            window_host::TOP_WINDOW_MESSAGE_ENDPOINT_SLOT,
            v8::Boolean::new(scope, true).into(),
        ),
        PendingWindowMessageEndpoint::LightweightPopup(popup_id) => set_private_value(
            scope,
            window,
            CROSS_ORIGIN_LIGHTWEIGHT_POPUP_ID_SLOT,
            v8::BigInt::new_from_u64(scope, popup_id).into(),
        ),
        PendingWindowMessageEndpoint::ChildWindow(_) => {
            unreachable!("top-level Window projection cannot target a child frame")
        }
    }
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
    set_cross_origin_window_backing_slot(
        scope,
        window,
        CROSS_ORIGIN_WINDOW_CLOSED_SLOT,
        v8::Boolean::new(scope, false).into(),
    );
    set_cross_origin_window_backing_slot(
        scope,
        window,
        CROSS_ORIGIN_WINDOW_OPENER_SLOT,
        v8::null(scope).into(),
    );
    set_cross_origin_object_slot(scope, window, "then", v8::undefined(scope).into());
    let location = build_detached_cross_origin_location_proxy(scope);
    set_private_value(
        scope,
        window,
        CROSS_ORIGIN_WINDOW_LOCATION_SLOT,
        location.into(),
    );
    install_cross_origin_window_accessors(scope, window);
    install_cross_origin_window_methods(scope, window);
    install_cross_origin_denied_accessors(scope, window, CROSS_ORIGIN_DENIED_WINDOW_PROPERTIES);
    install_cross_origin_symbol_slots(scope, window, "Window");
    let Some(proxy) = wrap_cross_origin_window_with_has_trap(scope, window) else {
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

fn set_cross_origin_window_backing_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
    value: v8::Local<'s, v8::Value>,
) {
    let storage = cross_origin_proxy_storage_object(scope, object);
    set_private_value(scope, storage, slot, value);
}

fn install_cross_origin_window_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    CrossOriginWindowLiveAccessorsDeclaration {
        storage: object.into(),
        window: (),
        self_value: (),
        location: (),
        closed: (),
        frames: (),
        length: (),
        top: (),
        opener: (),
        parent: (),
    }
    .initialize(scope, object)
    .expect("cross-origin Window accessors declaration should initialize");
}

fn set_cross_origin_window_self_identity_slots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    identity: v8::Local<'s, v8::Object>,
) {
    for slot in [
        CROSS_ORIGIN_WINDOW_SELF_SLOT,
        CROSS_ORIGIN_WINDOW_PARENT_SLOT,
        CROSS_ORIGIN_WINDOW_TOP_SLOT,
    ] {
        set_cross_origin_window_backing_slot(scope, window, slot, identity.into());
    }
}

fn wrap_cross_origin_window_with_has_trap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let handler = CrossOriginWindowProxyHandlerDeclaration {
        has: (),
        delete_property: (),
        define_property: (),
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

fn install_cross_origin_window_identity_slots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    handle: DomHandle,
    identity: v8::Local<'s, v8::Object>,
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
    set_cross_origin_window_backing_slot(
        scope,
        window,
        CROSS_ORIGIN_WINDOW_SELF_SLOT,
        identity.into(),
    );
    set_cross_origin_window_backing_slot(
        scope,
        window,
        CROSS_ORIGIN_WINDOW_PARENT_SLOT,
        parent.into(),
    );
    set_cross_origin_window_backing_slot(scope, window, CROSS_ORIGIN_WINDOW_TOP_SLOT, top.into());
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
}

fn install_cross_origin_denied_accessors(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    names: &[&'static str],
) {
    for name in names {
        let _ = define_function_accessor_property(
            scope,
            object,
            name,
            cross_origin_window_denied_callback,
            None,
            cross_origin_window_denied_callback,
            None,
            cross_origin_property_attributes(),
        );
    }
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
    for name in CROSS_ORIGIN_LOCATION_DENIED_PROPERTIES {
        let _ = define_function_accessor_property(
            scope,
            target,
            name,
            cross_origin_window_denied_callback,
            None,
            cross_origin_window_denied_callback,
            None,
            cross_origin_property_attributes(),
        );
    }
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
    for name in CROSS_ORIGIN_LOCATION_DENIED_PROPERTIES {
        let _ = define_function_accessor_property(
            scope,
            target,
            name,
            cross_origin_window_denied_callback,
            None,
            cross_origin_window_denied_callback,
            None,
            cross_origin_property_attributes(),
        );
    }
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
        set: (),
        delete_property: (),
        define_property: (),
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
    to_string_tag: &'static str,
) {
    let tag_value = v8str(scope, to_string_tag).into();
    let _ = object.define_own_property(
        scope,
        v8::Symbol::get_to_string_tag(scope).into(),
        tag_value,
        cross_origin_property_attributes(),
    );
    let undefined = v8::undefined(scope).into();
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

fn child_window_cross_origin_handler_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<'s, v8::Object>,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>)> {
    let holder_context = holder.get_creation_context(scope)?;
    let host_ptr = crate::util::context_host_ptr_from_context_slot(holder_context)?;
    let host = unsafe { &*host_ptr };
    if let Some(identity) = host.window_execution_context_identity_for_access_check(holder_context)
        && host.window_execution_context_identity_is_current(identity)
    {
        let super::super::OwnerDispatchScope::Child(handle) = identity.dispatch_scope() else {
            return None;
        };
        if !host.window_execution_context_identity_is_default_world(identity) {
            return None;
        }
        return host
            .child_window_proxy_records
            .cross_origin_handler_data(scope, handle);
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

fn child_window_cross_origin_named_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let holder = args.holder();
    let Some(surface) = child_window_cross_origin_access_surface(scope, holder) else {
        return v8::Intercepted::kNo;
    };
    if let Some(value) = child_window_cross_origin_named_child_value(scope, surface, key) {
        let Some(value) = value else {
            return v8::Intercepted::kNo;
        };
        rv.set(value);
        return v8::Intercepted::kYes;
    }
    if surface.has_own_property(scope, key).unwrap_or(false)
        && let Some(value) = surface.get(scope, key.into())
    {
        rv.set(value);
        return v8::Intercepted::kYes;
    }
    v8::Intercepted::kNo
}

fn child_window_cross_origin_named_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    value: v8::Local<'s, v8::Value>,
    args: v8::PropertyCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    let Some(surface) = child_window_cross_origin_access_surface(scope, args.holder()) else {
        return v8::Intercepted::kNo;
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
    v8::Intercepted::kNo
}

fn child_window_cross_origin_named_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    let Some(surface) = child_window_cross_origin_access_surface(scope, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    if let Some(value) = child_window_cross_origin_named_child_value(scope, surface, key) {
        if value.is_none() {
            return v8::Intercepted::kNo;
        }
        rv.set_int32(cross_origin_named_property_attributes().as_u32() as i32);
        return v8::Intercepted::kYes;
    }
    if surface.has_own_property(scope, key).unwrap_or(false) {
        rv.set_int32(cross_origin_property_attributes().as_u32() as i32);
        return v8::Intercepted::kYes;
    }
    v8::Intercepted::kNo
}

fn child_window_cross_origin_named_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback_args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let mut property_names = v8::GetPropertyNamesArgsBuilder::new();
    property_names.property_filter(v8::PropertyFilter::ALL_PROPERTIES);
    property_names.key_conversion(v8::KeyConversionMode::ConvertToString);
    let names = child_window_cross_origin_access_surface(scope, callback_args.holder())
        .and_then(|surface| surface.get_own_property_names(scope, property_names.build()))
        .unwrap_or_else(|| v8::Array::new(scope, 0));
    rv.set(names);
}

fn child_window_cross_origin_named_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(surface) = child_window_cross_origin_access_surface(scope, args.holder()) else {
        return v8::Intercepted::kNo;
    };
    if let Some(value) = child_window_cross_origin_named_child_value(scope, surface, key) {
        let Some(value) = value else {
            return v8::Intercepted::kNo;
        };
        let Ok(descriptor) =
            CrossOriginPropertyDescriptorDeclaration::new(value, false, false, true).bind(scope)
        else {
            return v8::Intercepted::kNo;
        };
        rv.set(descriptor.into());
        return v8::Intercepted::kYes;
    }
    let Some(descriptor) = surface.get_own_property_descriptor(scope, key) else {
        return v8::Intercepted::kNo;
    };
    rv.set(descriptor);
    v8::Intercepted::kYes
}

fn child_window_cross_origin_named_child_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    surface: v8::Local<'s, v8::Object>,
    key: v8::Local<'s, v8::Name>,
) -> Option<Option<v8::Local<'s, v8::Value>>> {
    let key = v8::Local::<v8::String>::try_from(key).ok()?;
    let key_name = key.to_rust_string_lossy(scope);
    if !is_cross_origin_named_child_slot_name(&key_name) {
        return None;
    }
    let parent_handle = child_handle_from_object(scope, surface)?;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let child_handle = unsafe { &*host_ptr }
        .child_browsing_context_named_child_handle(Some(parent_handle), &key_name);
    Some(child_handle.and_then(|child_handle| {
        unsafe { &mut *host_ptr }
            .child_browsing_context_window_proxy_for_top(scope, child_handle)
            .map(Into::into)
    }))
}

fn child_window_cross_origin_indexed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(value) = child_window_cross_origin_indexed_value(scope, args.holder(), index) else {
        rv.set_undefined();
        return v8::Intercepted::kYes;
    };
    rv.set(value);
    v8::Intercepted::kYes
}

fn child_window_cross_origin_indexed_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<'s, v8::Object>,
    index: u32,
) -> Option<v8::Local<'s, v8::Value>> {
    let surface = child_window_cross_origin_access_surface(scope, holder)?;
    let Some(parent_handle) = child_handle_from_object(scope, surface) else {
        return surface.get_index(scope, index);
    };
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let host = unsafe { &mut *host_ptr };
    let child_handle =
        host.child_browsing_context_child_frame_handle_by_index(parent_handle, index as usize)?;
    host.child_browsing_context_window_proxy_for_top(scope, child_handle)
        .map(Into::into)
}

fn child_window_cross_origin_indexed_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _index: u32,
    _value: v8::Local<'s, v8::Value>,
    _args: v8::PropertyCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, ()>,
) -> v8::Intercepted {
    let _ = scope;
    v8::Intercepted::kNo
}

fn child_window_cross_origin_indexed_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    if child_window_cross_origin_indexed_value(scope, args.holder(), index).is_none() {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(cross_origin_index_property_attributes().as_u32() as i32);
    v8::Intercepted::kYes
}

fn child_window_cross_origin_indexed_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let count = child_window_cross_origin_access_surface(scope, args.holder())
        .and_then(|surface| child_handle_from_object(scope, surface))
        .and_then(|handle| {
            context_host_ptr_from_global_bridge(scope).map(|host_ptr| {
                unsafe { &mut *host_ptr }.child_browsing_context_child_frame_count(handle)
            })
        })
        .unwrap_or(0);
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
    let Some(value) = child_window_cross_origin_indexed_value(scope, args.holder(), index) else {
        return v8::Intercepted::kNo;
    };
    let Ok(descriptor) =
        CrossOriginPropertyDescriptorDeclaration::new(value, false, true, false).bind(scope)
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

fn cross_origin_window_proxy_has_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(target) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_cross_origin_location_security_error(scope);
        return;
    };
    if target.has(scope, args.get(1)).unwrap_or(false) {
        rv.set(v8::Boolean::new(scope, true).into());
        return;
    }
    throw_cross_origin_location_security_error(scope);
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

pub(crate) fn cross_origin_lightweight_popup_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    let value = get_cross_origin_proxy_private_value(
        scope,
        object,
        CROSS_ORIGIN_LIGHTWEIGHT_POPUP_ID_SLOT,
    )?;
    let value = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (popup_id, lossless) = value.u64_value();
    lossless.then_some(popup_id)
}

fn cross_origin_accessing_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Context> {
    // Our access surface lives in the target realm. For cross-origin function
    // callbacks V8 exposes the initiating realm as the incumbent context;
    // direct access-check failures can still fall back to the target realm
    // until the optional per-accessing-realm membrane milestone.
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
    let receiver = v8::Local::<v8::Object>::try_from(args.get(2)).unwrap_or(target);
    match target.get_with_receiver(scope, args.get(1), receiver) {
        Some(value) => rv.set(value),
        None => rv.set_undefined(),
    }
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
        let receiver = v8::Local::<v8::Object>::try_from(args.get(3)).unwrap_or(target);
        rv.set(
            v8::Boolean::new(
                scope,
                target
                    .set_with_receiver(scope, args.get(1), args.get(2), receiver)
                    .unwrap_or(false),
            )
            .into(),
        );
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

fn cross_origin_window_noop_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if child_handle_from_object(scope, args.this()).is_none()
        && get_cross_origin_proxy_private_value(
            scope,
            args.this(),
            DETACHED_CROSS_ORIGIN_WINDOW_PROXY_SLOT,
        )
        .is_none()
    {
        throw_cross_origin_illegal_invocation(scope);
    }
}

fn cross_origin_window_backing_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let storage = v8::Local::<v8::Object>::try_from(data).ok()?;
    get_private_value(scope, storage, slot)
}

fn cross_origin_window_stored_value_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    slot: &str,
) {
    if let Some(value) = cross_origin_window_backing_value(scope, args.data(), slot) {
        rv.set(value);
    } else {
        throw_cross_origin_illegal_invocation(scope);
    }
}

fn cross_origin_window_self_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    cross_origin_window_stored_value_getter(scope, args, rv, CROSS_ORIGIN_WINDOW_SELF_SLOT);
}

fn cross_origin_window_closed_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    cross_origin_window_stored_value_getter(scope, args, rv, CROSS_ORIGIN_WINDOW_CLOSED_SLOT);
}

fn cross_origin_window_has_discarded_child_browsing_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> bool {
    child_handle_from_object(scope, window).is_some_and(|handle| {
        context_host_ptr_from_global_bridge(scope)
            .is_none_or(|host_ptr| !unsafe { &*host_ptr }.child_browsing_context_is_live(handle))
    })
}

fn cross_origin_window_top_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if cross_origin_window_has_discarded_child_browsing_context(scope, args.this()) {
        rv.set_null();
        return;
    }
    cross_origin_window_stored_value_getter(scope, args, rv, CROSS_ORIGIN_WINDOW_TOP_SLOT);
}

fn cross_origin_window_opener_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    cross_origin_window_stored_value_getter(scope, args, rv, CROSS_ORIGIN_WINDOW_OPENER_SLOT);
}

fn cross_origin_window_parent_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if cross_origin_window_has_discarded_child_browsing_context(scope, args.this()) {
        rv.set_null();
        return;
    }
    cross_origin_window_stored_value_getter(scope, args, rv, CROSS_ORIGIN_WINDOW_PARENT_SLOT);
}

fn cross_origin_window_length_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
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
        if get_cross_origin_proxy_private_value(
            scope,
            args.this(),
            DETACHED_CROSS_ORIGIN_WINDOW_PROXY_SLOT,
        )
        .is_some()
        {
            rv.set_int32(0);
            return;
        }
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

fn cross_origin_location_navigate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) -> bool {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let Some(handle) = child_handle_from_object(scope, receiver) else {
        throw_cross_origin_illegal_invocation(scope);
        return false;
    };
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
    let host = unsafe { &mut *host_ptr };
    let target = host.resolve_child_browsing_context_url(handle, &raw);
    if cross_origin_location_target_is_same_document(host, handle, &target)
        && let Some(window) = host.existing_child_browsing_context_window_wrapper(scope, handle)
        && !dispatch_cross_document_navigation_navigate_event_for_window(
            scope,
            window,
            target.as_str(),
            None,
            false,
            None,
        )
    {
        return false;
    }
    let _ = host.navigate_child_browsing_context_to_url(scope, handle, target.as_str());
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
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let Some(handle) = child_handle_from_object(scope, args.this()) else {
        throw_cross_origin_illegal_invocation(scope);
        return;
    };
    let Some(parsed) = webidl::parse_args::<CrossOriginLocationReplaceArgs>(scope, &args) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let target = host.resolve_child_browsing_context_url(handle, &parsed.url);
    let _ = host.queue_child_browsing_context_navigation_from_existing_seed(
        handle,
        target.as_str(),
        true,
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
    v8::PropertyAttribute::DONT_ENUM
        | v8::PropertyAttribute::DONT_DELETE
        | v8::PropertyAttribute::READ_ONLY
}

fn cross_origin_index_property_attributes() -> v8::PropertyAttribute {
    v8::PropertyAttribute::DONT_DELETE | v8::PropertyAttribute::READ_ONLY
}

fn cross_origin_named_property_attributes() -> v8::PropertyAttribute {
    v8::PropertyAttribute::DONT_ENUM | v8::PropertyAttribute::READ_ONLY
}
