use super::super::{JsContextHost, PendingWindowMessageEndpoint};
use crate::{
    context_bootstrap::{
        CHILD_BROWSING_CONTEXT_HANDLE_SLOT,
        dispatch_cross_document_navigation_navigate_event_for_window,
    },
    definitions::define_function_accessor_property,
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
use moli_webapi_declare::{WebApiCrossOriginDeclaration, WebApiObject};
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

mod cross_origin_bindings;
mod location;
use cross_origin_bindings::*;

// The same DOM handle can acquire a new browsing context after reinsertion.
// Keep the creating frame identity with the facade's navigation authority.
#[derive(Clone)]
struct ChildWindowProxyFacadeContextHandle(DomHandle, String);

#[derive(Default)]
pub(in crate::native_bridge::context_host) struct ChildWindowProxyRecords {
    records: HashMap<DomHandle, ChildWindowProxyRecord>,
    // Live Windows are rooted by `records`; detached projections are traced
    // through the iframe wrapper's private slots. These identity lookups must
    // not keep a retired child realm alive after its last author reference.
    detached_content_window_wrappers: HashMap<DomHandle, v8::Weak<v8::Object>>,
    detached_content_document_wrappers: HashMap<DomHandle, v8::Weak<v8::Object>>,
}

#[derive(Default)]
struct ChildWindowProxyRecord {
    live_window_wrapper: Option<v8::Global<v8::Object>>,
    facade_context: Option<v8::Global<v8::Context>>,
    browsing_context_parent_window: Option<v8::Global<v8::Object>>,
    browsing_context_top_window: Option<v8::Global<v8::Object>>,
    browsing_context_opener: Option<ChildWindowProxyOpener>,
    live_window_exposed_to_top: bool,
    cross_origin_window_proxy: Option<v8::Global<v8::Object>>,
    default_execution_context_id: Option<i64>,
}

struct ChildWindowProxyOpener {
    endpoint: PendingWindowMessageEndpoint,
    window: v8::Global<v8::Object>,
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
    }

    pub(in crate::native_bridge::context_host) fn take_facade_context(
        &mut self,
        handle: DomHandle,
    ) -> Option<v8::Global<v8::Context>> {
        self.records.get_mut(&handle)?.facade_context.take()
    }

    fn clear_retired_realm_projections(&mut self, handle: DomHandle) {
        let Some(record) = self.records.get_mut(&handle) else {
            return;
        };
        record.cross_origin_endpoint_projections.clear();
        record.realm_top_window_wrapper = None;
    }

    pub(in crate::native_bridge::context_host) fn promote_shell_to_live(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        window: v8::Local<'_, v8::Object>,
    ) {
        let record = self.record_mut(handle);
        record.cross_origin_window_proxy = None;
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

    pub(in crate::native_bridge::context_host) fn set_browsing_context_opener(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        endpoint: PendingWindowMessageEndpoint,
        window: v8::Local<'_, v8::Object>,
    ) {
        self.record_mut(handle).browsing_context_opener = Some(ChildWindowProxyOpener {
            endpoint,
            window: v8::Global::new(scope, window),
        });
    }

    pub(in crate::native_bridge::context_host) fn clear_browsing_context_opener(
        &mut self,
        handle: DomHandle,
    ) {
        if let Some(record) = self.records.get_mut(&handle) {
            record.browsing_context_opener = None;
        }
    }

    pub(in crate::native_bridge::context_host) fn browsing_context_opener<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_, ()>,
        handle: DomHandle,
    ) -> Option<(PendingWindowMessageEndpoint, v8::Local<'s, v8::Object>)> {
        let opener = self
            .records
            .get(&handle)?
            .browsing_context_opener
            .as_ref()?;
        Some((opener.endpoint, v8::Local::new(scope, &opener.window)))
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

    pub(in crate::native_bridge::context_host) fn set_cross_origin_proxy(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        proxy: v8::Local<'_, v8::Object>,
    ) {
        self.record_mut(handle).cross_origin_window_proxy = Some(v8::Global::new(scope, proxy));
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

    pub(in crate::native_bridge::context_host) fn prune_collected_detached_content_surfaces(
        &mut self,
    ) {
        for cache in [
            &mut self.detached_content_window_wrappers,
            &mut self.detached_content_document_wrappers,
        ] {
            // Run after V8 returns, without a Host borrow spanning GC. Empty
            // Weak handles still own native control blocks until they drop.
            cache.retain(|_, wrapper| !wrapper.is_empty());
            if cache.is_empty() {
                *cache = HashMap::new();
            } else if cache.len() <= cache.capacity() / 4 {
                // Keep author-reachable identities; only rehash sparse maps.
                cache.shrink_to_fit();
            }
        }
    }

    #[cfg(test)]
    pub(in crate::native_bridge::context_host) fn detached_content_surface_cache_counts_for_test(
        &self,
    ) -> [usize; 6] {
        // Window entries/dead targets, Document entries/dead targets, then map capacities.
        // is_empty() neither creates V8 locals nor makes an author-unreachable target live.
        [
            self.detached_content_window_wrappers.len(),
            self.detached_content_window_wrappers
                .values()
                .filter(|wrapper| wrapper.is_empty())
                .count(),
            self.detached_content_document_wrappers.len(),
            self.detached_content_document_wrappers
                .values()
                .filter(|wrapper| wrapper.is_empty())
                .count(),
            self.detached_content_window_wrappers.capacity(),
            self.detached_content_document_wrappers.capacity(),
        ]
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

const CROSS_ORIGIN_WINDOW_NOOP_METHODS: &[&str] = &["blur", "close", "focus"];

const CROSS_ORIGIN_WINDOW_LOCATION_SLOT: &str = "__moliCrossOriginWindowLocation";
const CROSS_ORIGIN_LOCATION_PROXY_SLOT: &str = "__moliCrossOriginLocationProxy";
const CROSS_ORIGIN_LOCATION_PROXY_SELF_SLOT: &str = "__moliCrossOriginLocationProxySelf";
const DETACHED_CROSS_ORIGIN_WINDOW_PROXY_SLOT: &str = "__moliDetachedCrossOriginWindowProxy";
const CROSS_ORIGIN_TOP_WINDOW_PROXY_SLOT: &str = "__moliCrossOriginTopWindowProxy";
const CROSS_ORIGIN_LIGHTWEIGHT_POPUP_ID_SLOT: &str = "__moliCrossOriginLightweightPopupId";
const CROSS_ORIGIN_WINDOW_NAMED_CHILD_SLOTS: &str = "__moliCrossOriginWindowNamedChildSlots";

const SYNTHETIC_WINDOW_ACCESS_INITIALIZED_SLOT: &str = "__moliSyntheticWindowAccessInitialized";

const CROSS_ORIGIN_ACCESS_ERROR: &str =
    "Blocked a frame with a different origin from accessing a cross-origin frame.";

pub(crate) fn install_child_window_proxy_access_check_handlers(
    global_template: v8::Local<'_, v8::ObjectTemplate>,
) {
    install_window_access_check_handlers(global_template, window_access_check_callback);
}

pub(crate) fn mark_popup_window_access_check_initialized(
    scope: &mut v8::PinScope<'_, '_>,
    window: v8::Local<'_, v8::Object>,
) {
    set_private_value(scope, window, SYNTHETIC_WINDOW_ACCESS_INITIALIZED_SLOT, v8::Boolean::new(scope, true).into());
}

pub(crate) fn install_synthetic_window_access_check_handlers(
    template: v8::Local<'_, v8::ObjectTemplate>,
) {
    install_window_access_check_handlers(template, synthetic_window_access_check_callback);
}

fn install_window_access_check_handlers(
    global_template: v8::Local<'_, v8::ObjectTemplate>,
    access_check: v8::AccessCheckCallback,
) {
    global_template.set_security_token_access_check_and_handlers(
        access_check,
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

unsafe extern "C" fn synthetic_window_access_check_callback(
    accessing_context: v8::Local<'_, v8::Context>,
    accessed_object: v8::Local<'_, v8::Object>,
    _data: v8::Local<'_, v8::Value>,
) -> bool {
    let scope = std::pin::pin!(unsafe { v8::CallbackScope::new(accessing_context) });
    let scope = &mut scope.init();
    // A synthetic shell is initialized before it is exposed to script. Its
    // private access surface marks the end of that native initialization.
    if get_private_value(
        scope,
        accessed_object,
        SYNTHETIC_WINDOW_ACCESS_INITIALIZED_SLOT,
    )
    .is_none()
    {
        return true;
    }
    synthetic_window_context_can_access(scope, accessing_context, accessed_object)
}

pub(crate) fn synthetic_window_context_can_access<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    accessing_context: v8::Local<'s, v8::Context>,
    window: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(popup_id) = crate::native_bridge::lightweight_popup_id_from_window(scope, window)
    else {
        return true;
    };
    let Ok(host_ptr) = crate::native_bridge::runtime_ptr_from_object(scope, window) else {
        return false;
    };
    if crate::util::context_host_ptr_from_context_slot(accessing_context) != Some(host_ptr) {
        return false;
    }
    let host = unsafe { &*host_ptr };
    host.window_security_origin_for_context(scope, accessing_context)
        .zip(host.lightweight_popup_security_origin(popup_id))
        .is_some_and(|(source, target)| source.can_access(&target))
}

unsafe extern "C" fn window_access_check_callback(
    accessing_context: v8::Local<'_, v8::Context>,
    accessed_object: v8::Local<'_, v8::Object>,
    _data: v8::Local<'_, v8::Value>,
) -> bool {
    let scope = std::pin::pin!(unsafe { v8::CallbackScope::new(accessing_context) });
    let scope = &mut scope.init();
    let Some(accessed_context) = accessed_object.get_creation_context(scope) else {
        return false;
    };
    contexts_can_script_access(scope, accessing_context, accessed_context)
}

fn contexts_can_script_access<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    accessing_context: v8::Local<'s, v8::Context>,
    accessed_context: v8::Local<'s, v8::Context>,
) -> bool {
    if accessing_context == accessed_context
        || accessing_context
            .get_security_token(scope)
            .strict_equals(accessed_context.get_security_token(scope))
    {
        return true;
    }

    super::super::window_contexts_allow_access(accessing_context, accessed_context)
}

pub(crate) fn caller_can_access_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> bool {
    if crate::native_bridge::lightweight_popup_id_from_window(scope, window).is_some() {
        return synthetic_window_context_can_access(scope, cross_origin_accessing_context(scope), window);
    }
    let Some(accessed) = window.get_creation_context(scope) else {
        return false;
    };
    let accessing = cross_origin_accessing_context(scope);
    contexts_can_script_access(scope, accessing, accessed)
}

impl JsContextHost {
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
#[webapi(plain)]
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
    #[webapi(method, callback = cross_origin_window_close_callback, readonly, dont_delete)]
    close: (),
    #[webapi(method, callback = cross_origin_window_noop_callback, readonly, dont_delete)]
    focus: (),
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct CrossOriginPropertyDescriptorDeclaration<'scope> {
    value: v8::Local<'scope, v8::Value>,
    writable: bool,
    enumerable: bool,
    configurable: bool,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct CrossOriginLocationProxyHandlerDeclaration {
    #[webapi(method, length = 3, callback = cross_origin_location_proxy_get_callback)]
    get: (),
    #[webapi(method, length = 4, callback = cross_origin_location_proxy_set_callback)]
    set: (),
    #[webapi(method, length = 2, callback = cross_origin_location_proxy_descriptor_callback)]
    get_own_property_descriptor: (),
    #[webapi(method, length = 1, callback = cross_origin_location_proxy_own_keys_callback)]
    own_keys: (),
    #[webapi(method, length = 2, callback = cross_origin_location_proxy_has_callback)]
    has: (),
    #[webapi(method, length = 2, callback = cross_origin_window_denied_callback)]
    delete_property: (),
    #[webapi(method, length = 3, callback = cross_origin_window_denied_callback)]
    define_property: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct CrossOriginWindowProxyHandlerDeclaration {
    #[webapi(method, length = 2, callback = cross_origin_window_proxy_has_callback)]
    has: (),
    #[webapi(method, length = 2, callback = cross_origin_window_denied_callback)]
    delete_property: (),
    #[webapi(method, length = 3, callback = cross_origin_window_denied_callback)]
    define_property: (),
}

#[derive(Default, WebApiObject)]
#[webapi(plain)]
struct CrossOriginWindowLiveAccessorsDeclaration {
    #[webapi(
        accessor_property,
        dont_delete,
        getter = cross_origin_window_length_getter_callback,
        setter = cross_origin_window_denied_callback
    )]
    length: (),
    #[webapi(
        accessor_property,
        dont_delete,
        getter = cross_origin_window_location_getter_callback,
        setter = cross_origin_location_navigate_setter_callback
    )]
    location: (),
}

#[derive(Default, WebApiObject)]
#[webapi(plain)]
struct CrossOriginWindowLocationAccessorDeclaration {
    #[webapi(
        accessor_property,
        dont_delete,
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

    // This is a property of the committed Document, independent of its URL or
    // history entries. Same-document URL updates preserve the creation kind;
    // a later navigation to about:blank installs a non-initial Document.
    pub(crate) fn child_current_document_is_initial_empty(&self, handle: DomHandle) -> bool {
        self.frame_owner_store
            .current_child_document_creation_kind(handle)
            .is_some_and(crate::frame_owner_model::DocumentCreationKind::is_initial_empty)
    }

    pub(crate) fn child_initial_empty_has_pending_attribute_navigation(
        &self,
        handle: DomHandle,
    ) -> bool {
        self.child_current_document_is_initial_empty(handle)
            && self
                .child_browsing_contexts
                .get(&handle)
                .is_some_and(|entry| entry.pending_attribute_bootstrap_commit())
    }

    pub(crate) fn child_has_committed_navigation(&self, handle: DomHandle) -> bool {
        self.frame_owner_store
            .child_has_committed_navigation(handle)
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
        self.discard_child_parser_preloads(handle, Some(owner));
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

    /// Only navigation within the same browsing context reuses its proxy.
    /// A removed/reinserted iframe element can have the same DOM handle while
    /// exposing an entirely new browsing context and WindowProxy.
    pub(crate) fn child_window_proxy_frame_is_current(
        &self,
        handle: DomHandle,
        frame_id: &str,
    ) -> bool {
        self.frame_owner_frame_id_for_child_handle(handle)
            .is_some_and(|current| current.0 == frame_id)
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
        // Endpoint projections contain functions from the outgoing Realm.
        // Clear them for both same-origin and cross-origin navigation; a
        // successor must not reuse a parent's postMessage function whose
        // creating child Realm has already retired.
        self.child_window_proxy_records
            .clear_retired_realm_projections(handle);
        // A same-origin caller can retain functions from the retired inner
        // global. Keep that proxy detached until the replacement LocalWindow
        // takes it over so those closures continue to resolve against their
        // old inner global. Cross-origin callers can observe only the stable
        // WindowProxy whitelist, so park that proxy on a restricted facade.
        if self.top_window_can_access_child(handle) {
            return true;
        }
        let Some(frame_id) = self.frame_owner_frame_id_for_child_handle(handle) else {
            return false;
        };
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
        let previous = context.set_slot(Rc::new(ChildWindowProxyFacadeContextHandle(
            handle, frame_id.0,
        )));
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
        if install_live_cross_origin_child_window_surface(
            facade_scope,
            window_proxy,
            handle,
            parent,
            top,
            window_proxy,
            child_frame_count,
            &named_indices,
        )
        .is_none()
        {
            return false;
        }
        self.child_window_proxy_records
            .set_facade_context(handle, facade_context);
        true
    }

    pub(crate) fn child_window_proxy_shell_is_exposed(&self, handle: DomHandle) -> bool {
        self.child_window_proxy_records
            .live_window_exposed_to_top(handle)
    }

    pub(crate) fn child_browsing_context_window_proxy_for_accessing_context<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let accessing = cross_origin_accessing_context(scope);
        if self.accessing_context_can_access_child_window(accessing, handle) {
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
            let context = proxy.get_creation_context(scope)?;
            let scope = &mut v8::ContextScope::new(scope, context);
            let top = Self::child_window_object_slot(scope, proxy, "top")
                .unwrap_or_else(|| scope.get_current_context().global(scope));
            install_cross_origin_window_index_slots(scope, proxy, child_frame_count, proxy, top)?;
            let named_indices = self.child_browsing_context_child_frame_named_indices(handle);
            install_cross_origin_window_named_slots(scope, proxy, &named_indices)?;
            return Some(proxy);
        }
        if !self.child_browsing_contexts.contains_key(&handle) {
            return None;
        }

        let frame_id = self.frame_owner_frame_id_for_child_handle(handle)?;
        let (proxy, facade_context) = self.bridge.bindings.instantiate_window_proxy_shell(scope);
        let global = scope.get_current_context().global(scope);
        let top = self.child_browsing_context_root_window(scope, handle, global);
        let parent = self.child_browsing_context_parent_window(scope, handle, top);
        self.child_window_proxy_records
            .set_browsing_context_parent_top(scope, handle, parent, top);
        let named_indices = self.child_browsing_context_child_frame_named_indices(handle);
        let context = v8::Local::new(scope, &facade_context);
        crate::util::install_context_host_pointer_slot(
            context,
            self as *mut Self,
            self.context_host_lifecycle_handle(),
        );
        context.set_slot(Rc::new(ChildWindowProxyFacadeContextHandle(
            handle, frame_id.0,
        )));
        {
            let scope = &mut v8::ContextScope::new(scope, context);
            set_null_prototype(scope, proxy);
            install_live_cross_origin_child_window_surface(
                scope,
                proxy,
                handle,
                parent,
                top,
                proxy,
                child_frame_count,
                &named_indices,
            )?;
        }
        self.child_window_proxy_records
            .set_facade_context(handle, facade_context);
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

    pub(crate) fn child_window_proxy_is_current<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
        window: v8::Local<'s, v8::Object>,
    ) -> bool {
        // Isolated worlds have their own global proxy for the same native
        // LocalWindow. Accept them through the registered owner/realm identity.
        if let Some(context) = window.get_creation_context(scope)
            && window.strict_equals(context.global(scope).into())
            && self
                .window_execution_context_identity_for_access_check(context)
                .is_some_and(|identity| {
                    identity.dispatch_scope() == super::super::OwnerDispatchScope::Child(handle)
                        && self.window_execution_context_identity_is_current(identity)
                })
        {
            return true;
        }
        // A facade parked between LocalWindow generations has no registered
        // execution realm, but still represents the current browsing context.
        self.child_window_proxy_records
            .records
            .get(&handle)
            .is_some_and(|record| {
                [
                    record.live_window_wrapper.as_ref(),
                    record.cross_origin_window_proxy.as_ref(),
                ]
                .into_iter()
                .flatten()
                .any(|current| v8::Local::new(scope, current).strict_equals(window.into()))
            })
    }

    pub(crate) fn existing_child_browsing_context_window_wrapper<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.child_window_proxy_records.live_window(scope, handle)
    }

    pub(crate) fn set_child_browsing_context_opener<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
        endpoint: PendingWindowMessageEndpoint,
        opener: v8::Local<'s, v8::Object>,
    ) {
        if !self.child_browsing_context_is_live(handle) {
            return;
        }
        self.child_window_proxy_records
            .set_browsing_context_opener(scope, handle, endpoint, opener);
    }

    pub(crate) fn set_top_window_opener(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        endpoint: PendingWindowMessageEndpoint,
        opener: v8::Local<'_, v8::Object>,
    ) {
        self.top_window_opener = Some((endpoint, v8::Global::new(scope, opener)));
    }

    pub(crate) fn top_window_opener<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        if self.auxiliary_window().is_some()
            && let Some(environment) = self.page_script_environment()
        {
            // Named reuse changes the related Page's opener synchronously.
            // Its script environment also records an explicitly disowned
            // opener; an older endpoint must not restore that relationship.
            let opener = environment.opener_in_scope(scope)?;
            let host = opener
                .get_creation_context(scope)
                .and_then(crate::util::context_host_ptr_from_context_slot)?;
            return (!unsafe { &*host }.browsing_context_is_closed()).then_some(opener);
        }
        let (endpoint, opener) = self.top_window_opener.as_ref()?;
        let opener = v8::Local::new(scope, opener);
        self.window_opener_endpoint_is_live(scope, *endpoint, opener)
            .then_some(opener)
    }

    pub(crate) fn clear_top_window_opener(&mut self) {
        self.top_window_opener = None;
    }

    pub(crate) fn clear_child_browsing_context_opener(&mut self, handle: DomHandle) {
        self.child_window_proxy_records
            .clear_browsing_context_opener(handle);
    }

    pub(crate) fn child_browsing_context_opener<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let (endpoint, opener) = self
            .child_window_proxy_records
            .browsing_context_opener(scope, handle)?;
        self.window_opener_endpoint_is_live(scope, endpoint, opener)
            .then_some(opener)
    }

    pub(in crate::native_bridge::context_host) fn window_opener_endpoint_is_live<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        endpoint: PendingWindowMessageEndpoint,
        opener: v8::Local<'s, v8::Object>,
    ) -> bool {
        match endpoint {
            PendingWindowMessageEndpoint::TopWindow => true,
            PendingWindowMessageEndpoint::ChildWindow(opener_handle) => {
                if !self.child_browsing_context_is_live(opener_handle) {
                    false
                } else {
                    opener
                        .get_creation_context(scope)
                        .and_then(|context| {
                            self.window_execution_context_identity_for_access_check(context)
                        })
                        .is_some_and(|identity| {
                            identity.dispatch_scope()
                                == super::super::OwnerDispatchScope::Child(opener_handle)
                                && self.window_execution_context_identity_is_current(identity)
                        })
                }
            }
            PendingWindowMessageEndpoint::LightweightPopup(popup_id) => {
                self.lightweight_popup_is_open(popup_id)
            }
        }
    }

    pub(crate) fn child_browsing_context_top_window_for_current_realm<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.child_window_proxy_records
            .browsing_context_top(scope, handle)
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
        let top = self.child_browsing_context_popup_root_window_for_realm(scope, handle).unwrap_or(root);
        let parent = popup_owner
            .or(stable_parent)
            .or(existing_parent)
            .unwrap_or_else(|| self.child_browsing_context_parent_window(scope, handle, top));
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
        if let Some(top) = self.child_browsing_context_popup_root_window_for_realm(scope, handle) {
            let parent = self.child_browsing_context_parent_window(scope, handle, top);
            return (parent, top);
        }
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

        let top = self
            .child_window_proxy_records
            .browsing_context_top(scope, handle)
            .or_else(|| {
                Self::child_window_non_cross_origin_object_slot(scope, window, "__moliWindowTop")
            })
            .unwrap_or_else(|| self.child_browsing_context_root_window(scope, handle, global));
        let parent = self
            .child_window_proxy_records
            .browsing_context_parent(scope, handle)
            .or_else(|| {
                Self::child_window_non_cross_origin_object_slot(scope, window, "__moliWindowParent")
            })
            .unwrap_or_else(|| self.child_browsing_context_parent_window(scope, handle, top));
        (parent, top)
    }

    pub(in crate::native_bridge::context_host) fn child_browsing_context_root_window<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
        fallback: v8::Local<'s, v8::Object>,
    ) -> v8::Local<'s, v8::Object> {
        if let Some(window) = self.child_browsing_context_popup_root_window_for_realm(scope, handle)
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
        self.popup_window_for_child_realm(scope, handle, popup_id)
    }

    fn child_browsing_context_popup_root_window_for_realm<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        // The root can be a popup even when the immediate parent is a child
        // frame. Project it for the accessing child, not for its parent.
        let super::super::OwnerDispatchScope::LightweightPopup(popup_id) =
            self.child_browsing_context_root_scope(handle)?
        else {
            return None;
        };
        self.popup_window_for_child_realm(scope, handle, popup_id)
    }

    fn popup_window_for_child_realm<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        _handle: DomHandle,
        popup_id: u64,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.lightweight_popup_window(scope, popup_id)
    }

    fn cross_origin_window_endpoint_projection_for_child<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        _accessing_handle: DomHandle,
        endpoint: PendingWindowMessageEndpoint,
    ) -> Option<v8::Local<'s, v8::Object>> {
        match endpoint {
            PendingWindowMessageEndpoint::TopWindow => {
                let dispatch = super::super::OwnerDispatchScope::Top;
                let owner = self.current_window_execution_context_owner(dispatch)?;
                let (_, context) = self.window_execution_context(scope, owner, dispatch)?;
                Some(context.global(scope))
            }
            PendingWindowMessageEndpoint::ChildWindow(handle) => {
                self.child_browsing_context_cross_origin_window_proxy(scope, handle)
            }
            PendingWindowMessageEndpoint::LightweightPopup(popup_id) => {
                self.lightweight_popup_window(scope, popup_id)
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
) -> Option<()> {
    for index in 0..count.min(u32::MAX as usize) {
        let index_name = index.to_string();
        let key = v8_string(scope, &index_name)?;
        if window.has_own_property(scope, key.into())? {
            continue;
        }
        let child = build_detached_cross_origin_window_index_proxy(scope, parent, top)?;
        window
            .define_own_property(
                scope,
                key.into(),
                child.into(),
                cross_origin_index_property_attributes(),
            )
            .filter(|defined| *defined)?;
    }
    Some(())
}

fn install_cross_origin_window_named_slots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    named_indices: &[(usize, String)],
) -> Option<()> {
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
        let key = v8_string(scope, name)?;
        if window.has_own_property(scope, key.into())? {
            continue;
        }
        let value = window.get_index(scope, *index as u32)?;
        window
            .define_own_property(
                scope,
                key.into(),
                value,
                cross_origin_named_property_attributes(),
            )
            .filter(|defined| *defined)?;
    }
    set_cross_origin_window_named_slot_registry(scope, window, &current_names);
    Some(())
}

fn is_cross_origin_named_child_slot_name(name: &str) -> bool {
    !name.is_empty()
        && crate::native_bridge::collections::array_index_property_name(name).is_none()
        && !CROSS_ORIGIN_WINDOW_NOOP_METHODS.contains(&name)
        && !matches!(
            name,
            "window"
                | "self"
                | "top"
                | "parent"
                | "frames"
                | "location"
                | "closed"
                | "opener"
                | "postMessage"
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
) -> Option<()> {
    let location = build_cross_origin_location_proxy(scope, handle)?;
    install_cross_origin_window_identity_slots(scope, window, handle, parent, top);
    install_cross_origin_window_index_slots(scope, window, child_frame_count, indexed_parent, top)?;
    install_cross_origin_symbol_slots(scope, window);
    set_cross_origin_object_slot(
        scope,
        window,
        "closed",
        v8::Boolean::new(scope, false).into(),
    );
    location::set_slot(
        scope,
        window,
        CROSS_ORIGIN_WINDOW_LOCATION_SLOT,
        location.into(),
    )?;
    CrossOriginWindowLiveAccessorsDeclaration::default()
        .initialize(scope, window)
        .ok()?;
    set_cross_origin_object_slot(scope, window, "opener", v8::null(scope).into());
    set_cross_origin_object_slot(scope, window, "then", v8::undefined(scope).into());
    install_cross_origin_window_methods(scope, window)?;
    install_cross_origin_denied_accessors(scope, window, CROSS_ORIGIN_DENIED_WINDOW_PROPERTIES)?;
    install_cross_origin_window_named_slots(scope, window, named_indices)
}

fn build_detached_cross_origin_window_index_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent: v8::Local<'s, v8::Object>,
    top: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let location = build_detached_cross_origin_location_proxy(scope)?;
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
    location::set_slot(
        scope,
        window,
        CROSS_ORIGIN_WINDOW_LOCATION_SLOT,
        location.into(),
    )?;
    CrossOriginWindowLocationAccessorDeclaration::default()
        .initialize(scope, window)
        .ok()?;
    install_cross_origin_window_methods(scope, window)?;
    install_cross_origin_denied_accessors(scope, window, CROSS_ORIGIN_DENIED_WINDOW_PROPERTIES)?;
    install_cross_origin_symbol_slots(scope, window);
    let proxy = wrap_cross_origin_window_with_has_trap(scope, window)?;
    set_cross_origin_object_slot(scope, window, "self", proxy.into());
    set_cross_origin_object_slot(scope, window, "window", proxy.into());
    set_cross_origin_object_slot(scope, window, "globalThis", proxy.into());
    set_cross_origin_object_slot(scope, window, "frames", proxy.into());
    Some(proxy)
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
    set_cross_origin_object_slot(scope, window, "globalThis", identity.into());
    set_cross_origin_object_slot(scope, window, "parent", identity.into());
    set_cross_origin_object_slot(scope, window, "top", identity.into());
    set_cross_origin_object_slot(scope, window, "frames", identity.into());
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
) -> Option<()> {
    CrossOriginWindowMethodsDeclaration::default()
        .initialize(scope, object)
        .ok()
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
    set_cross_origin_object_slot(scope, window, "globalThis", window.into());
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
}

fn install_cross_origin_denied_accessors(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    names: &[&'static str],
) -> Option<()> {
    for name in names {
        define_function_accessor_property(
            scope,
            object,
            name,
            cross_origin_window_denied_callback,
            None,
            cross_origin_window_denied_callback,
            None,
            cross_origin_property_attributes(),
        )
        .ok()?;
    }
    Some(())
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
) -> Option<v8::Local<'s, v8::Object>> {
    let target = new_cross_origin_location_proxy_target(scope)?;
    location::set_slot(
        scope,
        target,
        CHILD_BROWSING_CONTEXT_HANDLE_SLOT,
        v8::Number::new(scope, handle.index() as f64).into(),
    )?;
    initialize_cross_origin_location_proxy(scope, target)
}

fn build_detached_cross_origin_location_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let target = new_cross_origin_location_proxy_target(scope)?;
    initialize_cross_origin_location_proxy(scope, target)
}

fn initialize_cross_origin_location_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    install_cross_origin_location_keys(scope, target)?;
    let location = wrap_cross_origin_location_proxy(scope, target)?;
    location::set_slot(
        scope,
        target,
        CROSS_ORIGIN_LOCATION_PROXY_SELF_SLOT,
        location.into(),
    )?;
    Some(location)
}

fn new_cross_origin_location_proxy_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let target = v8::Object::new(scope);
    target
        .set_prototype(scope, v8::null(scope).into())
        .filter(|set| *set)?;
    location::set_slot(
        scope,
        target,
        CROSS_ORIGIN_LOCATION_PROXY_SLOT,
        v8::Boolean::new(scope, true).into(),
    )?;
    moli_webapi_declare::initialize_web_api_object(scope, target, "Location").ok()?;
    Some(target)
}

fn wrap_cross_origin_location_proxy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let handler = CrossOriginLocationProxyHandlerDeclaration {
        get: (),
        set: (),
        get_own_property_descriptor: (),
        own_keys: (),
        has: (),
        delete_property: (),
        define_property: (),
    }
    .bind(scope)
    .ok()?;
    set_null_prototype(scope, handler);
    let proxy = v8::Proxy::new(scope, target, handler)?;
    moli_webapi_declare::register_web_api_proxy(scope, proxy).ok()?;
    let proxy: v8::Local<'s, v8::Value> = proxy.into();
    v8::Local::<v8::Object>::try_from(proxy).ok()
}

fn install_cross_origin_location_keys<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Option<()> {
    for key in cross_origin_location_keys(scope) {
        target
            .define_own_property(
                scope,
                key.try_into().ok()?,
                v8::undefined(scope).into(),
                v8::PropertyAttribute::DONT_ENUM,
            )
            .filter(|defined| *defined)?;
    }
    Some(())
}

fn cross_origin_location_keys<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Vec<v8::Local<'s, v8::Value>> {
    let mut keys: Vec<v8::Local<v8::Value>> = LocationMembers::cross_origin_properties()
        .iter()
        .map(|member| v8str(scope, member.name()).into())
        .collect();
    keys.push(v8str(scope, "then").into());
    for symbol in [
        v8::Symbol::get_to_string_tag(scope),
        v8::Symbol::get_has_instance(scope),
        v8::Symbol::get_is_concat_spreadable(scope),
    ] {
        keys.push(symbol.into());
    }
    keys
}

fn install_cross_origin_symbol_slots(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) {
    // CrossOriginPropertyFallback exposes undefined for all three symbols;
    // the interface's normal toStringTag is only visible to same-origin code.
    let undefined = v8::undefined(scope).into();
    for symbol in [
        v8::Symbol::get_to_string_tag(scope),
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
    // Proxy traps may execute in the target context; their initiating script
    // settings object determines the realm of exposed functions and errors.
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
    let Ok(key) = v8::Local::<v8::Name>::try_from(args.get(1)) else {
        return;
    };
    let member = v8::Local::<v8::String>::try_from(key)
        .ok()
        .and_then(|key| LocationMembers::cross_origin_property(&key.to_rust_string_lossy(scope)));
    let Some(member) = member.filter(|member| member.has_getter()) else {
        if is_fallback_key(scope, key) {
            rv.set_undefined();
        } else {
            throw_cross_origin_location_security_error(scope);
        }
        return;
    };
    let Ok(receiver) = v8::Local::<v8::Object>::try_from(args.get(2)) else {
        throw_cross_origin_illegal_invocation(scope);
        return;
    };
    let context = cross_origin_accessing_context(scope);
    let scope = &mut v8::ContextScope::new(scope, context);
    if let Some(value) = member.get(scope, receiver) {
        rv.set(value);
    }
}

fn cross_origin_location_proxy_descriptor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(key) = v8::Local::<v8::Name>::try_from(args.get(1)) else {
        return;
    };
    let member = v8::Local::<v8::String>::try_from(key)
        .ok()
        .and_then(|key| LocationMembers::cross_origin_property(&key.to_rust_string_lossy(scope)));
    let context = cross_origin_accessing_context(scope);
    let scope = &mut v8::ContextScope::new(scope, context);
    if let Some(member) = member {
        if let Some(descriptor) = member.descriptor(scope) {
            rv.set(descriptor.into());
        }
    } else if is_fallback_key(scope, key) {
        if let Ok(descriptor) = CrossOriginPropertyDescriptorDeclaration::new(
            v8::undefined(scope).into(),
            false,
            false,
            true,
        )
        .bind(scope)
        {
            rv.set(descriptor.into());
        }
    } else {
        throw_cross_origin_location_security_error(scope);
    }
}

fn cross_origin_location_proxy_own_keys_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let context = cross_origin_accessing_context(scope);
    let scope = &mut v8::ContextScope::new(scope, context);
    let keys = cross_origin_location_keys(scope);
    rv.set(v8::Array::new_with_elements(scope, &keys).into());
}

fn cross_origin_location_proxy_has_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(key) = v8::Local::<v8::Name>::try_from(args.get(1)) else {
        return;
    };
    let known = v8::Local::<v8::String>::try_from(key)
        .ok()
        .and_then(|key| LocationMembers::cross_origin_property(&key.to_rust_string_lossy(scope)))
        .is_some();
    if known || is_fallback_key(scope, key) {
        rv.set_bool(true);
    } else {
        throw_cross_origin_location_security_error(scope);
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

fn cross_origin_window_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    if crate::native_bridge::lightweight_popup_id_from_window(scope, args.this()).is_some() {
        super::super::popups::lightweight_popup_close_callback(scope, args, rv);
    } else {
        crate::context_bootstrap::window_close_callback(scope, args, rv);
    }
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
        throw_cross_origin_illegal_invocation(scope);
        return;
    };
    let count = unsafe { &mut *host_ptr }.child_browsing_context_child_frame_count(handle);
    rv.set_uint32(count as u32);
}

fn live_location_for_cross_origin_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if crate::web_api_interfaces::Window::is_instance(scope, receiver) {
        return crate::context_bootstrap::window_location_for_holder(scope, receiver);
    }
    get_cross_origin_proxy_private_value(scope, receiver, CROSS_ORIGIN_WINDOW_LOCATION_SLOT)?;
    let dispatch_scope = if let Some(popup_id) = cross_origin_lightweight_popup_id(scope, receiver)
    {
        super::super::OwnerDispatchScope::LightweightPopup(popup_id)
    } else if is_cross_origin_top_window_proxy(scope, receiver) {
        super::super::OwnerDispatchScope::Top
    } else {
        super::super::OwnerDispatchScope::Child(child_handle_from_object(scope, receiver)?)
    };
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let host = unsafe { &mut *host_ptr };
    let owner = host.current_window_execution_context_owner(dispatch_scope)?;
    let (_, context) = host.window_execution_context(scope, owner, dispatch_scope)?;
    crate::context_bootstrap::window_location_for_holder(scope, context.global(scope))
}

fn cross_origin_window_location_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(location) = live_location_for_cross_origin_window(scope, args.this()) {
        rv.set(location.into());
    } else if let Some(location) =
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
    if crate::context_bootstrap::navigate_borrowed_location(
        scope,
        args.this(),
        args.get(0),
        crate::context_bootstrap::LocationNavigationKind::Assign,
    ) {
        return;
    }
    let _ = cross_origin_location_navigate(scope, args.this(), args.get(0));
}

fn cross_origin_location_navigate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) -> bool {
    if location::is_window_location(scope, receiver) {
        return location::navigate(
            scope,
            receiver,
            value,
            crate::context_bootstrap::LocationNavigationKind::Assign,
        );
    }
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
    if cross_origin_location_target_is_fragment_navigation(host, handle, &target)
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

fn cross_origin_location_target_is_fragment_navigation(
    host: &JsContextHost,
    handle: DomHandle,
    target: &url::Url,
) -> bool {
    target.fragment().is_some()
        && host
            .child_browsing_context_current_url(handle)
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
    if crate::context_bootstrap::is_native_location(scope, args.this()) {
        let Some(parsed) = webidl::parse_args::<CrossOriginLocationReplaceArgs>(scope, &args)
        else {
            return;
        };
        let Some(value) = v8_string(scope, &parsed.url) else {
            return;
        };
        crate::context_bootstrap::navigate_borrowed_location(
            scope,
            args.this(),
            value.into(),
            crate::context_bootstrap::LocationNavigationKind::Replace,
        );
        return;
    }
    if location::is_window_location(scope, args.this()) {
        let Some(parsed) = webidl::parse_args::<CrossOriginLocationReplaceArgs>(scope, &args)
        else {
            return;
        };
        let Some(value) = v8_string(scope, &parsed.url) else {
            return;
        };
        location::navigate(
            scope,
            args.this(),
            value.into(),
            crate::context_bootstrap::LocationNavigationKind::Replace,
        );
        return;
    }
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
    let context = cross_origin_accessing_context(scope);
    let initiator = host.window_dispatch_scope_for_context(scope, context);
    let _ = host.queue_child_browsing_context_navigation_from_existing_seed(
        handle,
        target.as_str(),
        true,
        initiator,
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
