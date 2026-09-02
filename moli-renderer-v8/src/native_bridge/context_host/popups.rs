use super::{
    JsContextHost, NavigationHistoryEntrySeed, child_frame_runtime::WINDOW_EVENT_HANDLER_PROPERTIES,
};
use crate::{
    content_security_policy::content_security_policy_forces_opaque_origin,
    context_bootstrap::{
        SharedWebStorageStore, WINDOW_NAME_SLOT, apply_local_window_location_navigation,
        deep_clone_shared_web_storage_store, dispatch_simple_event_target_event,
        install_navigation_bootstrap_entry_for_holder, install_simple_event_target_methods,
        install_storage_aliases_for_window,
        install_window_location_history_navigation_runtime_state, new_shared_web_storage_store,
        scoped_indexed_db_factory, sync_document_location_runtime_state_from_window,
        sync_window_location_history_navigation_runtime_surface,
        sync_window_location_runtime_state, web_storage_area_key_for_storage_key,
    },
    document_runtime::create_content_security_policy_violation_event,
    document_runtime::{DocumentPolicyContainer, DocumentSandboxPolicy, DomHandle},
    host::HostTimerOwner,
    native_bridge::{
        ComputedStyleDescriptor, ComputedStyleTargetKey, OwnerDispatchScope, WindowTaskTarget,
        child_window_handle_from_marker_data,
        document::set_document_associated_window,
        element::{
            STYLE_DECLARATION_FORCED_EMPTY_COMPUTED_SLOT, STYLE_DECLARATION_PSEUDO_ELEMENT_SLOT,
            STYLE_DECLARATION_READ_DOCUMENT_SLOT, STYLE_DECLARATION_SCREEN_HEIGHT_SLOT,
            STYLE_DECLARATION_SCREEN_WIDTH_SLOT, STYLE_DECLARATION_TARGET_CONTEXT_EPOCH_SLOT,
            STYLE_DECLARATION_TARGET_EMPTY_COMPUTED_SLOT, STYLE_DECLARATION_VIEWPORT_HEIGHT_SLOT,
            STYLE_DECLARATION_VIEWPORT_WIDTH_SLOT, SpecialBrowsingContextTarget,
        },
        helpers::{
            child_script_declared_global_names, object_has_own_named_property, set_object_slot,
            set_object_value,
        },
    },
    runtime::RendererPendingPopupActivation,
    types::{
        LoadedChildDocument, LoadedChildScriptSource, PopupClassicScriptLoadCompletion,
        PopupDocumentLoadCompletion, PopupDocumentLoadOutcome,
    },
    util::{
        context_host_ptr_from_global_bridge, define_non_enumerable_static_property,
        get_private_value, global_constructor_object, set_private_value, throw_type_error,
        v8_string, v8str,
    },
    window_document_identity::{
        LightweightPopupDocumentId, LightweightPopupDocumentOwner, LightweightPopupLocalWindowId,
    },
};
use anyhow::Result;
use moli_crypto::sha256_hex;
use moli_encoding::decode_html_document_with_fallback;
use moli_fetch::Request;
use moli_storage_key::{MoliStorageKey, StoragePartitionRelation, site_for_url};
use moli_url::origin_ascii_serialization;
use moli_webapi_declare::{ObjectLiteralDeclaration, WebApiObject};
use std::collections::{HashMap, HashSet};
use url::Url;

const LIGHTWEIGHT_POPUP_EVENT_LISTENERS_SLOT: &str = "__lmLightweightPopupEventListeners";
const LIGHTWEIGHT_POPUP_ID_SLOT: &str = "__lmLightweightPopupId";
const ACTIVE_LIGHTWEIGHT_POPUP_ID_SLOT: &str = "__lmActiveLightweightPopupId";
const LIGHTWEIGHT_POPUP_JAVASCRIPT_POPUP_ID_SLOT: &str = "__lmLightweightPopupJavascriptPopupId";
const LIGHTWEIGHT_POPUP_JAVASCRIPT_DOCUMENT_ID_SLOT: &str =
    "__lmLightweightPopupJavascriptDocumentId";
const LIGHTWEIGHT_POPUP_JAVASCRIPT_NAVIGATION_ID_SLOT: &str =
    "__lmLightweightPopupJavascriptNavigationId";
const LIGHTWEIGHT_POPUP_JAVASCRIPT_SOURCE_SLOT: &str = "__lmLightweightPopupJavascriptSource";
const LIGHTWEIGHT_POPUP_DOCUMENT_WRITE_SESSION_SLOT: &str =
    "__lmLightweightPopupDocumentWriteSession";
const LIGHTWEIGHT_POPUP_VIEWPORT_SURFACE_PROPERTIES: &[&str] = &[
    "innerWidth",
    "innerHeight",
    "outerWidth",
    "outerHeight",
    "devicePixelRatio",
];

/// Whether applying a popup response entered author code or an event-dispatch
/// algorithm before returning to the enclosing Page resource task.
///
/// This is an execution-produced fact, not scheduler metadata. The caller uses
/// it only to choose the already-selected task's completion boundary.
#[must_use = "popup body activity determines the enclosing task completion"]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PopupDocumentLoadBodyActivity {
    NoPageCodeOrEventDispatch,
    PageCodeOrEventDispatchAttempted,
}

/// Result of consuming one exact popup-document response terminal.
///
/// `NotApplied` means the inner popup target disappeared after the outer Page
/// owner had authorized the task. `Applied` remains distinct even if author
/// code replaced the popup while the body was running: any code already
/// entered still owns the surrounding task-end checkpoint.
#[must_use = "popup application determines the enclosing task completion"]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PopupDocumentLoadApplication {
    NotApplied,
    Applied {
        body_activity: PopupDocumentLoadBodyActivity,
    },
}

#[must_use = "popup script application determines the enclosing task completion"]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PopupClassicScriptLoadApplication {
    NotApplied,
    Applied {
        body_activity: PopupDocumentLoadBodyActivity,
    },
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct LightweightPopupDocumentStreamMethodsDeclaration<'scope> {
    popup_id: v8::Local<'scope, v8::BigInt>,
    #[webapi(
        method,
        callback = lightweight_popup_document_open_callback,
        data = self.popup_id
    )]
    open: (),
    #[webapi(
        method,
        callback = lightweight_popup_document_write_callback,
        data = self.popup_id
    )]
    write: (),
    #[webapi(
        method,
        callback = lightweight_popup_document_writeln_callback,
        data = self.popup_id
    )]
    writeln: (),
    #[webapi(
        method,
        callback = lightweight_popup_document_close_callback,
        data = self.popup_id
    )]
    close: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct LightweightPopupEventDeclaration<'scope> {
    r#type: v8::Local<'scope, v8::String>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct LightweightPopupPopStateEventDeclaration<'scope> {
    #[webapi(data_property)]
    state: v8::Local<'scope, v8::Value>,
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct LightweightPopupWindowMethodsDeclaration {
    #[webapi(method, length = 0, callback = lightweight_popup_close_callback)]
    close: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct LightweightPopupComputedStyleMethodDeclaration<'scope> {
    document: v8::Local<'scope, v8::BigInt>,
    #[webapi(
        method,
        length = 1,
        callback = lightweight_popup_get_computed_style_callback,
        data = self.document
    )]
    get_computed_style: (),
}

#[derive(Debug, Clone)]
pub(super) struct PendingLightweightPopupDocumentLoad {
    pub(super) target_url: Url,
    pub(super) previous_url: Url,
    pub(super) target: LightweightPopupDocumentFetchTarget,
    pub(super) document_state: LightweightPopupDocumentState,
    pub(super) resource_loader: Option<crate::network::navigation::NavigationResourceLoader>,
}

struct LightweightPopupClassicScriptContinuation {
    task: LightweightPopupNavigationTaskToken,
    document_handle: DomHandle,
    document_url: Url,
    scripts: Vec<DomHandle>,
    next_script_index: usize,
    response_content_security_policies: Vec<String>,
    response_content_security_report_only_policies: Vec<String>,
    response_content_security_reporting_endpoints:
        crate::content_security_policy::ContentSecurityPolicyReportingEndpoints,
}

enum LightweightPopupClassicScriptAdvance {
    Completed(PopupDocumentLoadBodyActivity),
    Pending(PopupDocumentLoadBodyActivity),
}

struct LightweightPopupDocumentProjectionSource<'a> {
    task: LightweightPopupNavigationTaskToken,
    document_url: &'a Url,
    markup: &'a str,
    content_type: Option<&'a str>,
    character_set: Option<&'a str>,
    policy_container: &'a DocumentPolicyContainer,
}

struct LightweightPopupDocumentProjectionApplication {
    document_handle: Option<DomHandle>,
    body_activity: PopupDocumentLoadBodyActivity,
    classic_script_load_pending: bool,
}

pub(super) struct PendingLightweightPopupClassicScriptLoad {
    target: LightweightPopupClassicScriptFetchTarget,
    script_handle: DomHandle,
    request_url: Url,
    continuation: LightweightPopupClassicScriptContinuation,
    cancel_handle: moli_fetch::FetchCancelHandle,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct LightweightPopupNavigationId(u64);

impl LightweightPopupNavigationId {
    fn new(value: u64) -> Self {
        Self(value)
    }

    fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy)]
enum LightweightPopupNavigationDocumentTarget {
    CurrentDocument,
    NewDocument,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct LightweightPopupNavigationTaskToken {
    document_owner: LightweightPopupDocumentOwner,
    navigation_id: LightweightPopupNavigationId,
}

impl LightweightPopupNavigationTaskToken {
    #[cfg(test)]
    pub(crate) fn for_test(
        document_owner: LightweightPopupDocumentOwner,
        navigation_id: u64,
    ) -> Self {
        Self {
            document_owner,
            navigation_id: LightweightPopupNavigationId::new(navigation_id),
        }
    }

    fn from_parts(
        document_owner: LightweightPopupDocumentOwner,
        navigation_id: LightweightPopupNavigationId,
    ) -> Self {
        Self {
            document_owner,
            navigation_id,
        }
    }

    pub(crate) fn document_owner(self) -> LightweightPopupDocumentOwner {
        self.document_owner
    }

    pub(crate) fn popup_id(self) -> u64 {
        self.document_owner.popup_id()
    }

    pub(crate) fn navigation_id(self) -> u64 {
        self.navigation_id.as_u64()
    }
}

/// Exact PageVm-local authorization target for one popup navigation fetch.
///
/// The navigation task binds the stable popup browsing context, the Document
/// owner that the response may commit, and the navigation generation. The
/// load id distinguishes the concrete in-flight request within that
/// navigation. A stable Page queue adds the producing root Document token to
/// namespace these PageVm-local counters across replacement.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct LightweightPopupDocumentFetchTarget {
    load_id: u64,
    task: LightweightPopupNavigationTaskToken,
}

impl LightweightPopupDocumentFetchTarget {
    fn new(load_id: u64, task: LightweightPopupNavigationTaskToken) -> Self {
        Self { load_id, task }
    }

    #[cfg(test)]
    pub(crate) fn for_test(load_id: u64, task: LightweightPopupNavigationTaskToken) -> Self {
        Self::new(load_id, task)
    }

    pub(crate) fn load_id(self) -> u64 {
        self.load_id
    }

    pub(crate) fn task(self) -> LightweightPopupNavigationTaskToken {
        self.task
    }
}

/// Exact owner of one parser-blocking classic script fetch in a lightweight
/// popup Document. The navigation token prevents a delayed response from
/// entering a replacement Document; `load_id` distinguishes successive
/// parser scripts in the same Document.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct LightweightPopupClassicScriptFetchTarget {
    load_id: u64,
    task: LightweightPopupNavigationTaskToken,
}

impl LightweightPopupClassicScriptFetchTarget {
    fn new(load_id: u64, task: LightweightPopupNavigationTaskToken) -> Self {
        Self { load_id, task }
    }

    pub(crate) fn load_id(self) -> u64 {
        self.load_id
    }

    pub(crate) fn task(self) -> LightweightPopupNavigationTaskToken {
        self.task
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LightweightPopupStorageScope {
    origin: String,
    area_key: String,
    storage_key: MoliStorageKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LightweightPopupDocumentState {
    pub(super) base_url: Url,
    pub(super) policy_container: DocumentPolicyContainer,
    pub(super) document_domain_override: Option<String>,
}

struct LightweightPopupDocumentRecord {
    owner: LightweightPopupDocumentOwner,
    local_window_id: LightweightPopupLocalWindowId,
    url: Url,
    access_origin: super::window_security_tokens::WindowAccessOrigin,
    state: LightweightPopupDocumentState,
    storage_scope: LightweightPopupStorageScope,
    wrapper: Option<v8::Global<v8::Object>>,
    handle: Option<DomHandle>,
    script_globals: HashMap<String, v8::Global<v8::Value>>,
    incomplete_child_frame_loads: HashSet<DomHandle>,
    pending_load_event: Option<LightweightPopupNavigationTaskToken>,
    queued_load_event: Option<LightweightPopupNavigationTaskToken>,
    load_event_dispatched: bool,
}

struct LightweightPopupDocumentCommit {
    owner: LightweightPopupDocumentOwner,
    location_url: Url,
    origin: LightweightPopupDocumentCommitOrigin,
    state: LightweightPopupDocumentState,
    storage_scope: LightweightPopupStorageScope,
    navigation_loader: Option<crate::network::navigation::NavigationResourceLoader>,
}

enum LightweightPopupDocumentCommitOrigin {
    RetainCurrent,
    InheritExact(super::window_security_tokens::WindowAccessOrigin),
    FromNavigationResponse(String),
}

#[derive(Debug)]
struct LightweightPopupDocumentTransition {
    retired_owner: Option<LightweightPopupDocumentOwner>,
    retired_local_window_id: Option<LightweightPopupLocalWindowId>,
    retired_document_handle: Option<DomHandle>,
    current_local_window_id: LightweightPopupLocalWindowId,
}

#[derive(Debug)]
struct LightweightPopupCloseTransition {
    retired_owner: LightweightPopupDocumentOwner,
    retired_local_window_id: LightweightPopupLocalWindowId,
    retired_document_handle: Option<DomHandle>,
}

struct LightweightPopupOpenState {
    document: LightweightPopupDocumentRecord,
    session_storage_store: SharedWebStorageStore,
}

enum LightweightPopupLifecycle {
    Open(Box<LightweightPopupOpenState>),
    Closed,
}

pub(super) struct LightweightPopupBrowsingContextRecord {
    window_proxy: v8::Global<v8::Object>,
    opener: Option<super::PendingWindowMessageEndpoint>,
    location_url: Url,
    opener_sandbox_policy: Option<DocumentSandboxPolicy>,
    lifecycle: LightweightPopupLifecycle,
    navigation_id: LightweightPopupNavigationId,
}

impl LightweightPopupBrowsingContextRecord {
    fn is_open(&self) -> bool {
        matches!(self.lifecycle, LightweightPopupLifecycle::Open(_))
    }
}

pub(crate) struct OpenedLightweightPopup<'scope> {
    pub(crate) window: v8::Local<'scope, v8::Object>,
    pub(crate) popup_id: u64,
    pub(crate) created_new_browsing_context: bool,
}

fn inherit_lightweight_popup_opener_sandbox(
    target: &mut DocumentSandboxPolicy,
    opener: Option<DocumentSandboxPolicy>,
) {
    let Some(opener) = opener else {
        return;
    };
    target.forces_opaque_origin |= opener.forces_opaque_origin;
    target.allows_scripts &= opener.allows_scripts;
    target.allows_popups_to_escape &= opener.allows_popups_to_escape;
    target.sandboxes_document_domain |= opener.sandboxes_document_domain;
}

impl LightweightPopupDocumentState {
    fn new(base_url: Url, policy_container: DocumentPolicyContainer) -> Self {
        Self {
            base_url,
            policy_container,
            document_domain_override: None,
        }
    }

    fn reset_for_empty_document(&mut self, base_url: Url) {
        self.base_url = base_url;
        self.document_domain_override = None;
    }

    fn apply_navigation_response(
        mut self,
        base_url: Url,
        response: DocumentPolicyContainer,
    ) -> Self {
        self.base_url = base_url;
        self.document_domain_override = None;
        self.policy_container.referrer_policy = response.referrer_policy;
        self.policy_container.cross_origin_embedder_policy = response.cross_origin_embedder_policy;
        self.policy_container.document_isolation_policy = response.document_isolation_policy;
        self.policy_container.cross_origin_isolated = response.cross_origin_isolated;
        self.policy_container.document_content_security_policies =
            response.document_content_security_policies;
        self.policy_container.response_content_security_policies =
            response.response_content_security_policies;
        self.policy_container
            .response_content_security_report_only_policies =
            response.response_content_security_report_only_policies;
        self.policy_container.content_security_reporting_endpoints =
            response.content_security_reporting_endpoints;
        self.policy_container.sandbox = response.sandbox;
        self
    }
}

impl LightweightPopupStorageScope {
    fn new(origin: String, area_key: String, storage_key: MoliStorageKey) -> Self {
        Self {
            origin,
            area_key,
            storage_key,
        }
    }

    fn from_web_storage_scope(scope: super::child_frames::WebStorageScope) -> Self {
        let (origin, area_key, storage_key) = scope.into_parts();
        Self {
            origin,
            area_key,
            storage_key,
        }
    }

    pub(super) fn origin(&self) -> &str {
        &self.origin
    }

    pub(super) fn area_key(&self) -> &str {
        &self.area_key
    }

    pub(super) fn storage_key(&self) -> &MoliStorageKey {
        &self.storage_key
    }
}

impl JsContextHost {
    fn lightweight_popup_record(
        &self,
        popup_id: u64,
    ) -> Option<&LightweightPopupBrowsingContextRecord> {
        self.lightweight_popup_browsing_contexts.get(&popup_id)
    }

    fn lightweight_popup_record_mut(
        &mut self,
        popup_id: u64,
    ) -> Option<&mut LightweightPopupBrowsingContextRecord> {
        self.lightweight_popup_browsing_contexts.get_mut(&popup_id)
    }

    fn lightweight_popup_document_record(
        &self,
        popup_id: u64,
    ) -> Option<&LightweightPopupDocumentRecord> {
        match &self.lightweight_popup_record(popup_id)?.lifecycle {
            LightweightPopupLifecycle::Open(open) => Some(&open.document),
            LightweightPopupLifecycle::Closed => None,
        }
    }

    fn lightweight_popup_document_record_mut(
        &mut self,
        popup_id: u64,
    ) -> Option<&mut LightweightPopupDocumentRecord> {
        match &mut self.lightweight_popup_record_mut(popup_id)?.lifecycle {
            LightweightPopupLifecycle::Open(open) => Some(&mut open.document),
            LightweightPopupLifecycle::Closed => None,
        }
    }

    fn lightweight_popup_has_document_projection(&self, popup_id: u64) -> bool {
        self.lightweight_popup_document_record(popup_id)
            .is_some_and(|document| document.wrapper.is_some())
    }

    fn set_lightweight_popup_document_wrapper(
        &mut self,
        popup_id: u64,
        wrapper: v8::Global<v8::Object>,
    ) -> bool {
        let Some(document) = self.lightweight_popup_document_record_mut(popup_id) else {
            return false;
        };
        document.wrapper = Some(wrapper);
        true
    }

    fn clear_lightweight_popup_document_projection(&mut self, popup_id: u64) {
        self.forget_lightweight_popup_document_handle(popup_id);
        if let Some(document) = self.lightweight_popup_document_record_mut(popup_id) {
            document.wrapper = None;
        }
    }

    fn lightweight_popup_navigation_id(
        &self,
        popup_id: u64,
    ) -> Option<LightweightPopupNavigationId> {
        self.lightweight_popup_record(popup_id)
            .map(|record| record.navigation_id)
    }

    fn close_lightweight_popup_browsing_context(
        &mut self,
        popup_id: u64,
    ) -> Option<LightweightPopupCloseTransition> {
        let record = self.lightweight_popup_record_mut(popup_id)?;
        let lifecycle = std::mem::replace(&mut record.lifecycle, LightweightPopupLifecycle::Closed);
        let LightweightPopupLifecycle::Open(open) = lifecycle else {
            return None;
        };
        record.navigation_id = LightweightPopupNavigationId::new(
            record
                .navigation_id
                .as_u64()
                .checked_add(1)
                .expect("lightweight popup navigation id space exhausted"),
        );
        record.opener = None;
        Some(LightweightPopupCloseTransition {
            retired_owner: open.document.owner,
            retired_local_window_id: open.document.local_window_id,
            retired_document_handle: open.document.handle,
        })
    }

    fn set_lightweight_popup_same_document_url(&mut self, popup_id: u64, url: Url) -> bool {
        let Some(record) = self.lightweight_popup_record_mut(popup_id) else {
            return false;
        };
        let LightweightPopupLifecycle::Open(open) = &mut record.lifecycle else {
            return false;
        };
        record.location_url = url.clone();
        open.document.url = url;
        true
    }

    pub(crate) fn open_lightweight_popup_window<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        opener: Option<v8::Local<'s, v8::Object>>,
        opener_child_handle: Option<DomHandle>,
        target_name: &str,
        href: &str,
        creator_base_url: Url,
        creator_policy_container: DocumentPolicyContainer,
    ) -> Option<OpenedLightweightPopup<'s>> {
        if opener.is_some()
            && let Some(name) = trackable_lightweight_popup_window_name(target_name)
            && let Some(popup_id) = self.lightweight_popup_window_names.get(&name).copied()
            && self.lightweight_popup_is_open(popup_id)
            && let Some(window) = self.reopen_lightweight_popup_window(
                scope,
                popup_id,
                opener,
                opener_child_handle,
                href,
                creator_base_url.clone(),
                creator_policy_container.clone(),
            )
        {
            return Some(OpenedLightweightPopup {
                window,
                popup_id,
                created_new_browsing_context: false,
            });
        }
        let (window, popup_id) = self.create_lightweight_popup_window(
            scope,
            host_ptr,
            opener,
            opener_child_handle,
            target_name,
            href,
            opener_child_handle
                .and_then(|handle| self.child_browsing_context_popup_opener_sandbox_policy(handle)),
            creator_base_url,
            creator_policy_container,
        )?;
        Some(OpenedLightweightPopup {
            window,
            popup_id,
            created_new_browsing_context: true,
        })
    }

    pub(crate) fn create_lightweight_popup_window<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        opener: Option<v8::Local<'s, v8::Object>>,
        opener_child_handle: Option<DomHandle>,
        target_name: &str,
        href: &str,
        opener_sandbox_policy: Option<DocumentSandboxPolicy>,
        creator_base_url: Url,
        creator_policy_container: DocumentPolicyContainer,
    ) -> Option<(v8::Local<'s, v8::Object>, u64)> {
        let parsed_url = Url::parse(href).ok()?;
        let initial_url = if parsed_url.scheme() == "javascript" {
            about_blank_url()
        } else {
            parsed_url.clone()
        };
        let initial_base_url = lightweight_popup_initial_base_url(&initial_url, creator_base_url);
        let initial_referrer = creator_policy_container.document_referrer.clone();
        let opener_endpoint =
            lightweight_popup_initiator_endpoint(scope, opener, opener_child_handle);
        let creator_resource_authority = self
            .document_resource_loader_for_dispatch_scope(self.entered_owner_dispatch_scope(scope))
            .expect("new popup Document requires its creator's exact resource authority")
            .clone();
        let storage_scope = self.lightweight_popup_storage_scope_for_initiated_navigation(
            scope,
            opener,
            opener_child_handle,
            &initial_url,
            opener_sandbox_policy.is_some_and(|policy| policy.forces_opaque_origin),
        );
        let tracked_name = opener
            .is_some()
            .then(|| trackable_lightweight_popup_window_name(target_name))
            .flatten();
        let popup_id = self.next_lightweight_popup_id;
        self.next_lightweight_popup_id = self
            .next_lightweight_popup_id
            .checked_add(1)
            .expect("lightweight popup id space exhausted");
        let window = self
            .bridge
            .bindings
            .instantiate_window_shell(scope, host_ptr);
        if let Some(dom_exception) = global_constructor_object(scope, "DOMException") {
            // Lightweight popups share the opener's V8 realm, so their
            // synthetic Window shell must expose the same realm constructor
            // used by DOM exceptions thrown from popup-owned callbacks.
            define_non_enumerable_static_property(
                scope,
                window,
                "DOMException",
                dom_exception.into(),
            );
        }
        let popup_id_private_value = v8::BigInt::new_from_u64(scope, popup_id);
        set_private_value(
            scope,
            window,
            LIGHTWEIGHT_POPUP_ID_SLOT,
            popup_id_private_value.into(),
        );
        install_window_location_history_navigation_runtime_state(
            scope,
            window,
            initial_url.as_str(),
        )
        .ok()?;
        sync_window_location_history_navigation_runtime_surface(scope, window);
        set_object_slot(scope, window, "__moliWindowSelf", window.into());
        set_object_slot(scope, window, "__moliWindowParent", window.into());
        set_object_slot(scope, window, "__moliWindowTop", window.into());
        set_object_slot(scope, window, "__moliWindowFrames", window.into());
        set_object_slot(
            scope,
            window,
            "closed",
            v8::Boolean::new(scope, false).into(),
        );
        set_object_slot(scope, window, "self", window.into());
        set_object_slot(scope, window, "window", window.into());
        set_object_slot(scope, window, "globalThis", window.into());
        set_object_slot(scope, window, "parent", window.into());
        set_object_slot(scope, window, "top", window.into());
        set_object_slot(scope, window, "frames", window.into());
        if let Some(name) = trackable_lightweight_popup_window_name(target_name).as_deref()
            && let Some(value) = v8_string(scope, name)
        {
            set_object_slot(scope, window, WINDOW_NAME_SLOT, value.into());
            set_object_slot(scope, window, "name", value.into());
        }
        if let Some(opener) = opener {
            set_object_slot(scope, window, "opener", opener.into());
            install_lightweight_popup_viewport_surface_from_opener(scope, opener, window);
        } else {
            let opener = v8::null(scope);
            set_object_slot(scope, window, "opener", opener.into());
        }
        if let Ok(navigator) =
            crate::context_bootstrap::build_lightweight_popup_window_navigator_object(
                scope, popup_id,
            )
        {
            set_object_slot(scope, window, "navigator", navigator.into());
        }
        let _ = install_storage_aliases_for_window(scope, window);
        install_simple_event_target_methods(
            scope,
            window,
            LIGHTWEIGHT_POPUP_EVENT_LISTENERS_SLOT,
            false,
        );
        let _ = LightweightPopupWindowMethodsDeclaration::default().initialize(scope, window);
        let initial_document_owner = self.allocate_lightweight_popup_document_owner(popup_id);
        let initial_local_window_id = self.allocate_lightweight_popup_local_window_id();
        let initial_execution_context_owner =
            super::WindowExecutionContextOwner::LightweightPopup {
                popup_id,
                local_window_id: initial_local_window_id,
            };
        let initial_origin =
            if opener_sandbox_policy.is_some_and(|policy| policy.forces_opaque_origin) {
                super::window_security_tokens::WindowAccessOrigin::opaque(
                    initial_execution_context_owner,
                )
            } else if moli_url::is_about_blank(&initial_url)
                && let Some(inherited) = opener_endpoint.and_then(|endpoint| {
                    self.window_access_origin_for_dispatch_scope(endpoint.dispatch_scope())
                })
            {
                inherited
            } else if storage_scope.origin() == "null" {
                super::window_security_tokens::WindowAccessOrigin::opaque(
                    initial_execution_context_owner,
                )
            } else {
                super::window_security_tokens::WindowAccessOrigin::from_serialized_origin(
                    storage_scope.origin().to_owned(),
                    None,
                )?
            };
        let mut initial_policy_container = creator_policy_container;
        inherit_lightweight_popup_opener_sandbox(
            &mut initial_policy_container.sandbox,
            opener_sandbox_policy,
        );
        let initial_document_state =
            LightweightPopupDocumentState::new(initial_base_url.clone(), initial_policy_container);
        let initial_resource_origin = initial_origin.serialized_origin();
        let session_storage_store = match opener {
            Some(opener) => self.cloned_lightweight_popup_session_storage_store(
                scope,
                opener,
                opener_child_handle,
                &storage_scope,
            ),
            None => new_shared_web_storage_store(),
        };
        self.lightweight_popup_browsing_contexts.insert(
            popup_id,
            LightweightPopupBrowsingContextRecord {
                window_proxy: v8::Global::new(scope, window),
                opener: opener_endpoint,
                location_url: initial_url.clone(),
                opener_sandbox_policy,
                lifecycle: LightweightPopupLifecycle::Open(Box::new(LightweightPopupOpenState {
                    document: LightweightPopupDocumentRecord {
                        owner: initial_document_owner,
                        local_window_id: initial_local_window_id,
                        url: initial_url.clone(),
                        access_origin: initial_origin,
                        state: initial_document_state.clone(),
                        storage_scope,
                        wrapper: None,
                        handle: None,
                        script_globals: HashMap::new(),
                        incomplete_child_frame_loads: HashSet::new(),
                        pending_load_event: None,
                        queued_load_event: None,
                        load_event_dispatched: false,
                    },
                    session_storage_store,
                })),
                navigation_id: LightweightPopupNavigationId::new(1),
            },
        );
        self.register_committed_document_resource_loader(
            crate::network::context::DocumentFetchContext::new(
                super::WindowDocumentOwner::LightweightPopup(initial_document_owner),
                initial_url.clone(),
                initial_base_url.clone(),
                initial_resource_origin,
            ),
            crate::network::context::DocumentResourceAuthoritySource::Inherited(
                creator_resource_authority,
            ),
        );
        self.refresh_lightweight_popup_indexed_db_factory(scope, popup_id, window);
        if matches!(initial_url.scheme(), "http" | "https") {
            let _ = self.register_or_update_service_worker_popup_client(
                initial_document_owner,
                initial_url.clone(),
            );
        }
        if let Some(name) = tracked_name {
            self.lightweight_popup_window_names.insert(name, popup_id);
        }
        if moli_url::is_about_blank(&initial_url)
            && let Some(document) =
                crate::dom_parser::parse_browsing_context_document_projection_from_source(
                    scope,
                    initial_url.clone(),
                    "<!doctype html><html><head></head><body></body></html>",
                    Some("text/html"),
                    None,
                    crate::parser::HtmlParser::with_scripting_enabled(
                        self.lightweight_popup_scripting_enabled(popup_id),
                    ),
                )
        {
            if let Some(document_handle) =
                self.remember_lightweight_popup_document_handle(scope, popup_id, document)
            {
                let _ = crate::context_bootstrap::install_css_runtime_state_for_document(
                    scope,
                    window,
                    Some(document_handle),
                );
                install_lightweight_popup_get_computed_style(scope, window, document_handle);
            }
            sync_lightweight_popup_document_window_slots(
                scope,
                document,
                window,
                &initial_base_url,
                &initial_referrer,
            );
            set_object_slot(scope, window, "document", document.into());
            let _ = self
                .set_lightweight_popup_document_wrapper(popup_id, v8::Global::new(scope, document));
        }
        let navigation_id = self
            .lightweight_popup_navigation_id(popup_id)
            .expect("newly inserted popup record must have a navigation id");
        let navigation_task =
            LightweightPopupNavigationTaskToken::from_parts(initial_document_owner, navigation_id);
        let queue_synthetic_load = if parsed_url.scheme() == "javascript" {
            let source = crate::javascript_url::decoded_source(&parsed_url);
            self.queue_lightweight_popup_javascript_url_task(scope, navigation_task, source);
            true
        } else if moli_url::is_about_blank(&initial_url) {
            false
        } else {
            self.start_lightweight_popup_document_load(
                navigation_task,
                initial_url.clone(),
                about_blank_url(),
                initial_document_state,
            )
            .is_none()
        };
        if queue_synthetic_load {
            self.queue_lightweight_popup_load_event(navigation_task);
        }
        Some((window, popup_id))
    }

    fn reopen_lightweight_popup_window<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        popup_id: u64,
        opener: Option<v8::Local<'s, v8::Object>>,
        opener_child_handle: Option<DomHandle>,
        href: &str,
        creator_base_url: Url,
        creator_policy_container: DocumentPolicyContainer,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let parsed_url = Url::parse(href).ok()?;
        let initial_base_url = lightweight_popup_initial_base_url(&parsed_url, creator_base_url);
        let window = self.lightweight_popup_window(scope, popup_id)?;
        let mut navigation_state =
            LightweightPopupDocumentState::new(initial_base_url, creator_policy_container);
        let opener_sandbox_policy = self
            .lightweight_popup_record(popup_id)
            .and_then(|record| record.opener_sandbox_policy);
        inherit_lightweight_popup_opener_sandbox(
            &mut navigation_state.policy_container.sandbox,
            opener_sandbox_policy,
        );
        let initiator_endpoint =
            lightweight_popup_initiator_endpoint(scope, opener, opener_child_handle);
        let previous_url =
            lightweight_popup_location_href(scope, window).unwrap_or_else(about_blank_url);
        let target_url = parsed_url;
        let document_target = if target_url.scheme() == "javascript" {
            LightweightPopupNavigationDocumentTarget::CurrentDocument
        } else {
            LightweightPopupNavigationDocumentTarget::NewDocument
        };
        let navigation_task = self.start_lightweight_popup_navigation_attempt(
            popup_id,
            target_url.clone(),
            document_target,
        )?;
        if target_url.scheme() == "javascript" {
            let source = crate::javascript_url::decoded_source(&target_url);
            self.queue_lightweight_popup_javascript_url_task(scope, navigation_task, source);
            return Some(window);
        }
        let document_owner = navigation_task.document_owner();
        self.apply_lightweight_popup_location_navigation(
            scope,
            popup_id,
            window,
            &target_url,
            crate::context_bootstrap::LocationNavigationKind::Assign,
            Some(&previous_url),
        );
        let queue_synthetic_load = if moli_url::is_about_blank(&target_url) {
            let storage_scope = self.lightweight_popup_storage_scope_for_initiated_navigation(
                scope,
                opener,
                opener_child_handle,
                &target_url,
                opener_sandbox_policy.is_some_and(|policy| policy.forces_opaque_origin),
            );
            navigation_state.reset_for_empty_document(target_url.clone());
            if !self.commit_lightweight_popup_document(
                scope,
                window,
                LightweightPopupDocumentCommit {
                    owner: document_owner,
                    location_url: target_url.clone(),
                    origin: if opener_sandbox_policy
                        .is_some_and(|policy| policy.forces_opaque_origin)
                    {
                        LightweightPopupDocumentCommitOrigin::FromNavigationResponse(
                            "null".to_owned(),
                        )
                    } else if let Some(inherited) = initiator_endpoint.and_then(|endpoint| {
                        self.window_access_origin_for_dispatch_scope(endpoint.dispatch_scope())
                    }) {
                        LightweightPopupDocumentCommitOrigin::InheritExact(inherited)
                    } else {
                        LightweightPopupDocumentCommitOrigin::FromNavigationResponse(
                            storage_scope.origin().to_owned(),
                        )
                    },
                    state: navigation_state,
                    storage_scope,
                    navigation_loader: None,
                },
            ) {
                return None;
            }
            if !self.lightweight_popup_committed_navigation_task_is_current(navigation_task) {
                return Some(window);
            }
            self.unregister_service_worker_popup_client(popup_id);
            self.install_lightweight_popup_empty_document(scope, popup_id, window, target_url);
            true
        } else {
            self.start_lightweight_popup_document_load(
                navigation_task,
                target_url,
                previous_url,
                navigation_state,
            )
            .is_none()
        };
        if queue_synthetic_load {
            if !self.lightweight_popup_committed_navigation_task_is_current(navigation_task) {
                return Some(window);
            }
            self.queue_lightweight_popup_load_event(navigation_task);
        }
        Some(window)
    }

    pub(crate) fn lightweight_popup_window<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        popup_id: u64,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.lightweight_popup_browsing_contexts
            .get(&popup_id)
            .map(|record| v8::Local::new(scope, &record.window_proxy))
    }

    pub(in crate::native_bridge::context_host) fn lightweight_popup_opener_endpoint(
        &self,
        popup_id: u64,
    ) -> Option<super::PendingWindowMessageEndpoint> {
        self.lightweight_popup_browsing_contexts
            .get(&popup_id)
            .and_then(|record| record.opener)
    }

    pub(crate) fn lightweight_popup_origin(&self, popup_id: u64) -> Option<String> {
        self.lightweight_popup_access_origin(popup_id)
            .map(|origin| origin.serialized_origin())
    }

    pub(in crate::native_bridge::context_host) fn lightweight_popup_access_origin(
        &self,
        popup_id: u64,
    ) -> Option<super::window_security_tokens::WindowAccessOrigin> {
        self.lightweight_popup_document_record(popup_id)
            .map(|document| document.access_origin.clone())
    }

    pub(crate) fn lightweight_popup_bound_web_storage_scope(
        &self,
        popup_id: u64,
    ) -> Option<super::child_frames::WebStorageScope> {
        self.lightweight_popup_document_record(popup_id)
            .map(|document| {
                let scope = &document.storage_scope;
                super::child_frames::WebStorageScope::from_parts(
                    scope.origin().to_owned(),
                    scope.area_key().to_owned(),
                    scope.storage_key().clone(),
                )
            })
    }

    fn lightweight_popup_location_url(&self, popup_id: u64) -> Option<Url> {
        self.lightweight_popup_browsing_contexts
            .get(&popup_id)
            .map(|record| record.location_url.clone())
    }

    pub(crate) fn lightweight_popup_document_url(&self, popup_id: u64) -> Option<Url> {
        self.lightweight_popup_document_record(popup_id)
            .map(|document| document.url.clone())
    }

    pub(crate) fn lightweight_popup_session_storage_store(
        &self,
        popup_id: u64,
    ) -> Option<SharedWebStorageStore> {
        match &self.lightweight_popup_record(popup_id)?.lifecycle {
            LightweightPopupLifecycle::Open(open) => Some(open.session_storage_store.clone()),
            LightweightPopupLifecycle::Closed => None,
        }
    }

    pub(crate) fn lightweight_popup_initial_empty_document_storage_key(
        &self,
        popup_id: u64,
    ) -> Option<MoliStorageKey> {
        let document = self.lightweight_popup_document_record(popup_id)?;
        moli_url::is_about_blank(&document.url)
            .then(|| document.storage_scope.storage_key().clone())
    }

    fn cloned_lightweight_popup_session_storage_store<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        opener: v8::Local<'s, v8::Object>,
        opener_child_handle: Option<DomHandle>,
        target_scope: &LightweightPopupStorageScope,
    ) -> SharedWebStorageStore {
        let opener_popup_id = lightweight_popup_id_from_window(scope, opener);
        let source_store = if let Some(popup_id) = opener_popup_id {
            let Some(store) = self.lightweight_popup_session_storage_store(popup_id) else {
                return new_shared_web_storage_store();
            };
            store
        } else {
            self.session_storage_store.clone()
        };
        let target_store = deep_clone_shared_web_storage_store(&source_store);
        let Some(source_scope) =
            self.lightweight_popup_opener_storage_scope(scope, Some(opener), opener_child_handle)
        else {
            return target_store;
        };
        if source_scope.origin() != target_scope.origin()
            || source_scope.area_key() == target_scope.area_key()
        {
            return target_store;
        }
        let entries = {
            let mut source = source_store.lock();
            source
                .sorted_keys_utf16(source_scope.area_key())
                .into_iter()
                .filter_map(|key| {
                    source
                        .get_item_utf16(source_scope.area_key(), &key)
                        .map(|value| (key, value))
                })
                .collect::<Vec<_>>()
        };
        {
            let mut target = target_store.lock();
            for (key, value) in entries {
                let _ = target.set_item_utf16(target_scope.area_key(), &key, &value);
            }
        }
        target_store
    }

    fn lightweight_popup_storage_scope_for_initiated_navigation<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        opener: Option<v8::Local<'s, v8::Object>>,
        opener_child_handle: Option<DomHandle>,
        target_url: &Url,
        sandbox_forces_opaque_origin: bool,
    ) -> LightweightPopupStorageScope {
        if sandbox_forces_opaque_origin {
            return self.lightweight_popup_opaque_storage_scope(target_url);
        }
        if moli_url::is_about_blank(target_url)
            && let Some(opener_scope) =
                self.lightweight_popup_opener_storage_scope(scope, opener, opener_child_handle)
        {
            if opener_scope.origin() == "null" {
                return opener_scope;
            }
            if opener_child_handle.is_none() {
                return opener_scope;
            }
            let storage_key = web_storage_key_for_child_about_blank_popup(&opener_scope);
            let area_key = web_storage_area_key_for_storage_key(&storage_key);
            return LightweightPopupStorageScope::new(
                opener_scope.origin().to_owned(),
                area_key,
                storage_key,
            );
        }
        LightweightPopupStorageScope::from_web_storage_scope(
            self.web_storage_scope_for_url_as_first_party(target_url),
        )
    }

    fn lightweight_popup_response_storage_scope(
        &mut self,
        final_url: &Url,
        response_content_security_policies: &[String],
        sandbox_forces_opaque_origin: bool,
    ) -> LightweightPopupStorageScope {
        if sandbox_forces_opaque_origin
            || content_security_policy_forces_opaque_origin(response_content_security_policies)
        {
            return self.lightweight_popup_opaque_storage_scope(final_url);
        }
        LightweightPopupStorageScope::from_web_storage_scope(
            self.web_storage_scope_for_url_as_first_party(final_url),
        )
    }

    fn lightweight_popup_opaque_storage_scope(
        &mut self,
        url: &Url,
    ) -> LightweightPopupStorageScope {
        let top_level_site = site_for_url(self.document_url());
        let relation = StoragePartitionRelation::from_sites(&site_for_url(url), &top_level_site);
        let storage_key = MoliStorageKey::new(
            "null".to_owned(),
            top_level_site,
            Some(
                self.browser_context_runtime
                    .next_web_storage_opaque_context_nonce(),
            ),
            relation,
        );
        LightweightPopupStorageScope::new(
            "null".to_owned(),
            web_storage_area_key_for_storage_key(&storage_key),
            storage_key,
        )
    }

    fn lightweight_popup_opener_storage_scope<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        opener: Option<v8::Local<'s, v8::Object>>,
        opener_child_handle: Option<DomHandle>,
    ) -> Option<LightweightPopupStorageScope> {
        if let Some(opener) = opener
            && let Some(popup_id) = lightweight_popup_id_from_window(scope, opener)
        {
            return self
                .lightweight_popup_bound_web_storage_scope(popup_id)
                .map(LightweightPopupStorageScope::from_web_storage_scope);
        }
        if let Some(handle) = opener_child_handle {
            let top_origin = origin_ascii_serialization(self.document_url());
            return self
                .child_browsing_context_web_storage_scope(handle, &top_origin)
                .map(LightweightPopupStorageScope::from_web_storage_scope);
        }
        Some(LightweightPopupStorageScope::from_web_storage_scope(
            self.top_web_storage_scope(),
        ))
    }

    pub(crate) fn active_lightweight_popup_base_url(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> Option<Url> {
        active_lightweight_popup_id(scope)
            .and_then(|popup_id| self.lightweight_popup_base_url(scope, popup_id))
    }

    pub(crate) fn lightweight_popup_request_base_url(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        popup_id: u64,
    ) -> Option<Url> {
        self.lightweight_popup_base_url(scope, popup_id)
    }

    pub(crate) fn active_lightweight_popup_content_security_policies(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> Option<&[String]> {
        let popup_id = active_lightweight_popup_id(scope)?;
        Some(
            self.lightweight_popup_document_record(popup_id)?
                .state
                .policy_container
                .document_content_security_policies
                .as_slice(),
        )
    }

    pub(crate) fn active_lightweight_popup_policy_container(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> Option<&DocumentPolicyContainer> {
        let popup_id = active_lightweight_popup_id(scope)?;
        self.lightweight_popup_document_record(popup_id)
            .map(|document| &document.state.policy_container)
    }

    pub(crate) fn lightweight_popup_policy_container(
        &self,
        popup_id: u64,
    ) -> Option<&DocumentPolicyContainer> {
        self.lightweight_popup_document_record(popup_id)
            .map(|document| &document.state.policy_container)
    }

    pub(crate) fn active_lightweight_popup_referrer_policy(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> Option<&str> {
        self.active_lightweight_popup_policy_container(scope)
            .and_then(|policy| policy.referrer_policy.as_deref())
    }

    pub(crate) fn lightweight_popup_referrer_policy(&self, popup_id: u64) -> Option<&str> {
        self.lightweight_popup_document_record(popup_id)
            .and_then(|document| document.state.policy_container.referrer_policy.as_deref())
    }

    pub(crate) fn lightweight_popup_cross_origin_embedder_policy(
        &self,
        popup_id: u64,
    ) -> crate::cross_origin_isolation::CrossOriginEmbedderPolicy {
        self.lightweight_popup_document_record(popup_id)
            .map(|document| document.state.policy_container.cross_origin_embedder_policy)
            .unwrap_or_default()
    }

    pub(crate) fn lightweight_popup_document_isolation_policy(
        &self,
        popup_id: u64,
    ) -> crate::cross_origin_isolation::DocumentIsolationPolicy {
        self.lightweight_popup_document_record(popup_id)
            .map(|document| document.state.policy_container.document_isolation_policy)
            .unwrap_or_default()
    }

    pub(crate) fn lightweight_popup_cross_origin_isolated(&self, popup_id: u64) -> bool {
        self.lightweight_popup_document_record(popup_id)
            .is_some_and(|document| document.state.policy_container.cross_origin_isolated)
    }

    fn lightweight_popup_base_url(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        popup_id: u64,
    ) -> Option<Url> {
        self.lightweight_popup_document_record(popup_id)
            .map(|document| document.state.base_url.clone())
            .or_else(|| {
                self.lightweight_popup_window(scope, popup_id)
                    .and_then(|window| lightweight_popup_location_href(scope, window))
            })
            .or_else(|| self.lightweight_popup_location_url(popup_id))
    }

    fn apply_lightweight_popup_location_navigation<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        popup_id: u64,
        window: v8::Local<'s, v8::Object>,
        target_url: &Url,
        kind: crate::context_bootstrap::LocationNavigationKind,
        current_url: Option<&Url>,
    ) {
        let effective_kind = lightweight_popup_effective_navigation_kind(current_url, kind);
        let base_url = self
            .lightweight_popup_base_url(scope, popup_id)
            .unwrap_or_else(|| target_url.clone());
        let document_referrer = self
            .lightweight_popup_document_record(popup_id)
            .map(|document| document.state.policy_container.document_referrer.clone())
            .unwrap_or_default();
        sync_lightweight_popup_window_location(
            scope,
            window,
            target_url.as_str(),
            &base_url,
            &document_referrer,
        );
        apply_local_window_location_navigation(scope, window, target_url, effective_kind);
        sync_window_location_history_navigation_runtime_surface(scope, window);
    }

    pub(crate) fn has_pending_lightweight_popup_document_loads(&self) -> bool {
        !self.pending_lightweight_popup_document_loads.is_empty()
    }

    pub(crate) fn has_pending_lightweight_popup_resource_loads(&self) -> bool {
        self.has_pending_lightweight_popup_document_loads()
            || !self
                .pending_lightweight_popup_classic_script_loads
                .is_empty()
    }

    pub(crate) fn lightweight_popup_has_pending_document_load(&self, popup_id: u64) -> bool {
        self.pending_lightweight_popup_document_loads
            .values()
            .any(|pending| pending.target.task().popup_id() == popup_id)
    }

    pub(crate) fn lightweight_popup_is_open(&self, popup_id: u64) -> bool {
        self.lightweight_popup_record(popup_id)
            .is_some_and(LightweightPopupBrowsingContextRecord::is_open)
    }

    pub(crate) fn open_lightweight_popup_ids(&self) -> Vec<u64> {
        let mut popup_ids = self
            .lightweight_popup_browsing_contexts
            .iter()
            .filter_map(|(popup_id, record)| record.is_open().then_some(*popup_id))
            .collect::<Vec<_>>();
        popup_ids.sort_unstable();
        popup_ids
    }

    pub(crate) fn navigate_lightweight_popup_window_to_url(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        popup_id: u64,
        target_url: Url,
        kind: crate::context_bootstrap::LocationNavigationKind,
    ) -> bool {
        if !self.lightweight_popup_is_open(popup_id) {
            return false;
        }
        let Some(window) = self.lightweight_popup_window(scope, popup_id) else {
            return false;
        };
        let current_url = lightweight_popup_location_href(scope, window)
            .or_else(|| self.lightweight_popup_location_url(popup_id));
        if !matches!(
            kind,
            crate::context_bootstrap::LocationNavigationKind::Reload
        ) && urls_refer_to_same_document_except_fragment(current_url.as_ref(), &target_url)
        {
            let previous_url = current_url.clone();
            let base_url = self
                .lightweight_popup_base_url(scope, popup_id)
                .unwrap_or_else(|| target_url.clone());
            let document_referrer = self
                .lightweight_popup_document_record(popup_id)
                .map(|document| document.state.policy_container.document_referrer.clone())
                .unwrap_or_default();
            let _ = self.set_lightweight_popup_same_document_url(popup_id, target_url.clone());
            sync_lightweight_popup_window_location(
                scope,
                window,
                target_url.as_str(),
                &base_url,
                &document_referrer,
            );
            if let Some(previous_url) = previous_url {
                self.dispatch_lightweight_popup_same_document_navigation_events(
                    scope,
                    popup_id,
                    window,
                    previous_url.as_str(),
                    target_url.as_str(),
                );
            }
            return true;
        }
        if !matches!(
            kind,
            crate::context_bootstrap::LocationNavigationKind::Reload
        ) && let Some(previous_url) =
            self.pending_lightweight_popup_same_document_previous_url(popup_id, &target_url)
        {
            let base_url = self
                .lightweight_popup_base_url(scope, popup_id)
                .unwrap_or_else(|| target_url.clone());
            let document_referrer = self
                .lightweight_popup_document_record(popup_id)
                .map(|document| document.state.policy_container.document_referrer.clone())
                .unwrap_or_default();
            if let Some(record) = self.lightweight_popup_record_mut(popup_id) {
                record.location_url = target_url.clone();
            }
            self.update_pending_lightweight_popup_previous_urls(popup_id, target_url.clone());
            sync_lightweight_popup_window_location(
                scope,
                window,
                target_url.as_str(),
                &base_url,
                &document_referrer,
            );
            self.dispatch_lightweight_popup_same_document_navigation_events(
                scope,
                popup_id,
                window,
                previous_url.as_str(),
                target_url.as_str(),
            );
            return true;
        }

        let previous_url = current_url.clone().unwrap_or_else(about_blank_url);
        if target_url.scheme() == "javascript" {
            let owner = self.entered_owner_dispatch_scope(scope);
            let csp_source = crate::javascript_url::csp_source(&target_url);
            if !self.allows_inline_javascript_navigation_by_csp(scope, owner, &csp_source) {
                return true;
            }
            let Some(navigation_task) = self.start_lightweight_popup_navigation_attempt(
                popup_id,
                target_url.clone(),
                LightweightPopupNavigationDocumentTarget::CurrentDocument,
            ) else {
                return false;
            };
            let source = crate::javascript_url::decoded_source(&target_url);
            self.queue_lightweight_popup_javascript_url_task(scope, navigation_task, source);
            return true;
        }
        let Some((mut navigation_state, storage_scope)) =
            self.lightweight_popup_document_commit_seed(popup_id)
        else {
            return false;
        };
        let Some(navigation_task) = self.start_lightweight_popup_navigation_attempt(
            popup_id,
            target_url.clone(),
            LightweightPopupNavigationDocumentTarget::NewDocument,
        ) else {
            return false;
        };
        let document_owner = navigation_task.document_owner();
        if moli_url::is_about_blank(&target_url) {
            navigation_state.reset_for_empty_document(target_url.clone());
            let inherited_origin = self
                .window_access_origin_for_dispatch_scope(self.entered_owner_dispatch_scope(scope));
            if !self.commit_lightweight_popup_document(
                scope,
                window,
                LightweightPopupDocumentCommit {
                    owner: document_owner,
                    location_url: target_url.clone(),
                    origin: inherited_origin.map_or(
                        LightweightPopupDocumentCommitOrigin::RetainCurrent,
                        LightweightPopupDocumentCommitOrigin::InheritExact,
                    ),
                    state: navigation_state,
                    storage_scope,
                    navigation_loader: None,
                },
            ) {
                return false;
            }
            if !self.lightweight_popup_committed_navigation_task_is_current(navigation_task) {
                return true;
            }
            self.unregister_service_worker_popup_client(popup_id);
            self.apply_lightweight_popup_location_navigation(
                scope,
                popup_id,
                window,
                &target_url,
                kind,
                current_url.as_ref(),
            );
            self.install_lightweight_popup_empty_document(scope, popup_id, window, target_url);
            self.queue_lightweight_popup_load_event(navigation_task);
            return true;
        }

        self.apply_lightweight_popup_location_navigation(
            scope,
            popup_id,
            window,
            &target_url,
            kind,
            current_url.as_ref(),
        );
        if self
            .start_lightweight_popup_document_load(
                navigation_task,
                target_url,
                previous_url,
                navigation_state,
            )
            .is_none()
        {
            self.queue_lightweight_popup_load_event(navigation_task);
        }
        true
    }

    pub(crate) fn queue_lightweight_popup_cross_document_traversal(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        popup_id: u64,
        target_url: &str,
        entry_seed: NavigationHistoryEntrySeed,
    ) -> bool {
        if !self.lightweight_popup_is_open(popup_id) {
            return false;
        }
        let Some(window) = self.lightweight_popup_window(scope, popup_id) else {
            return false;
        };
        let Some(target_url) = Url::parse(target_url).ok() else {
            return false;
        };
        let previous_url = lightweight_popup_location_href(scope, window)
            .or_else(|| self.lightweight_popup_location_url(popup_id))
            .unwrap_or_else(about_blank_url);
        let Some((mut navigation_state, storage_scope)) =
            self.lightweight_popup_document_commit_seed(popup_id)
        else {
            return false;
        };
        let Some(navigation_task) = self.start_lightweight_popup_navigation_attempt(
            popup_id,
            target_url.clone(),
            LightweightPopupNavigationDocumentTarget::NewDocument,
        ) else {
            return false;
        };
        let document_owner = navigation_task.document_owner();
        install_navigation_bootstrap_entry_for_holder(scope, window, &entry_seed);
        let base_url = self
            .lightweight_popup_base_url(scope, popup_id)
            .unwrap_or_else(|| target_url.clone());
        let document_referrer = navigation_state.policy_container.document_referrer.clone();
        sync_lightweight_popup_window_location(
            scope,
            window,
            target_url.as_str(),
            &base_url,
            &document_referrer,
        );
        if moli_url::is_about_blank(&target_url) {
            navigation_state.reset_for_empty_document(target_url.clone());
            if !self.commit_lightweight_popup_document(
                scope,
                window,
                LightweightPopupDocumentCommit {
                    owner: document_owner,
                    location_url: target_url.clone(),
                    // Traversal history does not yet persist an exact origin
                    // snapshot for lightweight popups. Keep the current exact
                    // origin instead of reconstructing one from about:blank.
                    origin: LightweightPopupDocumentCommitOrigin::RetainCurrent,
                    state: navigation_state,
                    storage_scope,
                    navigation_loader: None,
                },
            ) {
                return false;
            }
            if !self.lightweight_popup_committed_navigation_task_is_current(navigation_task) {
                return true;
            }
            self.unregister_service_worker_popup_client(popup_id);
            self.install_lightweight_popup_empty_document(scope, popup_id, window, target_url);
            self.queue_lightweight_popup_load_event(navigation_task);
            return true;
        }
        if self
            .start_lightweight_popup_document_load(
                navigation_task,
                target_url,
                previous_url,
                navigation_state,
            )
            .is_none()
        {
            self.queue_lightweight_popup_load_event(navigation_task);
        }
        true
    }

    pub(crate) fn lightweight_popup_document_is_open(
        &self,
        document_handle: crate::document_runtime::DomHandle,
    ) -> bool {
        self.lightweight_popup_id_for_document_handle(document_handle)
            .is_some()
    }

    pub(crate) fn current_lightweight_popup_document_owner(
        &self,
        popup_id: u64,
    ) -> Option<LightweightPopupDocumentOwner> {
        self.lightweight_popup_document_record(popup_id)
            .map(|document| document.owner)
    }

    pub(crate) fn lightweight_popup_document_owner_is_current(
        &self,
        owner: LightweightPopupDocumentOwner,
    ) -> bool {
        self.current_lightweight_popup_document_owner(owner.popup_id()) == Some(owner)
    }

    fn lightweight_popup_navigation_attempt_is_current(
        &self,
        task: LightweightPopupNavigationTaskToken,
    ) -> bool {
        self.lightweight_popup_is_open(task.popup_id())
            && self.lightweight_popup_navigation_id(task.popup_id()) == Some(task.navigation_id)
    }

    fn lightweight_popup_committed_navigation_task_is_current(
        &self,
        task: LightweightPopupNavigationTaskToken,
    ) -> bool {
        self.lightweight_popup_navigation_attempt_is_current(task)
            && self.lightweight_popup_document_owner_is_current(task.document_owner)
    }

    pub(crate) fn current_lightweight_popup_local_window_id(
        &self,
        popup_id: u64,
    ) -> Option<LightweightPopupLocalWindowId> {
        self.lightweight_popup_document_record(popup_id)
            .map(|document| document.local_window_id)
    }

    fn allocate_lightweight_popup_local_window_id(&mut self) -> LightweightPopupLocalWindowId {
        let id = LightweightPopupLocalWindowId::new(self.next_lightweight_popup_local_window_id);
        self.next_lightweight_popup_local_window_id = self
            .next_lightweight_popup_local_window_id
            .checked_add(1)
            .expect("lightweight popup LocalWindow id overflow");
        id
    }

    fn allocate_lightweight_popup_document_owner(
        &mut self,
        popup_id: u64,
    ) -> LightweightPopupDocumentOwner {
        let document_id = LightweightPopupDocumentId::new(self.next_lightweight_popup_document_id);
        self.next_lightweight_popup_document_id = self
            .next_lightweight_popup_document_id
            .wrapping_add(1)
            .max(1);
        LightweightPopupDocumentOwner::new(popup_id, document_id)
    }

    fn lightweight_popup_document_commit_seed(
        &self,
        popup_id: u64,
    ) -> Option<(LightweightPopupDocumentState, LightweightPopupStorageScope)> {
        let document = self.lightweight_popup_document_record(popup_id)?;
        Some((document.state.clone(), document.storage_scope.clone()))
    }

    fn commit_lightweight_popup_document<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        window: v8::Local<'s, v8::Object>,
        commit: LightweightPopupDocumentCommit,
    ) -> bool {
        let popup_id = commit.owner.popup_id();
        let committed_document_url = commit.location_url.clone();
        let committed_base_url = commit.state.base_url.clone();
        let Some(current_document) = self.lightweight_popup_document_record(popup_id) else {
            return false;
        };
        let changed_document = current_document.owner != commit.owner;
        let inherited_resource_authority = (changed_document && commit.navigation_loader.is_none())
            .then(|| {
                self.document_resource_loader_for_window_owner(
                    super::WindowDocumentOwner::LightweightPopup(current_document.owner),
                )
                .expect("synthetic popup Document requires its previous exact authority")
                .clone()
            });
        let retained_access_origin = current_document.access_origin.clone();
        let current_local_window_id = if changed_document {
            self.allocate_lightweight_popup_local_window_id()
        } else {
            current_document.local_window_id
        };
        let execution_context_owner = super::WindowExecutionContextOwner::LightweightPopup {
            popup_id,
            local_window_id: current_local_window_id,
        };
        let access_origin = match commit.origin {
            LightweightPopupDocumentCommitOrigin::RetainCurrent => retained_access_origin,
            LightweightPopupDocumentCommitOrigin::InheritExact(origin) => origin,
            LightweightPopupDocumentCommitOrigin::FromNavigationResponse(serialized_origin) => {
                if serialized_origin == "null" {
                    super::window_security_tokens::WindowAccessOrigin::opaque(
                        execution_context_owner,
                    )
                } else {
                    let Some(access_origin) =
                        super::window_security_tokens::WindowAccessOrigin::from_serialized_origin(
                            serialized_origin,
                            commit.state.document_domain_override.clone(),
                        )
                    else {
                        return false;
                    };
                    access_origin
                }
            }
        };
        let resource_context_origin = access_origin.serialized_origin();
        let transition = {
            let Some(record) = self.lightweight_popup_record_mut(popup_id) else {
                return false;
            };
            let LightweightPopupLifecycle::Open(open) = &mut record.lifecycle else {
                return false;
            };
            let retired_document = std::mem::replace(
                &mut open.document,
                LightweightPopupDocumentRecord {
                    owner: commit.owner,
                    local_window_id: current_local_window_id,
                    url: commit.location_url.clone(),
                    access_origin,
                    state: commit.state,
                    storage_scope: commit.storage_scope,
                    wrapper: None,
                    handle: None,
                    script_globals: HashMap::new(),
                    incomplete_child_frame_loads: HashSet::new(),
                    pending_load_event: None,
                    queued_load_event: None,
                    load_event_dispatched: false,
                },
            );
            record.location_url = commit.location_url;
            LightweightPopupDocumentTransition {
                retired_owner: changed_document.then_some(retired_document.owner),
                retired_local_window_id: changed_document
                    .then_some(retired_document.local_window_id),
                retired_document_handle: retired_document.handle,
                current_local_window_id,
            }
        };
        if changed_document {
            clear_lightweight_popup_window_document_event_state(scope, window);
        }
        if let Some(retired_document_handle) = transition.retired_document_handle {
            self.retire_lightweight_popup_document_handle(popup_id, retired_document_handle);
            self.clear_custom_element_registry_associations_for_document(retired_document_handle);
        }
        if let Some(retired_owner) = transition.retired_owner {
            self.retire_lightweight_popup_document_owner(retired_owner);
        }
        if changed_document {
            let resource_authority = match commit.navigation_loader {
                Some(loader) => {
                    let seed = loader.commit(committed_document_url.clone()).expect(
                        "successful popup navigation must commit its exact resource loader",
                    );
                    crate::network::context::DocumentResourceAuthoritySource::Navigation(seed)
                }
                None => crate::network::context::DocumentResourceAuthoritySource::Inherited(
                    inherited_resource_authority
                        .expect("synthetic popup Document must capture its previous authority"),
                ),
            };
            self.register_committed_document_resource_loader(
                crate::network::context::DocumentFetchContext::new(
                    super::WindowDocumentOwner::LightweightPopup(commit.owner),
                    committed_document_url,
                    committed_base_url,
                    resource_context_origin,
                ),
                resource_authority,
            );
        }
        if let Some(retired_local_window_id) = transition.retired_local_window_id {
            self.retire_lightweight_popup_local_window(popup_id, retired_local_window_id);
        }
        self.refresh_lightweight_popup_indexed_db_factory(scope, popup_id, window);
        tracing::debug!(
            retired_owner = ?transition.retired_owner,
            owner = ?commit.owner,
            retired_local_window_id = ?transition.retired_local_window_id,
            current_local_window_id = ?transition.current_local_window_id,
            "committed lightweight popup document and LocalWindow owner"
        );
        true
    }

    fn retire_lightweight_popup_document_owner(&mut self, owner: LightweightPopupDocumentOwner) {
        let _ = self
            .retire_document_resource_loader(super::WindowDocumentOwner::LightweightPopup(owner));
        self.finish_service_worker_clients_open_window_popup_with_null_for_owner(owner);
        self.retire_service_worker_window_document_owner(
            super::WindowDocumentOwner::LightweightPopup(owner),
        );
    }

    fn retire_lightweight_popup_local_window(
        &mut self,
        popup_id: u64,
        local_window_id: LightweightPopupLocalWindowId,
    ) {
        let execution_context_owner = super::WindowExecutionContextOwner::LightweightPopup {
            popup_id,
            local_window_id,
        };
        let retired_timer_count = unsafe { &mut *self.runtime }
            .cancel_window_execution_context_timers(execution_context_owner);
        let retired_webcrypto_count =
            self.retire_webcrypto_execution_context_owner(execution_context_owner);
        self.retire_opfs_execution_context_owner(execution_context_owner);
        let retired_worker_count =
            self.retire_workers_for_execution_context_owner(execution_context_owner);
        let retired_shared_worker_count = self
            .disconnect_shared_worker_clients_for_execution_context_owner(execution_context_owner);
        let retired_xhr_count =
            self.retire_window_xhrs_for_execution_context_owner(execution_context_owner);
        let retired_fetch_count =
            self.retire_window_fetches_for_execution_context_owner(execution_context_owner);
        self.retire_window_event_sources_for_execution_context_owner(execution_context_owner);
        let retired_window_message_count =
            self.retire_window_messages_for_execution_context_owner(execution_context_owner);
        let retired_window_execution_context =
            self.retire_window_execution_context(execution_context_owner);
        let retired_broadcast_channel_count =
            self.close_broadcast_channels_for_execution_context_owner(execution_context_owner);
        let retired_websocket_count =
            self.retire_websockets_for_execution_context_owner(execution_context_owner);
        let retired_image_decode_count =
            self.retire_image_decode_requests_for_execution_context_owner(execution_context_owner);
        let retired_message_port_count =
            self.retire_message_ports_for_execution_context_owner(execution_context_owner);
        tracing::debug!(
            ?execution_context_owner,
            retired_timer_count,
            retired_webcrypto_count,
            retired_worker_count,
            retired_shared_worker_count,
            retired_xhr_count,
            aborted_fetch_count = retired_fetch_count.0,
            detached_keepalive_fetch_count = retired_fetch_count.1,
            retired_window_execution_context,
            retired_broadcast_channel_count,
            retired_websocket_count,
            retired_image_decode_count,
            retired_message_port_count,
            retired_window_message_count,
            "retired lightweight popup runtime objects with LocalWindow"
        );
    }

    fn refresh_lightweight_popup_indexed_db_factory<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        popup_id: u64,
        window: v8::Local<'s, v8::Object>,
    ) {
        let dispatch_scope = super::OwnerDispatchScope::LightweightPopup(popup_id);
        let storage_scope = self
            .lightweight_popup_document_record(popup_id)
            .map(|document| document.storage_scope.clone());
        let execution_context = self
            .register_lightweight_popup_execution_context(scope, popup_id)
            .then(|| self.current_registered_window_execution_context_identity(dispatch_scope))
            .flatten();
        let (Some(storage_scope), Some(execution_context)) = (storage_scope, execution_context)
        else {
            // Never retain a factory stamped for the popup's retired LocalWindow.
            set_object_slot(scope, window, "indexedDB", v8::undefined(scope).into());
            tracing::warn!(
                popup_id,
                "failed to bind popup IndexedDB to its exact LocalWindow"
            );
            return;
        };
        if !install_lightweight_popup_indexed_db_factory(
            scope,
            window,
            &storage_scope,
            execution_context,
        ) {
            set_object_slot(scope, window, "indexedDB", v8::undefined(scope).into());
            tracing::warn!(popup_id, "failed to install popup IndexedDB factory");
        }
    }

    fn start_lightweight_popup_navigation_attempt(
        &mut self,
        popup_id: u64,
        target_url: Url,
        document_target: LightweightPopupNavigationDocumentTarget,
    ) -> Option<LightweightPopupNavigationTaskToken> {
        let current_owner = self.current_lightweight_popup_document_owner(popup_id)?;
        let document_owner = match document_target {
            LightweightPopupNavigationDocumentTarget::CurrentDocument => current_owner,
            LightweightPopupNavigationDocumentTarget::NewDocument => {
                self.allocate_lightweight_popup_document_owner(popup_id)
            }
        };
        self.finish_service_worker_clients_open_window_popup_with_null_for_owner(current_owner);
        self.cancel_lightweight_popup_document_loads(popup_id);
        self.cancel_lightweight_popup_classic_script_loads(popup_id);
        let record = self
            .lightweight_popup_record_mut(popup_id)
            .expect("current popup document owner must belong to an open browsing context");
        record.navigation_id = LightweightPopupNavigationId::new(
            record
                .navigation_id
                .as_u64()
                .checked_add(1)
                .expect("lightweight popup navigation id space exhausted"),
        );
        record.location_url = target_url;
        Some(LightweightPopupNavigationTaskToken::from_parts(
            document_owner,
            record.navigation_id,
        ))
    }

    fn cancel_lightweight_popup_document_loads(&mut self, popup_id: u64) {
        let canceled_loads = self
            .pending_lightweight_popup_document_loads
            .iter()
            .filter_map(|(load_id, pending)| {
                (pending.target.task().popup_id() == popup_id).then_some(*load_id)
            })
            .collect::<Vec<_>>();
        for load_id in canceled_loads {
            self.pending_lightweight_popup_document_loads
                .remove(&load_id);
        }
    }

    fn cancel_lightweight_popup_classic_script_loads(&mut self, popup_id: u64) {
        let canceled_loads = self
            .pending_lightweight_popup_classic_script_loads
            .iter()
            .filter_map(|(load_id, pending)| {
                (pending.target.task().popup_id() == popup_id).then_some(*load_id)
            })
            .collect::<Vec<_>>();
        for load_id in canceled_loads {
            if let Some(pending) = self
                .pending_lightweight_popup_classic_script_loads
                .remove(&load_id)
            {
                pending.cancel_handle.cancel();
            }
        }
    }

    fn pending_lightweight_popup_same_document_previous_url(
        &self,
        popup_id: u64,
        target_url: &Url,
    ) -> Option<Url> {
        self.pending_lightweight_popup_document_loads
            .values()
            .find_map(|pending| {
                (pending.target.task().popup_id() == popup_id
                    && urls_refer_to_same_document_except_fragment(
                        Some(&pending.previous_url),
                        target_url,
                    ))
                .then(|| pending.previous_url.clone())
            })
    }

    fn update_pending_lightweight_popup_previous_urls(&mut self, popup_id: u64, target_url: Url) {
        for pending in self.pending_lightweight_popup_document_loads.values_mut() {
            if pending.target.task().popup_id() == popup_id {
                pending.previous_url = target_url.clone();
            }
        }
    }

    fn install_lightweight_popup_empty_document<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        popup_id: u64,
        window: v8::Local<'s, v8::Object>,
        document_url: Url,
    ) {
        clear_lightweight_popup_window_document_event_state(scope, window);
        self.clear_lightweight_popup_document_projection(popup_id);
        let base_url = self
            .lightweight_popup_base_url(scope, popup_id)
            .unwrap_or_else(|| document_url.clone());
        let document_referrer =
            if let Some(document) = self.lightweight_popup_document_record_mut(popup_id) {
                document.state.base_url = base_url.clone();
                document.state.policy_container.document_referrer.clone()
            } else {
                String::new()
            };
        let Some(document) =
            crate::dom_parser::parse_browsing_context_document_projection_from_source(
                scope,
                document_url,
                "<!doctype html><html><head></head><body></body></html>",
                Some("text/html"),
                None,
                crate::parser::HtmlParser::with_scripting_enabled(
                    self.lightweight_popup_scripting_enabled(popup_id),
                ),
            )
        else {
            return;
        };
        if let Some(document_handle) =
            self.remember_lightweight_popup_document_handle(scope, popup_id, document)
        {
            let _ = crate::context_bootstrap::install_css_runtime_state_for_document(
                scope,
                window,
                Some(document_handle),
            );
            install_lightweight_popup_get_computed_style(scope, window, document_handle);
        }
        sync_lightweight_popup_document_window_slots(
            scope,
            document,
            window,
            &base_url,
            &document_referrer,
        );
        set_object_slot(scope, window, "document", document.into());
        let _ =
            self.set_lightweight_popup_document_wrapper(popup_id, v8::Global::new(scope, document));
    }

    fn start_lightweight_popup_document_load(
        &mut self,
        task: LightweightPopupNavigationTaskToken,
        target_url: Url,
        previous_url: Url,
        document_state: LightweightPopupDocumentState,
    ) -> Option<u64> {
        let popup_id = task.popup_id();
        if !self.lightweight_popup_navigation_attempt_is_current(task) {
            return None;
        }
        let load_id = self.next_lightweight_popup_document_load_id;
        self.next_lightweight_popup_document_load_id =
            self.next_lightweight_popup_document_load_id.wrapping_add(1);
        let target = LightweightPopupDocumentFetchTarget::new(load_id, task);
        let local_snapshot = self.materialize_local_child_snapshot_for_url(&target_url);
        let resource_loader = if local_snapshot.is_none() {
            let source_owner = self.current_lightweight_popup_document_owner(popup_id)?;
            let initiating_loader = self.document_resource_loader_for_window_owner(
                super::WindowDocumentOwner::LightweightPopup(source_owner),
            )?;
            Some(crate::network::navigation::NavigationResourceLoader::new(
                initiating_loader.request_client().clone(),
                target_url.clone(),
                initiating_loader.task_runner(),
            ))
        } else {
            None
        };
        self.pending_lightweight_popup_document_loads.insert(
            load_id,
            PendingLightweightPopupDocumentLoad {
                target_url: target_url.clone(),
                previous_url,
                target,
                document_state: document_state.clone(),
                resource_loader: resource_loader.clone(),
            },
        );
        if let Some(snapshot) = local_snapshot {
            let mut policy_container = snapshot.policy_container.clone();
            if policy_container
                .document_content_security_policies
                .is_empty()
            {
                policy_container.document_content_security_policies =
                    policy_container.response_content_security_policies.clone();
            }
            let loaded = LoadedChildDocument {
                final_url: snapshot.url,
                policy_container,
                content_type: snapshot.content_type,
                character_set: snapshot.character_set,
                markup: snapshot.markup,
                document_network: None,
            };
            let completion = PopupDocumentLoadCompletion::new(
                target,
                Ok(PopupDocumentLoadOutcome::Loaded(Box::new(loaded))),
            );
            if self
                .resource_completion_tx
                .send_popup_document(completion)
                .is_ok()
            {
                return Some(load_id);
            }
            self.pending_lightweight_popup_document_loads
                .remove(&load_id);
            return None;
        }
        let resource_loader =
            resource_loader.expect("remote popup navigation requires its captured loader");
        let completion_tx = self.resource_completion_tx.clone();
        let opener_character_set = self.document_character_set().to_owned();
        let task_resource_loader = resource_loader.clone();
        resource_loader.spawn_resource_task(async move {
            let result = async {
                let response = task_resource_loader
                    .fetch(
                        Request::get_with_url(target_url.clone())
                            .with_page_network_policy()
                            .with_top_level_navigation_cookie_context(),
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                let head = response.head();
                if popup_document_response_should_ignore_navigation(response.status, &head.headers)
                {
                    return Ok(PopupDocumentLoadOutcome::IgnoredNavigation);
                }
                moli_fetch::ensure_http_status_success(
                    response.final_url.as_str(),
                    response.status,
                    false,
                )
                .map_err(|error| error.to_string())?;
                let content_type =
                    super::child_documents::child_document_content_type_from_headers(&head.headers);
                let fallback = if content_type
                    .as_deref()
                    .is_some_and(moli_web_mime::is_dom_parser_xml_mime)
                {
                    "UTF-8".to_owned()
                } else {
                    opener_character_set.clone()
                };
                let (markup, character_set) = decode_html_document_with_fallback(
                    response.body_bytes(),
                    &head.headers,
                    Some(&fallback),
                );
                let policy_container = DocumentPolicyContainer::from_navigation_response_headers(
                    &head.headers,
                    &head.final_url,
                );
                Ok(PopupDocumentLoadOutcome::Loaded(Box::new(
                    LoadedChildDocument {
                        final_url: head.final_url,
                        policy_container,
                        content_type,
                        character_set: character_set.to_owned(),
                        markup,
                        document_network: None,
                    },
                )))
            }
            .await;
            let _ =
                completion_tx.send_popup_document(PopupDocumentLoadCompletion::new(target, result));
        });
        Some(load_id)
    }

    pub(crate) fn current_lightweight_popup_document_fetch_target(
        &self,
        load_id: u64,
    ) -> Option<LightweightPopupDocumentFetchTarget> {
        let target = self
            .pending_lightweight_popup_document_loads
            .get(&load_id)?
            .target;
        self.lightweight_popup_navigation_attempt_is_current(target.task())
            .then_some(target)
    }

    pub(crate) fn current_lightweight_popup_classic_script_fetch_target(
        &self,
        load_id: u64,
    ) -> Option<LightweightPopupClassicScriptFetchTarget> {
        let target = self
            .pending_lightweight_popup_classic_script_loads
            .get(&load_id)?
            .target;
        self.lightweight_popup_committed_navigation_task_is_current(target.task())
            .then_some(target)
    }

    pub(crate) fn discard_stale_lightweight_popup_classic_script_completion(
        &mut self,
        target: LightweightPopupClassicScriptFetchTarget,
    ) {
        let should_remove = self
            .pending_lightweight_popup_classic_script_loads
            .get(&target.load_id())
            .is_some_and(|pending| pending.target == target);
        if should_remove
            && let Some(pending) = self
                .pending_lightweight_popup_classic_script_loads
                .remove(&target.load_id())
        {
            pending.cancel_handle.cancel();
        }
    }

    pub(crate) fn apply_lightweight_popup_document_load_completion(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        completion: PopupDocumentLoadCompletion,
    ) -> PopupDocumentLoadApplication {
        let target = completion.target();
        let Some(pending) = self
            .pending_lightweight_popup_document_loads
            .get(&target.load_id())
        else {
            return PopupDocumentLoadApplication::NotApplied;
        };
        let task = pending.target.task();
        if pending.target != target || !self.lightweight_popup_navigation_attempt_is_current(task) {
            return PopupDocumentLoadApplication::NotApplied;
        }
        let pending = self
            .pending_lightweight_popup_document_loads
            .remove(&target.load_id())
            .expect("authorized popup terminal must retain its exact pending load");
        let popup_id = task.popup_id();
        let document_owner = task.document_owner();
        let Some(window) = self.lightweight_popup_window(scope, popup_id) else {
            return PopupDocumentLoadApplication::NotApplied;
        };
        let mut body_activity = PopupDocumentLoadBodyActivity::NoPageCodeOrEventDispatch;
        let mut load_event_task = task;
        let mut classic_script_load_pending = false;
        match completion.result {
            Ok(PopupDocumentLoadOutcome::IgnoredNavigation) => {
                if let Some(record) = self.lightweight_popup_record_mut(popup_id) {
                    record.location_url = pending.previous_url.clone();
                }
                self.update_service_worker_popup_client_if_registered(
                    popup_id,
                    pending.previous_url.clone(),
                );
                let base_url = self
                    .lightweight_popup_base_url(scope, popup_id)
                    .unwrap_or_else(|| pending.previous_url.clone());
                let document_referrer = self
                    .lightweight_popup_document_record(popup_id)
                    .map(|document| document.state.policy_container.document_referrer.clone())
                    .unwrap_or_default();
                sync_lightweight_popup_window_location(
                    scope,
                    window,
                    pending.previous_url.as_str(),
                    &base_url,
                    &document_referrer,
                );
                if !self.lightweight_popup_has_document_projection(popup_id)
                    && moli_url::is_about_blank(&pending.previous_url)
                {
                    self.install_lightweight_popup_empty_document(
                        scope,
                        popup_id,
                        window,
                        pending.previous_url,
                    );
                }
                self.finish_service_worker_clients_open_window_popup(document_owner);
                return PopupDocumentLoadApplication::Applied { body_activity };
            }
            Ok(PopupDocumentLoadOutcome::Loaded(loaded)) => {
                let mut loaded = *loaded;
                let opener_sandbox = self
                    .lightweight_popup_record(popup_id)
                    .and_then(|record| record.opener_sandbox_policy);
                inherit_lightweight_popup_opener_sandbox(
                    &mut loaded.policy_container.sandbox,
                    opener_sandbox,
                );
                let final_url = popup_document_final_url_with_request_fragment(
                    loaded.final_url,
                    &pending.target_url,
                );
                let storage_scope = self.lightweight_popup_response_storage_scope(
                    &final_url,
                    &loaded.policy_container.response_content_security_policies,
                    loaded.policy_container.sandbox.forces_opaque_origin,
                );
                let committed_state = pending
                    .document_state
                    .clone()
                    .apply_navigation_response(final_url.clone(), loaded.policy_container.clone());
                if !self.commit_lightweight_popup_document(
                    scope,
                    window,
                    LightweightPopupDocumentCommit {
                        owner: document_owner,
                        location_url: final_url.clone(),
                        origin: LightweightPopupDocumentCommitOrigin::FromNavigationResponse(
                            storage_scope.origin().to_owned(),
                        ),
                        state: committed_state,
                        storage_scope,
                        navigation_loader: pending.resource_loader.clone(),
                    },
                ) {
                    return PopupDocumentLoadApplication::NotApplied;
                }
                if !self.lightweight_popup_committed_navigation_task_is_current(task) {
                    return PopupDocumentLoadApplication::Applied { body_activity };
                }
                if matches!(final_url.scheme(), "http" | "https") {
                    let _ = self.register_or_update_service_worker_popup_client(
                        document_owner,
                        final_url.clone(),
                    );
                } else {
                    self.unregister_service_worker_popup_client(popup_id);
                }
                let base_url = self
                    .lightweight_popup_base_url(scope, popup_id)
                    .unwrap_or_else(|| final_url.clone());
                let document_referrer = self
                    .lightweight_popup_document_record(popup_id)
                    .map(|document| document.state.policy_container.document_referrer.clone())
                    .unwrap_or_default();
                sync_lightweight_popup_window_location(
                    scope,
                    window,
                    final_url.as_str(),
                    &base_url,
                    &document_referrer,
                );
                if let Some(application) = self.install_lightweight_popup_document_projection(
                    scope,
                    window,
                    LightweightPopupDocumentProjectionSource {
                        task,
                        document_url: &final_url,
                        markup: &loaded.markup,
                        content_type: loaded.content_type.as_deref(),
                        character_set: Some(&loaded.character_set),
                        policy_container: &loaded.policy_container,
                    },
                ) {
                    body_activity = application.body_activity;
                    classic_script_load_pending = application.classic_script_load_pending;
                    if !self.lightweight_popup_committed_navigation_task_is_current(task) {
                        return PopupDocumentLoadApplication::Applied { body_activity };
                    }
                    if !classic_script_load_pending
                        && let Some(document_handle) = application.document_handle
                    {
                        self.sync_child_browsing_context_subtree(scope, document_handle);
                    }
                    if !self.lightweight_popup_committed_navigation_task_is_current(task) {
                        return PopupDocumentLoadApplication::Applied { body_activity };
                    }
                }
            }
            Err(error) => {
                self.finish_service_worker_clients_open_window_popup_with_null_for_owner(
                    document_owner,
                );
                self.unregister_service_worker_popup_client(popup_id);
                tracing::debug!(
                    popup_id,
                    navigation_id = task.navigation_id(),
                    url = %pending.target_url,
                    error,
                    "lightweight popup document load failed"
                );
                let Some(current_owner) = self.current_lightweight_popup_document_owner(popup_id)
                else {
                    return PopupDocumentLoadApplication::Applied { body_activity };
                };
                load_event_task = LightweightPopupNavigationTaskToken::from_parts(
                    current_owner,
                    task.navigation_id,
                );
            }
        }
        if classic_script_load_pending {
            return PopupDocumentLoadApplication::Applied { body_activity };
        }
        self.queue_lightweight_popup_load_event(load_event_task);
        self.finish_service_worker_clients_open_window_popup(document_owner);
        PopupDocumentLoadApplication::Applied { body_activity }
    }

    pub(crate) fn lightweight_popup_id_for_document_handle(
        &self,
        document_handle: DomHandle,
    ) -> Option<u64> {
        let popup_id = self
            .lightweight_popup_document_handles
            .get(&document_handle)
            .copied()?;
        (self.lightweight_popup_document_handle(popup_id) == Some(document_handle))
            .then_some(popup_id)
    }

    pub(crate) fn lightweight_popup_document_handle(&self, popup_id: u64) -> Option<DomHandle> {
        self.lightweight_popup_document_record(popup_id)
            .and_then(|document| document.handle)
    }

    pub(crate) fn lightweight_popup_referrer_for_document_handle(
        &self,
        document_handle: DomHandle,
    ) -> Option<&str> {
        self.lightweight_popup_id_for_document_handle(document_handle)
            .and_then(|popup_id| self.lightweight_popup_document_record(popup_id))
            .map(|document| document.state.policy_container.document_referrer.as_str())
    }

    pub(crate) fn lightweight_popup_referrer_policy_for_document_handle(
        &self,
        document_handle: DomHandle,
    ) -> Option<&str> {
        self.lightweight_popup_id_for_document_handle(document_handle)
            .and_then(|popup_id| self.lightweight_popup_document_record(popup_id))
            .and_then(|document| document.state.policy_container.referrer_policy.as_deref())
    }

    pub(crate) fn lightweight_popup_document_domain_override(
        &self,
        popup_id: u64,
    ) -> Option<String> {
        self.lightweight_popup_document_record(popup_id)
            .and_then(|document| document.state.document_domain_override.clone())
    }

    pub(crate) fn lightweight_popup_document_domain_is_sandboxed(&self, popup_id: u64) -> bool {
        self.lightweight_popup_document_record(popup_id)
            .is_some_and(|document| {
                document
                    .state
                    .policy_container
                    .sandbox
                    .sandboxes_document_domain
            })
    }

    pub(crate) fn set_lightweight_popup_document_domain_override(
        &mut self,
        popup_id: u64,
        domain: String,
    ) -> bool {
        let Some(serialized_origin) = self
            .lightweight_popup_document_record(popup_id)
            .map(|document| document.access_origin.serialized_origin())
        else {
            return false;
        };
        let Some(access_origin) =
            super::window_security_tokens::WindowAccessOrigin::from_serialized_origin(
                serialized_origin,
                Some(domain.clone()),
            )
        else {
            return false;
        };
        let Some(document) = self.lightweight_popup_document_record_mut(popup_id) else {
            return false;
        };
        document.state.document_domain_override = Some(domain);
        document.access_origin = access_origin;
        true
    }

    pub(crate) fn lightweight_popup_id_for_node_owner_document(
        &self,
        handle: DomHandle,
    ) -> Option<u64> {
        self.dom_host()
            .node(handle)
            .and_then(|node| node.owner_document())
            .and_then(|document_handle| {
                self.lightweight_popup_id_for_document_handle(document_handle)
            })
    }

    fn remember_lightweight_popup_document_handle<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        popup_id: u64,
        document: v8::Local<'s, v8::Object>,
    ) -> Option<DomHandle> {
        if let Ok((_runtime_ptr, document_handle)) =
            crate::native_bridge::node::node_runtime_and_handle_from_object_or_detached(
                scope, document,
            )
        {
            if self.lightweight_popup_document_handle(popup_id) != Some(document_handle) {
                self.forget_lightweight_popup_document_handle(popup_id);
            }
            self.sync_owner_style_sheet_texts_for_document_tree_scopes(document_handle);
            self.dom_host_mut()
                .mark_subtree_connected_preserving_owner_document(document_handle);
            self.lightweight_popup_document_handles
                .insert(document_handle, popup_id);
            if let Some(current_document) = self.lightweight_popup_document_record_mut(popup_id) {
                current_document.handle = Some(document_handle);
            }
            return Some(document_handle);
        }
        None
    }

    fn forget_lightweight_popup_document_handle(&mut self, popup_id: u64) {
        let document_handle = self
            .lightweight_popup_document_record_mut(popup_id)
            .and_then(|document| document.handle.take());
        if let Some(document_handle) = document_handle {
            self.retire_lightweight_popup_document_handle(popup_id, document_handle);
        }
    }

    fn retire_lightweight_popup_document_handle(
        &mut self,
        popup_id: u64,
        document_handle: DomHandle,
    ) {
        if self
            .lightweight_popup_document_handles
            .get(&document_handle)
            == Some(&popup_id)
        {
            self.lightweight_popup_document_handles
                .remove(&document_handle);
        }
        self.note_style_subtree_context_change(document_handle);
        self.dom_host_mut()
            .mark_subtree_disconnected_preserving_owner_document(document_handle);
        self.style_engine
            .retire_document_style_world(document_handle);
    }

    pub(crate) fn dispatch_lightweight_popup_window_event<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        popup_id: u64,
        event_type: &str,
        event: v8::Local<'s, v8::Object>,
    ) -> bool {
        let Some(window) = self.lightweight_popup_window(scope, popup_id) else {
            return true;
        };
        let Some(context) = window.get_creation_context(scope) else {
            return true;
        };
        let context = v8::Global::new(scope, context);
        let window = v8::Global::new(scope, window);
        let event = v8::Global::new(scope, event);
        let context = v8::Local::new(scope, &context);
        let popup_scope = &mut v8::ContextScope::new(scope, context);
        let window = v8::Local::new(popup_scope, &window);
        let event = v8::Local::new(popup_scope, &event);
        if !self.register_lightweight_popup_execution_context(popup_scope, popup_id) {
            return true;
        }
        let previous = enter_lightweight_popup_event_dispatch(popup_scope, popup_id);
        let previous_message_source = self.enter_window_message_source_scope(
            super::PendingWindowMessageEndpoint::LightweightPopup(popup_id),
        );
        let allows_default = dispatch_simple_event_target_event(
            popup_scope,
            window,
            LIGHTWEIGHT_POPUP_EVENT_LISTENERS_SLOT,
            event_type,
            event,
        );
        self.restore_window_message_source_scope(previous_message_source);
        restore_lightweight_popup_event_dispatch(popup_scope, previous);
        allows_default
    }

    pub(crate) fn dispatch_lightweight_popup_content_security_policy_violation_event_best_effort<
        's,
    >(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        popup_id: u64,
        violation: &crate::document_runtime::DocumentContentSecurityPolicyViolation,
    ) {
        self.dispatch_lightweight_popup_content_security_policy_violation_event_with_reporting(
            scope, popup_id, violation, true,
        );
    }

    pub(crate) fn dispatch_lightweight_popup_content_security_policy_violation_event_without_report_best_effort<
        's,
    >(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        popup_id: u64,
        violation: &crate::document_runtime::DocumentContentSecurityPolicyViolation,
    ) {
        self.dispatch_lightweight_popup_content_security_policy_violation_event_with_reporting(
            scope, popup_id, violation, false,
        );
    }

    fn dispatch_lightweight_popup_content_security_policy_violation_event_with_reporting<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        popup_id: u64,
        violation: &crate::document_runtime::DocumentContentSecurityPolicyViolation,
        send_report: bool,
    ) {
        let Some(window) = self.lightweight_popup_window(scope, popup_id) else {
            return;
        };
        if send_report {
            let fields =
                crate::content_security_policy::ContentSecurityPolicyViolationEventFields::from(
                    violation,
                );
            let Some(document_owner) = self.current_lightweight_popup_document_owner(popup_id)
            else {
                tracing::debug!(popup_id, "discarded CSP report for retired popup document");
                return;
            };
            crate::network_host::send_content_security_policy_reports_for_lightweight_popup(
                scope,
                self,
                popup_id,
                document_owner,
                &fields,
                &violation.report_uri_endpoints,
                &violation.report_to_endpoints,
            );
        }
        match create_content_security_policy_violation_event(
            scope,
            window.into(),
            window.into(),
            violation,
        ) {
            Ok(event) => {
                self.dispatch_lightweight_popup_window_event(
                    scope,
                    popup_id,
                    "securitypolicyviolation",
                    event,
                );
            }
            Err(error) => {
                tracing::error!(
                    popup_id,
                    blocked_uri = violation.blocked_uri.as_str(),
                    message = error.to_string().as_str(),
                    "lightweight popup securitypolicyviolation dispatch failed"
                );
            }
        }
    }

    pub(crate) fn dispatch_lightweight_popup_load_event(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        popup_id: u64,
    ) {
        let window = self
            .lightweight_popup_window(scope, popup_id)
            .expect("authorized popup load event must retain its WindowProxy");
        let event = lightweight_popup_event(scope, window, "load")
            .expect("authorized popup load event must materialize its Event");
        self.dispatch_lightweight_popup_window_event(scope, popup_id, "load", event);
    }

    fn install_lightweight_popup_document_projection<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        window: v8::Local<'s, v8::Object>,
        source: LightweightPopupDocumentProjectionSource<'_>,
    ) -> Option<LightweightPopupDocumentProjectionApplication> {
        let popup_id = source.task.popup_id();
        let document = crate::dom_parser::parse_browsing_context_document_projection_from_source(
            scope,
            source.document_url.clone(),
            source.markup,
            source.content_type,
            source.character_set,
            crate::parser::HtmlParser::with_scripting_enabled(
                self.lightweight_popup_scripting_enabled(popup_id),
            ),
        )?;
        let host_ptr = self as *mut JsContextHost;
        let document_base_url = lightweight_popup_parsed_document_base_url(
            host_ptr,
            scope,
            document,
            source.document_url,
        );
        if let Some(document_record) = self.lightweight_popup_document_record_mut(popup_id) {
            document_record.state.base_url = document_base_url.clone();
        }
        // The committed record owns the navigation-derived referrer. The
        // response policy container only carries response-derived policy and
        // can still have an empty referrer here.
        let document_referrer = self
            .lightweight_popup_document_record(popup_id)
            .map(|document| document.state.policy_container.document_referrer.clone())
            .unwrap_or_default();
        sync_lightweight_popup_document_window_slots(
            scope,
            document,
            window,
            &document_base_url,
            &document_referrer,
        );
        set_object_slot(scope, window, "document", document.into());
        self.forget_lightweight_popup_document_handle(popup_id);
        let document_handle =
            self.remember_lightweight_popup_document_handle(scope, popup_id, document);
        if let Some(document_handle) = document_handle {
            let _ = crate::context_bootstrap::install_css_runtime_state_for_document(
                scope,
                window,
                Some(document_handle),
            );
            install_lightweight_popup_get_computed_style(scope, window, document_handle);
        }
        let _ =
            self.set_lightweight_popup_document_wrapper(popup_id, v8::Global::new(scope, document));
        let script_advance = self.execute_lightweight_popup_document_scripts(
            scope,
            source.task,
            document_handle,
            source.policy_container.sandbox.allows_scripts,
            &source.policy_container.response_content_security_policies,
            &source
                .policy_container
                .response_content_security_report_only_policies,
            &source.policy_container.content_security_reporting_endpoints,
        );
        let (body_activity, classic_script_load_pending) = match script_advance {
            LightweightPopupClassicScriptAdvance::Completed(activity) => (activity, false),
            LightweightPopupClassicScriptAdvance::Pending(activity) => (activity, true),
        };
        Some(LightweightPopupDocumentProjectionApplication {
            document_handle,
            body_activity,
            classic_script_load_pending,
        })
    }

    fn execute_lightweight_popup_document_scripts(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        task: LightweightPopupNavigationTaskToken,
        document_handle: Option<DomHandle>,
        scripting_enabled: bool,
        response_content_security_policies: &[String],
        response_content_security_report_only_policies: &[String],
        response_content_security_reporting_endpoints:
            &crate::content_security_policy::ContentSecurityPolicyReportingEndpoints,
    ) -> LightweightPopupClassicScriptAdvance {
        let Some(document_handle) = document_handle else {
            return LightweightPopupClassicScriptAdvance::Completed(
                PopupDocumentLoadBodyActivity::NoPageCodeOrEventDispatch,
            );
        };
        if !scripting_enabled {
            return LightweightPopupClassicScriptAdvance::Completed(
                PopupDocumentLoadBodyActivity::NoPageCodeOrEventDispatch,
            );
        }
        let Some(document) = self.lightweight_popup_document_record(task.popup_id()) else {
            return LightweightPopupClassicScriptAdvance::Completed(
                PopupDocumentLoadBodyActivity::NoPageCodeOrEventDispatch,
            );
        };
        let continuation = LightweightPopupClassicScriptContinuation {
            task,
            document_handle,
            document_url: document.url.clone(),
            scripts: self.dom_host().script_handles_in_subtree(document_handle),
            next_script_index: 0,
            response_content_security_policies: response_content_security_policies.to_vec(),
            response_content_security_report_only_policies:
                response_content_security_report_only_policies.to_vec(),
            response_content_security_reporting_endpoints:
                response_content_security_reporting_endpoints.clone(),
        };
        self.advance_lightweight_popup_classic_scripts(
            scope,
            continuation,
            PopupDocumentLoadBodyActivity::NoPageCodeOrEventDispatch,
        )
    }

    fn advance_lightweight_popup_classic_scripts(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        mut continuation: LightweightPopupClassicScriptContinuation,
        mut body_activity: PopupDocumentLoadBodyActivity,
    ) -> LightweightPopupClassicScriptAdvance {
        let task = continuation.task;
        let popup_id = task.popup_id();
        while let Some(&script) = continuation.scripts.get(continuation.next_script_index) {
            continuation.next_script_index += 1;
            if !self.lightweight_popup_committed_navigation_task_is_current(task) {
                return LightweightPopupClassicScriptAdvance::Completed(body_activity);
            }
            let script_type = self
                .child_document_native_script_attribute(script, "type")
                .unwrap_or_default();
            if !script_type.is_empty()
                && !script_type.eq_ignore_ascii_case("text/javascript")
                && !script_type.eq_ignore_ascii_case("application/javascript")
            {
                continue;
            }

            let script_src = self.child_document_native_script_attribute(script, "src");
            let source = if let Some(src) = script_src.as_deref() {
                if src.is_empty() {
                    continue;
                }
                let base_url = self
                    .lightweight_popup_document_record(popup_id)
                    .map(|document| &document.state.base_url)
                    .unwrap_or(&continuation.document_url);
                let Ok(script_url) = Url::options().base_url(Some(base_url)).parse(src) else {
                    continue;
                };
                if let Some(violation) = unsafe { &*self.runtime }
                    .script_element_request_csp_report_only_violation_for_document(
                        &continuation.document_url,
                        &continuation.response_content_security_report_only_policies,
                        &continuation.response_content_security_reporting_endpoints,
                        &script_url,
                    )
                {
                    body_activity = PopupDocumentLoadBodyActivity::PageCodeOrEventDispatchAttempted;
                    self.dispatch_lightweight_popup_content_security_policy_violation_event_best_effort(
                        scope, popup_id, &violation,
                    );
                    if !self.lightweight_popup_committed_navigation_task_is_current(task) {
                        return LightweightPopupClassicScriptAdvance::Completed(body_activity);
                    }
                }
                if let Some(violation) = unsafe { &*self.runtime }
                    .script_element_request_csp_violation_for_document(
                        Some(continuation.document_handle),
                        &continuation.document_url,
                        &continuation.response_content_security_policies,
                        &continuation.response_content_security_reporting_endpoints,
                        &script_url,
                    )
                {
                    tracing::debug!(
                        popup_id,
                        url = %continuation.document_url,
                        blocked_uri = violation.blocked_uri.as_str(),
                        policy = violation.original_policy.as_str(),
                        "lightweight popup external script blocked by CSP"
                    );
                    continue;
                }
                self.mark_child_document_native_script_already_started(script);
                if let Some(source) =
                    self.materialize_local_lightweight_popup_script_source(&script_url)
                {
                    source
                } else {
                    let Some((loader, request)) = self
                        .prepare_lightweight_popup_classic_script_load(&continuation, &script_url)
                    else {
                        continue;
                    };
                    self.start_lightweight_popup_classic_script_load(
                        continuation,
                        script,
                        script_url,
                        loader,
                        request,
                    );
                    return LightweightPopupClassicScriptAdvance::Pending(body_activity);
                }
            } else {
                self.child_document_native_script_text_content(script)
            };

            if source.trim().is_empty() {
                continue;
            }
            if script_src.is_none()
                && let Some(violation) = unsafe { &*self.runtime }
                    .inline_script_csp_report_only_violation_for_document(
                        &continuation.document_url,
                        &continuation.response_content_security_report_only_policies,
                        &continuation.response_content_security_reporting_endpoints,
                    )
            {
                body_activity = PopupDocumentLoadBodyActivity::PageCodeOrEventDispatchAttempted;
                self.dispatch_lightweight_popup_content_security_policy_violation_event_best_effort(
                    scope, popup_id, &violation,
                );
                if !self.lightweight_popup_committed_navigation_task_is_current(task) {
                    return LightweightPopupClassicScriptAdvance::Completed(body_activity);
                }
            }
            if script_src.is_none()
                && let Some(violation) = unsafe { &*self.runtime }
                    .inline_script_csp_violation_for_document(
                        Some(continuation.document_handle),
                        &continuation.document_url,
                        &continuation.response_content_security_policies,
                        &continuation.response_content_security_reporting_endpoints,
                    )
            {
                tracing::debug!(
                    popup_id,
                    url = %continuation.document_url,
                    blocked_uri = violation.blocked_uri.as_str(),
                    policy = violation.original_policy.as_str(),
                    "lightweight popup inline script blocked by CSP"
                );
                continue;
            }
            if !self.lightweight_popup_committed_navigation_task_is_current(task) {
                return LightweightPopupClassicScriptAdvance::Completed(body_activity);
            }
            self.mark_child_document_native_script_already_started(script);
            body_activity = PopupDocumentLoadBodyActivity::PageCodeOrEventDispatchAttempted;
            self.execute_lightweight_popup_classic_script_source(
                scope,
                task,
                script,
                &continuation.document_url,
                &source,
            );
        }
        LightweightPopupClassicScriptAdvance::Completed(body_activity)
    }

    fn execute_lightweight_popup_classic_script_source(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        task: LightweightPopupNavigationTaskToken,
        script: DomHandle,
        document_url: &Url,
        source: &str,
    ) {
        let popup_id = task.popup_id();
        let Some(window) = self.lightweight_popup_window(scope, popup_id) else {
            return;
        };
        let Some(document) = self
            .lightweight_popup_document_record(popup_id)
            .and_then(|record| record.wrapper.as_ref())
            .map(|wrapper| v8::Local::new(scope, wrapper))
        else {
            return;
        };
        let current_script = crate::native_bridge::document::detached_native_object_for_handle(
            scope,
            self as *mut JsContextHost,
            script,
        );
        if let Err(error) = self.execute_lightweight_popup_window_script_source(
            scope,
            popup_id,
            task.document_owner(),
            window,
            document,
            current_script,
            source,
        ) {
            tracing::debug!(
                popup_id,
                url = %document_url,
                error = %error,
                "lightweight popup script execution failed"
            );
        }
    }

    fn child_document_native_script_attribute(
        &self,
        script: DomHandle,
        name: &str,
    ) -> Option<String> {
        self.dom_host()
            .node(script)
            .and_then(crate::dom::native::Node::as_element)
            .and_then(|element| {
                let normalized_name = element.normalized_attribute_name(name);
                element.attribute(&normalized_name).map(str::to_owned)
            })
    }

    fn child_document_native_script_text_content(&self, script: DomHandle) -> String {
        self.dom_host().text_content(script).unwrap_or_default()
    }

    fn mark_child_document_native_script_already_started(&mut self, script: DomHandle) {
        let _ = self.dom_host_mut().set_script_already_started(script, true);
    }

    fn materialize_local_lightweight_popup_script_source(&self, url: &Url) -> Option<String> {
        match url.scheme() {
            "http" | "https" => None,
            "blob" => {
                let (body, _) = crate::blob::object_url_body_and_type(url.as_str())?;
                Some(body)
            }
            _ => None,
        }
    }

    fn prepare_lightweight_popup_classic_script_load(
        &self,
        continuation: &LightweightPopupClassicScriptContinuation,
        script_url: &Url,
    ) -> Option<(crate::network::context::DocumentResourceLoader, Request)> {
        let task = continuation.task;
        if !self.lightweight_popup_committed_navigation_task_is_current(task) {
            return None;
        }
        let loader = self.document_resource_loader_for_window_owner(
            super::WindowDocumentOwner::LightweightPopup(task.document_owner()),
        )?;
        let request = Request::new("GET", script_url.as_str(), None, vec![])
            .ok()?
            .with_page_network_policy()
            .with_initiator_url(&continuation.document_url)
            .with_script_fetch_metadata(Default::default());
        Some((loader, request))
    }

    fn start_lightweight_popup_classic_script_load(
        &mut self,
        continuation: LightweightPopupClassicScriptContinuation,
        script_handle: DomHandle,
        script_url: Url,
        loader: crate::network::context::DocumentResourceLoader,
        request: Request,
    ) {
        let task = continuation.task;
        let load_id = self.next_lightweight_popup_classic_script_load_id;
        self.next_lightweight_popup_classic_script_load_id = self
            .next_lightweight_popup_classic_script_load_id
            .wrapping_add(1);
        let target = LightweightPopupClassicScriptFetchTarget::new(load_id, task);
        let cancel_handle = moli_fetch::FetchCancelHandle::new();
        self.pending_lightweight_popup_classic_script_loads.insert(
            load_id,
            PendingLightweightPopupClassicScriptLoad {
                target,
                script_handle,
                request_url: script_url.clone(),
                continuation,
                cancel_handle: cancel_handle.clone(),
            },
        );

        let request_client = loader.request_client().clone();
        let completion_tx = self.resource_completion_tx.clone();
        loader.spawn_resource_task(async move {
            let result = async {
                let response = request_client
                    .fetch_text_stream_with_cancel(request, cancel_handle)
                    .await
                    .map_err(|error| {
                        format!("failed to fetch popup script `{script_url}`: {error}")
                    })?;
                moli_fetch::ensure_http_status_success(
                    response.final_url.as_str(),
                    response.status,
                    false,
                )
                .map_err(|error| error.to_string())?;
                let (head, source) = response.into_text_parts();
                Ok(LoadedChildScriptSource {
                    final_url: head.final_url,
                    redirected: head.redirected,
                    source,
                })
            }
            .await;
            let _ = completion_tx
                .send_popup_classic_script(PopupClassicScriptLoadCompletion::new(target, result));
        });
    }

    pub(crate) fn apply_lightweight_popup_classic_script_load_completion(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        completion: PopupClassicScriptLoadCompletion,
    ) -> PopupClassicScriptLoadApplication {
        let target = completion.target();
        let Some(pending) = self
            .pending_lightweight_popup_classic_script_loads
            .get(&target.load_id())
        else {
            return PopupClassicScriptLoadApplication::NotApplied;
        };
        if pending.target != target
            || !self.lightweight_popup_committed_navigation_task_is_current(target.task())
        {
            return PopupClassicScriptLoadApplication::NotApplied;
        }
        let pending = self
            .pending_lightweight_popup_classic_script_loads
            .remove(&target.load_id())
            .expect("authorized popup script terminal must retain its exact pending load");
        let mut body_activity = PopupDocumentLoadBodyActivity::NoPageCodeOrEventDispatch;
        let task = pending.continuation.task;
        let popup_id = task.popup_id();

        match completion.result {
            Ok(fetched_source) => {
                let (blocked, csp_event_attempted) = self
                    .lightweight_popup_script_redirect_final_url_blocked_by_csp(
                        scope,
                        popup_id,
                        Some(pending.continuation.document_handle),
                        &pending.continuation.document_url,
                        &pending.continuation.response_content_security_policies,
                        &pending
                            .continuation
                            .response_content_security_report_only_policies,
                        &pending
                            .continuation
                            .response_content_security_reporting_endpoints,
                        &fetched_source,
                    );
                if csp_event_attempted {
                    body_activity = PopupDocumentLoadBodyActivity::PageCodeOrEventDispatchAttempted;
                }
                if !blocked
                    && !fetched_source.source.trim().is_empty()
                    && self.lightweight_popup_committed_navigation_task_is_current(task)
                {
                    body_activity = PopupDocumentLoadBodyActivity::PageCodeOrEventDispatchAttempted;
                    self.execute_lightweight_popup_classic_script_source(
                        scope,
                        task,
                        pending.script_handle,
                        &pending.continuation.document_url,
                        &fetched_source.source,
                    );
                }
            }
            Err(error) => {
                tracing::debug!(
                    popup_id,
                    url = %pending.request_url,
                    error,
                    "lightweight popup external script load failed"
                );
            }
        }

        if !self.lightweight_popup_committed_navigation_task_is_current(task) {
            return PopupClassicScriptLoadApplication::Applied { body_activity };
        }
        match self.advance_lightweight_popup_classic_scripts(
            scope,
            pending.continuation,
            body_activity,
        ) {
            LightweightPopupClassicScriptAdvance::Pending(activity) => {
                PopupClassicScriptLoadApplication::Applied {
                    body_activity: activity,
                }
            }
            LightweightPopupClassicScriptAdvance::Completed(activity) => {
                if self.lightweight_popup_committed_navigation_task_is_current(task) {
                    if let Some(document_handle) = self.lightweight_popup_document_handle(popup_id)
                    {
                        self.sync_child_browsing_context_subtree(scope, document_handle);
                    }
                    if self.lightweight_popup_committed_navigation_task_is_current(task) {
                        self.queue_lightweight_popup_load_event(task);
                        self.finish_service_worker_clients_open_window_popup(task.document_owner());
                    }
                }
                PopupClassicScriptLoadApplication::Applied {
                    body_activity: activity,
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn lightweight_popup_script_redirect_final_url_blocked_by_csp(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        popup_id: u64,
        document_handle: Option<crate::document_runtime::DomHandle>,
        document_url: &Url,
        response_content_security_policies: &[String],
        response_content_security_report_only_policies: &[String],
        response_content_security_reporting_endpoints:
            &crate::content_security_policy::ContentSecurityPolicyReportingEndpoints,
        fetched_source: &LoadedChildScriptSource,
    ) -> (bool, bool) {
        if !fetched_source.redirected {
            return (false, false);
        }
        let mut event_dispatch_attempted = false;
        let redirect_status =
            crate::content_security_policy::ContentSecurityPolicyRedirectStatus::FollowedRedirect;
        if let Some(violation) = unsafe { &*self.runtime }
            .script_element_request_csp_report_only_violation_for_document_with_redirect_status(
                document_url,
                response_content_security_report_only_policies,
                response_content_security_reporting_endpoints,
                &fetched_source.final_url,
                redirect_status,
                None,
            )
        {
            event_dispatch_attempted = true;
            self.dispatch_lightweight_popup_content_security_policy_violation_event_best_effort(
                scope, popup_id, &violation,
            );
        }
        if let Some(violation) = unsafe { &*self.runtime }
            .script_element_request_csp_violation_for_document_with_redirect_status(
                document_handle,
                document_url,
                response_content_security_policies,
                response_content_security_reporting_endpoints,
                &fetched_source.final_url,
                redirect_status,
                None,
            )
        {
            event_dispatch_attempted = true;
            tracing::debug!(
                popup_id,
                url = %document_url,
                blocked_uri = violation.blocked_uri.as_str(),
                policy = violation.original_policy.as_str(),
                "lightweight popup external script redirect final URL blocked by CSP"
            );
            self.dispatch_lightweight_popup_content_security_policy_violation_event_best_effort(
                scope, popup_id, &violation,
            );
            return (true, event_dispatch_attempted);
        }
        (false, event_dispatch_attempted)
    }

    fn execute_lightweight_popup_window_script_source<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        popup_id: u64,
        script_document_owner: LightweightPopupDocumentOwner,
        window: v8::Local<'s, v8::Object>,
        document: v8::Local<'s, v8::Object>,
        current_script: Option<v8::Local<'s, v8::Object>>,
        source: &str,
    ) -> Result<()> {
        let Some(context) = window.get_creation_context(scope) else {
            anyhow::bail!("popup window has no creation context");
        };
        let context = v8::Global::new(scope, context);
        let window = v8::Global::new(scope, window);
        let document = v8::Global::new(scope, document);
        let current_script = current_script.map(|script| v8::Global::new(scope, script));
        let context = v8::Local::new(scope, &context);
        let popup_scope = &mut v8::ContextScope::new(scope, context);
        let window = v8::Local::new(popup_scope, &window);
        let document = v8::Local::new(popup_scope, &document);
        let current_script = current_script
            .as_ref()
            .map(|script| v8::Local::new(popup_scope, script));
        if !self.register_lightweight_popup_execution_context(popup_scope, popup_id) {
            anyhow::bail!("popup execution context owner is not current");
        }
        self.execute_lightweight_popup_window_script_source_in_context(
            popup_scope,
            popup_id,
            script_document_owner,
            window,
            document,
            current_script,
            source,
        )
    }

    fn execute_lightweight_popup_window_script_source_in_context<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        popup_id: u64,
        script_document_owner: LightweightPopupDocumentOwner,
        window: v8::Local<'s, v8::Object>,
        document: v8::Local<'s, v8::Object>,
        current_script: Option<v8::Local<'s, v8::Object>>,
        source: &str,
    ) -> Result<()> {
        if !self.lightweight_popup_document_owner_is_current(script_document_owner) {
            anyhow::bail!("popup script document owner is no longer current");
        }
        let current_script = current_script
            .map(|script| script.into())
            .unwrap_or_else(|| v8::null(scope).into());
        let scope_bindings = ObjectLiteralDeclaration::bind(scope).into_object();
        if let Some(document) = self
            .lightweight_popup_document_record(popup_id)
            .filter(|document| document.owner == script_document_owner)
        {
            for (name, value) in &document.script_globals {
                set_object_value(scope, scope_bindings, name, v8::Local::new(scope, value));
            }
        }
        let exported_global_names = child_script_declared_global_names(source);
        let exported_global_names_for_persist = exported_global_names.clone();
        let predeclared_globals = exported_global_names
            .iter()
            .map(|name| format!("var {name};"))
            .collect::<Vec<_>>()
            .join("");
        let exported_globals = exported_global_names
            .into_iter()
            .map(|name| {
                format!(
                    "{{\
                        let __moliExportedValue = typeof {name} !== 'undefined' ? {name} : window[{name:?}];\
                        if (typeof __moliExportedValue !== 'undefined') {{ {name} = __moliExportedValue; __scope[{name:?}] = __moliExportedValue; window[{name:?}] = __moliExportedValue; }}\
                    }}"
                )
            })
            .collect::<Vec<_>>()
            .join("");
        let wrapped_source = format!(
            "(function(__scope, window, document, currentScript, __moliNativeEnterActivePopup, __moliNativeRestoreActivePopup) {{\
                const __prevCurrentScript = document.currentScript ?? null;\
                const __previousActivePopupForScript = __moliNativeEnterActivePopup();\
                try {{\
                    Object.defineProperty(document, 'currentScript', {{ configurable: true, enumerable: false, writable: true, value: currentScript }});\
                    if (!('onload' in window)) {{ window.onload = null; }}\
                    with (__scope) {{\
                        with (window) {{\
                            {predeclared_globals}\ntry {{\n{source}\n{exported_globals}\n}} finally {{\n{exported_globals}\n}}\
                        }}\
                    }}\
                }} finally {{\
                    __moliNativeRestoreActivePopup(__previousActivePopupForScript);\
                    Object.defineProperty(document, 'currentScript', {{ configurable: true, enumerable: false, writable: true, value: __prevCurrentScript }});\
                }}\
            }})"
        );
        let Some(source) = v8_string(scope, &wrapped_source) else {
            anyhow::bail!("failed to allocate popup script wrapper source");
        };
        let Some(function) = v8::Script::compile(scope, source, None)
            .and_then(|script| script.run(scope))
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        else {
            anyhow::bail!("v8 failed to materialize popup script wrapper");
        };
        let popup_id_value = v8::BigInt::new_from_u64(scope, popup_id);
        let Some(enter_active_popup) =
            v8::Function::builder(enter_active_lightweight_popup_callback)
                .data(popup_id_value.into())
                .build(scope)
        else {
            anyhow::bail!("v8 failed to materialize popup active-window enter callback");
        };
        let Some(restore_active_popup) =
            v8::Function::builder(restore_active_lightweight_popup_callback).build(scope)
        else {
            anyhow::bail!("v8 failed to materialize popup active-window restore callback");
        };
        let previous_message_source = self.enter_window_message_source_scope(
            super::PendingWindowMessageEndpoint::LightweightPopup(popup_id),
        );
        let run_succeeded = function
            .call(
                scope,
                window.into(),
                &[
                    scope_bindings.into(),
                    window.into(),
                    document.into(),
                    current_script,
                    enter_active_popup.into(),
                    restore_active_popup.into(),
                ],
            )
            .is_some();
        self.restore_window_message_source_scope(previous_message_source);

        if !self.lightweight_popup_document_owner_is_current(script_document_owner) {
            if !run_succeeded {
                anyhow::bail!("v8 failed to execute popup script");
            }
            return Ok(());
        }
        let mut popup_bindings = {
            let Some(document) = self
                .lightweight_popup_document_record_mut(popup_id)
                .filter(|document| document.owner == script_document_owner)
            else {
                if !run_succeeded {
                    anyhow::bail!("v8 failed to execute popup script");
                }
                return Ok(());
            };
            std::mem::take(&mut document.script_globals)
        };
        let mut persisted_names = popup_bindings
            .keys()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        persisted_names.extend(exported_global_names_for_persist.iter().cloned());
        let mut document_remained_current = true;
        for name in persisted_names {
            if !self.lightweight_popup_document_owner_is_current(script_document_owner) {
                document_remained_current = false;
                break;
            }
            if !object_has_own_named_property(scope, scope_bindings, &name) {
                popup_bindings.remove(&name);
                if let Some(key) = v8_string(scope, &name) {
                    let _ = window.delete(scope, key.into());
                }
                if !self.lightweight_popup_document_owner_is_current(script_document_owner) {
                    document_remained_current = false;
                    break;
                }
                continue;
            }
            let Some(key) = v8_string(scope, &name) else {
                continue;
            };
            if let Some(value) = scope_bindings.get(scope, key.into()) {
                if !self.lightweight_popup_document_owner_is_current(script_document_owner) {
                    document_remained_current = false;
                    break;
                }
                set_object_slot(scope, window, &name, value);
                if !self.lightweight_popup_document_owner_is_current(script_document_owner) {
                    document_remained_current = false;
                    break;
                }
                popup_bindings.insert(name, v8::Global::new(scope, value));
            }
        }
        if document_remained_current {
            for name in exported_global_names_for_persist {
                if !self.lightweight_popup_document_owner_is_current(script_document_owner) {
                    document_remained_current = false;
                    break;
                }
                let Some(key) = v8_string(scope, &name) else {
                    continue;
                };
                let Some(value) = window.get(scope, key.into()) else {
                    continue;
                };
                if !self.lightweight_popup_document_owner_is_current(script_document_owner) {
                    document_remained_current = false;
                    break;
                }
                if value.is_undefined() {
                    continue;
                }
                set_object_slot(scope, window, &name, value);
                if !self.lightweight_popup_document_owner_is_current(script_document_owner) {
                    document_remained_current = false;
                    break;
                }
                popup_bindings.insert(name, v8::Global::new(scope, value));
            }
        }
        if document_remained_current
            && let Some(document) = self
                .lightweight_popup_document_record_mut(popup_id)
                .filter(|document| document.owner == script_document_owner)
        {
            document.script_globals = popup_bindings;
        }
        if !run_succeeded {
            anyhow::bail!("v8 failed to execute popup script");
        }
        Ok(())
    }

    fn execute_lightweight_popup_window_javascript_url_source(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        task: LightweightPopupNavigationTaskToken,
        source: &str,
    ) -> Result<()> {
        let popup_id = task.popup_id();
        let Some(window) = self.lightweight_popup_window(scope, popup_id) else {
            anyhow::bail!("popup javascript URL has no window");
        };
        let Some(context) = window.get_creation_context(scope) else {
            anyhow::bail!("popup window has no creation context");
        };
        let context = v8::Global::new(scope, context);
        let window = v8::Global::new(scope, window);
        let context = v8::Local::new(scope, &context);
        let popup_scope = &mut v8::ContextScope::new(scope, context);
        let window = v8::Local::new(popup_scope, &window);
        if !self.register_lightweight_popup_execution_context(popup_scope, popup_id) {
            anyhow::bail!("popup execution context owner is not current");
        }
        self.execute_lightweight_popup_window_javascript_url_source_in_context(
            popup_scope,
            task,
            window,
            source,
        )
    }

    fn execute_lightweight_popup_window_javascript_url_source_in_context<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        task: LightweightPopupNavigationTaskToken,
        window: v8::Local<'s, v8::Object>,
        source: &str,
    ) -> Result<()> {
        let popup_id = task.popup_id();
        let owner = OwnerDispatchScope::LightweightPopup(popup_id);
        let Some(source) = self.prepare_javascript_url_source_for_execution(scope, owner, source)
        else {
            return Ok(());
        };
        let Some(source_value) = v8_string(scope, &source) else {
            anyhow::bail!("failed to allocate popup javascript URL source");
        };
        let Some(wrapper_source) = v8_string(
            scope,
            "(function(window, __moliSource) { with (window) { return eval(__moliSource); } })",
        ) else {
            anyhow::bail!("failed to allocate popup javascript URL wrapper");
        };
        let Some(function) = v8::Script::compile(scope, wrapper_source, None)
            .and_then(|script| script.run(scope))
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        else {
            anyhow::bail!("v8 failed to compile popup javascript URL wrapper");
        };
        let previous_popup = enter_active_lightweight_popup_scope(scope, popup_id);
        let previous_message_source = self.enter_window_message_source_scope(
            super::PendingWindowMessageEndpoint::LightweightPopup(popup_id),
        );
        let eval_permit = crate::context_bootstrap::arm_internal_javascript_url_eval(scope);
        let completion = function.call(scope, window.into(), &[window.into(), source_value.into()]);
        crate::context_bootstrap::restore_internal_javascript_url_eval(scope, eval_permit);
        self.restore_window_message_source_scope(previous_message_source);
        restore_active_lightweight_popup_scope(scope, previous_popup);
        let Some(completion) = completion else {
            anyhow::bail!("v8 failed to execute popup javascript URL");
        };
        if completion.is_string()
            && self.lightweight_popup_committed_navigation_task_is_current(task)
        {
            let markup = completion
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_default();
            self.commit_lightweight_popup_javascript_url_string_completion(
                scope, task, window, markup,
            );
        }
        Ok(())
    }

    fn commit_lightweight_popup_javascript_url_string_completion<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        task: LightweightPopupNavigationTaskToken,
        window: v8::Local<'s, v8::Object>,
        markup: String,
    ) -> bool {
        if !self.lightweight_popup_committed_navigation_task_is_current(task) {
            return false;
        }
        let popup_id = task.popup_id();
        let Some(document_url) = self
            .lightweight_popup_document_record(popup_id)
            .map(|document| document.url.clone())
        else {
            return false;
        };
        let Some((document_state, storage_scope)) =
            self.lightweight_popup_document_commit_seed(popup_id)
        else {
            return false;
        };
        let policy_container = document_state.policy_container.clone();
        if !self.commit_lightweight_popup_document(
            scope,
            window,
            LightweightPopupDocumentCommit {
                owner: task.document_owner(),
                location_url: document_url.clone(),
                origin: LightweightPopupDocumentCommitOrigin::RetainCurrent,
                state: document_state,
                storage_scope,
                navigation_loader: None,
            },
        ) || !self.lightweight_popup_committed_navigation_task_is_current(task)
        {
            return false;
        }
        let base_url = self
            .lightweight_popup_base_url(scope, popup_id)
            .unwrap_or_else(|| document_url.clone());
        sync_lightweight_popup_window_location(
            scope,
            window,
            document_url.as_str(),
            &base_url,
            &policy_container.document_referrer,
        );
        let Some(application) = self.install_lightweight_popup_document_projection(
            scope,
            window,
            LightweightPopupDocumentProjectionSource {
                task,
                document_url: &document_url,
                markup: &markup,
                content_type: Some("text/html"),
                character_set: Some("UTF-8"),
                policy_container: &policy_container,
            },
        ) else {
            return false;
        };
        if !self.lightweight_popup_committed_navigation_task_is_current(task) {
            return true;
        }
        if !application.classic_script_load_pending {
            if let Some(document_handle) = application.document_handle {
                self.sync_child_browsing_context_subtree(scope, document_handle);
            }
            if self.lightweight_popup_committed_navigation_task_is_current(task) {
                self.queue_lightweight_popup_load_event(task);
            }
        }
        true
    }

    fn queue_lightweight_popup_load_event(&mut self, task: LightweightPopupNavigationTaskToken) {
        if !self.lightweight_popup_committed_navigation_task_is_current(task) {
            return;
        }
        let document = self
            .lightweight_popup_document_record_mut(task.popup_id())
            .expect("current popup navigation must retain its Document record");
        if document.load_event_dispatched {
            return;
        }
        document.pending_load_event = Some(task);
        self.publish_lightweight_popup_load_event_if_ready(task.popup_id());
    }

    fn publish_lightweight_popup_load_event_if_ready(&mut self, popup_id: u64) {
        let Some(task) = self
            .lightweight_popup_document_record(popup_id)
            .filter(|document| {
                !document.load_event_dispatched
                    && document.queued_load_event.is_none()
                    && document.incomplete_child_frame_loads.is_empty()
                    && !self
                        .pending_lightweight_popup_classic_script_loads
                        .values()
                        .any(|pending| pending.target.task().popup_id() == popup_id)
            })
            .and_then(|document| document.pending_load_event)
        else {
            return;
        };
        if !self.lightweight_popup_committed_navigation_task_is_current(task) {
            return;
        }
        self.lightweight_popup_document_record_mut(popup_id)
            .expect("ready popup load event must retain its Document record")
            .queued_load_event = Some(task);
        if self.page_popup_load_event_sender().send(task).is_err() {
            let document = self
                .lightweight_popup_document_record_mut(popup_id)
                .expect("rejected popup load event must retain its Document record");
            if document.queued_load_event == Some(task) {
                document.queued_load_event = None;
            }
            tracing::debug!(
                popup_id,
                navigation_id = task.navigation_id(),
                "retired Page DOM-manipulation route rejected popup load delivery"
            );
        }
    }

    pub(in crate::native_bridge::context_host) fn note_lightweight_popup_child_frame_load_started(
        &mut self,
        child_handle: DomHandle,
    ) {
        if !self
            .frame_owner_store
            .current_child_frame_load_is_pending(child_handle)
        {
            return;
        }
        let Some(popup_id) = self.lightweight_popup_id_for_node_owner_document(child_handle) else {
            return;
        };
        let document = self
            .lightweight_popup_document_record_mut(popup_id)
            .expect("a current popup-owned frame must retain its Document record");
        if !document.load_event_dispatched {
            document.incomplete_child_frame_loads.insert(child_handle);
        }
    }

    pub(in crate::native_bridge::context_host) fn note_lightweight_popup_child_frame_load_finished(
        &mut self,
        child_handle: DomHandle,
    ) {
        let Some(popup_id) = self.lightweight_popup_id_for_node_owner_document(child_handle) else {
            return;
        };
        let removed = self
            .lightweight_popup_document_record_mut(popup_id)
            .expect("a current popup-owned frame must retain its Document record")
            .incomplete_child_frame_loads
            .remove(&child_handle);
        if removed {
            self.publish_lightweight_popup_load_event_if_ready(popup_id);
        }
    }

    pub(crate) fn current_lightweight_popup_load_event_task(
        &self,
        expected: LightweightPopupNavigationTaskToken,
    ) -> Option<LightweightPopupNavigationTaskToken> {
        self.lightweight_popup_committed_navigation_task_is_current(expected)
            .then(|| self.lightweight_popup_document_record(expected.popup_id()))
            .flatten()
            .filter(|document| {
                !document.load_event_dispatched
                    && document.incomplete_child_frame_loads.is_empty()
                    && document.pending_load_event == Some(expected)
                    && document.queued_load_event == Some(expected)
            })
            .map(|_| expected)
    }

    pub(crate) fn take_current_lightweight_popup_load_event_task(
        &mut self,
        expected: LightweightPopupNavigationTaskToken,
    ) {
        assert_eq!(
            self.current_lightweight_popup_load_event_task(expected),
            Some(expected),
            "authorized popup load event must retain its exact pending payload"
        );
        let document = self
            .lightweight_popup_document_record_mut(expected.popup_id())
            .expect("authorized popup load event must retain its Document record");
        document.pending_load_event = None;
        document.queued_load_event = None;
        document.load_event_dispatched = true;
    }

    pub(crate) fn discard_stale_lightweight_popup_load_event_task(
        &mut self,
        stale: LightweightPopupNavigationTaskToken,
    ) -> bool {
        let Some(document) = self.lightweight_popup_document_record_mut(stale.popup_id()) else {
            return false;
        };
        if document.queued_load_event != Some(stale) {
            return false;
        }
        document.queued_load_event = None;
        self.publish_lightweight_popup_load_event_if_ready(stale.popup_id());
        true
    }

    fn dispatch_lightweight_popup_same_document_navigation_events<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        popup_id: u64,
        window: v8::Local<'s, v8::Object>,
        old_url: &str,
        new_url: &str,
    ) {
        if old_url == new_url {
            return;
        }
        let Some(document_owner) = self.current_lightweight_popup_document_owner(popup_id) else {
            return;
        };
        let Some(navigation_id) = self.lightweight_popup_navigation_id(popup_id) else {
            return;
        };
        let task = LightweightPopupNavigationTaskToken::from_parts(document_owner, navigation_id);
        if let Some(event) = lightweight_popup_event(scope, window, "popstate") {
            let state = v8::null(scope).into();
            let _ = LightweightPopupPopStateEventDeclaration::new(state).initialize(scope, event);
            self.dispatch_lightweight_popup_window_event(scope, popup_id, "popstate", event);
        }
        if !self.lightweight_popup_committed_navigation_task_is_current(task) {
            return;
        }
        if lightweight_popup_should_dispatch_hash_change(old_url, new_url) {
            self.queue_lightweight_popup_hash_change_event(
                popup_id,
                old_url.to_owned(),
                new_url.to_owned(),
            );
        }
    }

    fn queue_lightweight_popup_hash_change_event(
        &mut self,
        popup_id: u64,
        old_url: String,
        new_url: String,
    ) {
        let dispatch_scope = OwnerDispatchScope::LightweightPopup(popup_id);
        let Some(owner) = self.current_window_execution_context_owner(dispatch_scope) else {
            return;
        };
        let target = WindowTaskTarget::new(dispatch_scope, owner);
        let sender = self.page_hash_change_delivery_sender();
        if sender
            .send(
                target,
                crate::page_task_queue::RendererPageHashChangeData::new(old_url, new_url),
            )
            .is_err()
        {
            tracing::debug!(
                popup_id,
                "retired Page DOM-manipulation route rejected popup hashchange delivery"
            );
        }
    }

    fn queue_lightweight_popup_javascript_url_task(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        task: LightweightPopupNavigationTaskToken,
        source: String,
    ) -> bool {
        if !self.lightweight_popup_committed_navigation_task_is_current(task) {
            return false;
        }
        let popup_id = task.popup_id();
        let Some(window) = self.lightweight_popup_window(scope, popup_id) else {
            return false;
        };
        let data = ObjectLiteralDeclaration::bind(scope).into_object();
        let popup_id_value = v8::BigInt::new_from_u64(scope, popup_id);
        set_object_slot(
            scope,
            data,
            LIGHTWEIGHT_POPUP_JAVASCRIPT_POPUP_ID_SLOT,
            popup_id_value.into(),
        );
        set_object_slot(
            scope,
            data,
            LIGHTWEIGHT_POPUP_JAVASCRIPT_DOCUMENT_ID_SLOT,
            v8::BigInt::new_from_u64(scope, task.document_owner().document_id().as_u64()).into(),
        );
        let navigation_id_value = v8::BigInt::new_from_u64(scope, task.navigation_id());
        set_object_slot(
            scope,
            data,
            LIGHTWEIGHT_POPUP_JAVASCRIPT_NAVIGATION_ID_SLOT,
            navigation_id_value.into(),
        );
        let Some(source_value) = v8_string(scope, &source) else {
            return false;
        };
        set_object_slot(
            scope,
            data,
            LIGHTWEIGHT_POPUP_JAVASCRIPT_SOURCE_SLOT,
            source_value.into(),
        );
        let Some(callback) = v8::Function::builder(lightweight_popup_javascript_url_callback)
            .data(data.into())
            .build(scope)
        else {
            return false;
        };
        self.queue_timeout_with_receiver(
            scope,
            callback,
            window,
            0,
            HostTimerOwner::Window,
            Vec::new(),
        );
        true
    }

    pub(crate) fn record_pending_popup_activation(
        &mut self,
        activation: RendererPendingPopupActivation,
        window_open_event: Option<crate::RendererPendingWindowOpenEvent>,
    ) {
        let mut items = Vec::with_capacity(1 + usize::from(window_open_event.is_some()));
        if let Some(event) = window_open_event {
            items.push(crate::runtime::RendererOutputItem::Observation(
                crate::runtime::RendererProtocolObservation::WindowOpen(event),
            ));
        }
        items.push(crate::runtime::RendererOutputItem::OwnerAction(
            crate::runtime::RendererOwnerAction::Popup(activation.clone()),
        ));
        let published = self.append_live_turn_items(items);
        if published {
            return;
        }
        #[cfg(test)]
        self.pending_popup_activations.push(activation);
        #[cfg(not(test))]
        {
            let _ = activation;
            panic!("a production popup must have a concrete renderer output sink");
        }
    }

    #[cfg(test)]
    pub(crate) fn take_pending_popup_activations(&mut self) -> Vec<RendererPendingPopupActivation> {
        std::mem::take(&mut self.pending_popup_activations)
    }

    pub(crate) fn pending_popup_activation_count(&self) -> usize {
        #[cfg(test)]
        {
            self.pending_popup_activations.len()
        }
        #[cfg(not(test))]
        {
            0
        }
    }
}

pub(crate) fn lightweight_popup_id_from_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    let value = get_private_value(scope, window, LIGHTWEIGHT_POPUP_ID_SLOT)?;
    lightweight_popup_id_from_value(scope, value)
}

pub(crate) fn active_lightweight_popup_id(scope: &mut v8::PinScope<'_, '_>) -> Option<u64> {
    let global = scope.get_current_context().global(scope);
    let value = get_private_value(scope, global, ACTIVE_LIGHTWEIGHT_POPUP_ID_SLOT)?;
    lightweight_popup_id_from_value(scope, value)
}

pub(crate) fn enter_active_lightweight_popup_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    popup_id: u64,
) -> v8::Local<'s, v8::Value> {
    enter_lightweight_popup_event_dispatch(scope, popup_id)
}

pub(crate) fn restore_active_lightweight_popup_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    previous: v8::Local<'s, v8::Value>,
) {
    restore_lightweight_popup_event_dispatch(scope, previous);
}

pub(crate) fn enter_top_level_lightweight_popup_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Value> {
    let global = scope.get_current_context().global(scope);
    let previous = get_private_value(scope, global, ACTIVE_LIGHTWEIGHT_POPUP_ID_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        global,
        ACTIVE_LIGHTWEIGHT_POPUP_ID_SLOT,
        v8::undefined(scope).into(),
    );
    previous
}

pub(crate) fn defer_active_lightweight_popup_restore<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    previous: v8::Local<'s, v8::Value>,
) {
    let previous_popup_id = lightweight_popup_id_from_value(scope, previous);
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        restore_lightweight_popup_event_dispatch(scope, previous);
        return;
    };
    unsafe { &mut *host_ptr }
        .defer_active_lightweight_popup_restore_after_microtasks(previous_popup_id);
}

pub(crate) fn restore_deferred_active_lightweight_popup_scope_if_present(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
) -> bool {
    let Some(previous) = host.take_deferred_active_lightweight_popup_restore() else {
        return false;
    };
    restore_active_lightweight_popup_scope_to_id(scope, previous);
    true
}

fn restore_active_lightweight_popup_scope_to_id(
    scope: &mut v8::PinScope<'_, '_>,
    popup_id: Option<u64>,
) {
    let value = popup_id
        .map(|popup_id| v8::BigInt::new_from_u64(scope, popup_id).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    let global = scope.get_current_context().global(scope);
    set_private_value(scope, global, ACTIVE_LIGHTWEIGHT_POPUP_ID_SLOT, value);
}

fn enter_lightweight_popup_event_dispatch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    popup_id: u64,
) -> v8::Local<'s, v8::Value> {
    let global = scope.get_current_context().global(scope);
    let previous = get_private_value(scope, global, ACTIVE_LIGHTWEIGHT_POPUP_ID_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let value = v8::BigInt::new_from_u64(scope, popup_id);
    set_private_value(
        scope,
        global,
        ACTIVE_LIGHTWEIGHT_POPUP_ID_SLOT,
        value.into(),
    );
    previous
}

fn restore_lightweight_popup_event_dispatch<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    previous: v8::Local<'s, v8::Value>,
) {
    let global = scope.get_current_context().global(scope);
    set_private_value(scope, global, ACTIVE_LIGHTWEIGHT_POPUP_ID_SLOT, previous);
}

fn install_lightweight_popup_indexed_db_factory<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    storage_scope: &LightweightPopupStorageScope,
    execution_context: super::WindowExecutionContextIdentity,
) -> bool {
    let storage_key = storage_scope.storage_key().serialized_storage_key();
    let Some(factory) = scoped_indexed_db_factory(scope, &storage_key) else {
        return false;
    };
    if !crate::context_bootstrap::bind_indexed_db_factory_to_window_execution_context(
        scope,
        factory,
        execution_context,
    ) {
        return false;
    }
    let factory_value: v8::Local<'s, v8::Value> = factory.into();
    set_object_slot(scope, window, "indexedDB", factory_value);
    true
}

fn install_lightweight_popup_viewport_surface_from_opener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    opener: v8::Local<'s, v8::Object>,
    window: v8::Local<'s, v8::Object>,
) {
    for property in LIGHTWEIGHT_POPUP_VIEWPORT_SURFACE_PROPERTIES {
        let Some(value) = opener.get(scope, v8str(scope, property).into()) else {
            continue;
        };
        if value.is_number() {
            set_object_slot(scope, window, property, value);
        }
    }
}

fn lightweight_popup_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let window = args.this();
    let Some(popup_id) = lightweight_popup_id_from_window(scope, window) else {
        return;
    };
    let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let Some(transition) = host.close_lightweight_popup_browsing_context(popup_id) else {
        return;
    };
    set_object_slot(
        scope,
        window,
        "closed",
        v8::Boolean::new(scope, true).into(),
    );
    host.unregister_service_worker_popup_client(popup_id);
    host.cancel_lightweight_popup_document_loads(popup_id);
    host.cancel_lightweight_popup_classic_script_loads(popup_id);
    clear_lightweight_popup_window_document_event_state(scope, window);
    if let Some(document_handle) = transition.retired_document_handle {
        host.retire_lightweight_popup_document_handle(popup_id, document_handle);
        host.clear_custom_element_registry_associations_for_document(document_handle);
    }
    host.retire_lightweight_popup_document_owner(transition.retired_owner);
    host.retire_lightweight_popup_local_window(popup_id, transition.retired_local_window_id);
    host.lightweight_popup_window_names
        .retain(|_, named_popup_id| *named_popup_id != popup_id);
}

fn lightweight_popup_initiator_endpoint<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    opener: Option<v8::Local<'s, v8::Object>>,
    opener_child_handle: Option<DomHandle>,
) -> Option<super::PendingWindowMessageEndpoint> {
    let opener = opener?;
    Some(
        opener_child_handle
            .map(super::PendingWindowMessageEndpoint::ChildWindow)
            .or_else(|| {
                lightweight_popup_id_from_window(scope, opener)
                    .map(super::PendingWindowMessageEndpoint::LightweightPopup)
            })
            .unwrap_or(super::PendingWindowMessageEndpoint::TopWindow),
    )
}

fn trackable_lightweight_popup_window_name(target_name: &str) -> Option<String> {
    if target_name.is_empty() || SpecialBrowsingContextTarget::parse(target_name).is_some() {
        return None;
    }
    Some(target_name.to_owned())
}

fn web_storage_key_for_child_about_blank_popup(
    opener_scope: &LightweightPopupStorageScope,
) -> MoliStorageKey {
    let popup_top_level_site = format!(
        "popup:{}",
        sha256_hex(
            opener_scope
                .storage_key()
                .serialized_storage_key()
                .as_bytes()
        )
    );
    MoliStorageKey::new(
        opener_scope.origin().to_owned(),
        popup_top_level_site,
        None,
        StoragePartitionRelation::ThirdParty,
    )
}

fn lightweight_popup_initial_base_url(target_url: &Url, creator_base_url: Url) -> Url {
    if moli_url::is_about_blank(target_url) {
        creator_base_url
    } else {
        target_url.clone()
    }
}

fn lightweight_popup_parsed_document_base_url<'s>(
    runtime_ptr: *mut JsContextHost,
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    fallback: &Url,
) -> Url {
    crate::native_bridge::document::detached_native_handle_for_runtime(scope, runtime_ptr, document)
        .and_then(|document_handle| {
            let runtime = unsafe { &*runtime_ptr };
            runtime
                .dom_host()
                .node(document_handle)
                .and_then(|node| node.as_document())
                .map(|document| document.base_url().clone())
        })
        .unwrap_or_else(|| fallback.clone())
}

fn sync_lightweight_popup_document_window_slots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    window: v8::Local<'s, v8::Object>,
    base_url: &Url,
    referrer: &str,
) {
    sync_document_location_runtime_state_from_window(scope, document, window);
    if let Some(href) = window
        .get(scope, v8str(scope, "location").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .and_then(|location| location.get(scope, v8str(scope, "href").into()))
    {
        set_object_slot(scope, document, "URL", href);
        set_object_slot(scope, document, "documentURI", href);
    }
    if let Some(base_uri) = v8_string(scope, base_url.as_str()) {
        set_object_slot(scope, document, "baseURI", base_uri.into());
    }
    if let Some(referrer) = v8_string(scope, referrer) {
        set_object_slot(scope, document, "referrer", referrer.into());
    }
    set_document_associated_window(scope, document, window);
    install_lightweight_popup_document_stream_methods(scope, document, window);
}

fn install_lightweight_popup_document_stream_methods<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    window: v8::Local<'s, v8::Object>,
) {
    let Some(popup_id) = lightweight_popup_id_from_window(scope, window) else {
        return;
    };
    let data = v8::BigInt::new_from_u64(scope, popup_id);
    let _ = LightweightPopupDocumentStreamMethodsDeclaration::new(data).initialize(scope, document);
}

fn install_lightweight_popup_get_computed_style<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    document: DomHandle,
) {
    let document = v8::BigInt::new_from_u64(scope, document.index() as u64);
    let _ = LightweightPopupComputedStyleMethodDeclaration::new(document).initialize(scope, window);
}

fn lightweight_popup_get_computed_style_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.length() == 0 {
        throw_type_error(
            scope,
            "Failed to execute 'getComputedStyle' on 'Window': 1 argument required, but only 0 present.",
        );
        return;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_null();
        return;
    };
    let Some(document) = child_window_handle_from_marker_data(scope, args.data()) else {
        rv.set_null();
        return;
    };
    let Some(target) = crate::native_bridge::node::current_or_live_delegate_node_arg_handle(
        scope,
        host_ptr,
        args.get(0),
    ) else {
        rv.set_null();
        return;
    };
    let target_document = unsafe { &*host_ptr }
        .dom_host()
        .node(target)
        .and_then(crate::dom::native::Node::owner_document);
    if target_document != Some(document) {
        rv.set_null();
        return;
    }

    unsafe { &*host_ptr }
        .drain_pending_style_invalidations_for_computed_style_read_for_document(document);
    let crate::window_host::ComputedStylePseudoArgument {
        forced_empty,
        pseudo_element,
        pseudo_key,
    } = crate::window_host::computed_style_pseudo_argument_from_function_args(scope, &args);
    let descriptor =
        ComputedStyleDescriptor::new(pseudo_key, ComputedStyleTargetKey::PopupDocument(document));
    let host = unsafe { &mut *host_ptr };
    let Some(style) = host
        .native_bridge_mut()
        .wrap_computed_style(scope, host_ptr, target, descriptor)
    else {
        rv.set_null();
        return;
    };
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_FORCED_EMPTY_COMPUTED_SLOT,
        v8::Boolean::new(scope, forced_empty).into(),
    );
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_TARGET_EMPTY_COMPUTED_SLOT,
        v8::Boolean::new(scope, false).into(),
    );
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_TARGET_CONTEXT_EPOCH_SLOT,
        v8::undefined(scope).into(),
    );
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_READ_DOCUMENT_SLOT,
        v8::Integer::new_from_unsigned(scope, document.index_u32()).into(),
    );
    let pseudo_value = pseudo_element
        .as_deref()
        .and_then(|pseudo_element| v8_string(scope, pseudo_element).map(Into::into))
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_PSEUDO_ELEMENT_SLOT,
        pseudo_value,
    );
    let viewport = host.style_viewport();
    let width = viewport
        .width
        .map(|width| v8::Number::new(scope, width).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(scope, style, STYLE_DECLARATION_VIEWPORT_WIDTH_SLOT, width);
    let height = viewport
        .height
        .map(|height| v8::Number::new(scope, height).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(scope, style, STYLE_DECLARATION_VIEWPORT_HEIGHT_SLOT, height);
    let screen_width = viewport
        .screen_width
        .map(|width| v8::Number::new(scope, width).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_SCREEN_WIDTH_SLOT,
        screen_width,
    );
    let screen_height = viewport
        .screen_height
        .map(|height| v8::Number::new(scope, height).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_SCREEN_HEIGHT_SLOT,
        screen_height,
    );
    rv.set(style.into());
}

fn popup_document_final_url_with_request_fragment(mut final_url: Url, request_url: &Url) -> Url {
    if final_url.fragment().is_none()
        && let Some(fragment) = request_url.fragment()
    {
        final_url.set_fragment(Some(fragment));
    }
    final_url
}

fn enter_active_lightweight_popup_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(popup_id) = lightweight_popup_id_from_value(scope, args.data()) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let previous = enter_lightweight_popup_event_dispatch(scope, popup_id);
    rv.set(previous);
}

fn restore_active_lightweight_popup_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    restore_lightweight_popup_event_dispatch(scope, args.get(0));
}

fn lightweight_popup_id_from_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<u64> {
    u64_from_value(scope, value).filter(|id| *id != 0)
}

fn u64_from_value(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<u64> {
    if let Ok(big) = v8::Local::<v8::BigInt>::try_from(value) {
        let (id, lossless) = big.u64_value();
        return lossless.then_some(id);
    }
    value
        .number_value(scope)
        .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
        .map(|value| value as u64)
}

fn about_blank_url() -> Url {
    Url::parse("about:blank").expect("about:blank should parse")
}

fn urls_refer_to_same_document_except_fragment(current: Option<&Url>, target: &Url) -> bool {
    let Some(current) = current else {
        return false;
    };
    let mut current_without_fragment = current.clone();
    current_without_fragment.set_fragment(None);
    let mut target_without_fragment = target.clone();
    target_without_fragment.set_fragment(None);
    current_without_fragment == target_without_fragment
}

fn lightweight_popup_should_dispatch_hash_change(old_url: &str, new_url: &str) -> bool {
    if old_url == new_url {
        return false;
    }
    let Ok(old) = Url::parse(old_url) else {
        return false;
    };
    let Ok(new) = Url::parse(new_url) else {
        return false;
    };
    urls_refer_to_same_document_except_fragment(Some(&old), &new)
        && old.fragment() != new.fragment()
}

fn sync_lightweight_popup_window_location<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    href: &str,
    base_url: &Url,
    referrer: &str,
) {
    sync_window_location_runtime_state(scope, window, href);
    if let Some(document) = window
        .get(scope, v8str(scope, "document").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        sync_lightweight_popup_document_window_slots(scope, document, window, base_url, referrer);
    }
}

fn clear_lightweight_popup_window_document_event_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) {
    let undefined = v8::undefined(scope);
    set_private_value(
        scope,
        window,
        LIGHTWEIGHT_POPUP_EVENT_LISTENERS_SLOT,
        undefined.into(),
    );
    let null = v8::null(scope).into();
    for name in WINDOW_EVENT_HANDLER_PROPERTIES {
        let _ = window.set(scope, v8str(scope, name).into(), null);
    }
}

fn lightweight_popup_effective_navigation_kind(
    current_url: Option<&Url>,
    kind: crate::context_bootstrap::LocationNavigationKind,
) -> crate::context_bootstrap::LocationNavigationKind {
    if matches!(
        kind,
        crate::context_bootstrap::LocationNavigationKind::Assign
    ) && current_url.is_some_and(moli_url::is_about_blank)
    {
        return crate::context_bootstrap::LocationNavigationKind::Replace;
    }
    kind
}

fn lightweight_popup_location_href<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<Url> {
    let location = window
        .get(scope, v8str(scope, "location").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let href = location
        .get(scope, v8str(scope, "href").into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))?;
    Url::parse(&href).ok()
}

fn lightweight_popup_document_handle_for_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = crate::native_bridge::document::detached_native_handle_for_runtime(
        scope, host_ptr, document,
    )?;
    Some((host_ptr, handle))
}

fn lightweight_popup_document_write_session_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(
        scope,
        document,
        LIGHTWEIGHT_POPUP_DOCUMENT_WRITE_SESSION_SLOT,
    )
    .is_some_and(|value| value.boolean_value(scope))
}

fn set_lightweight_popup_document_write_session<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    active: bool,
) {
    let value = v8::Boolean::new(scope, active);
    set_private_value(
        scope,
        document,
        LIGHTWEIGHT_POPUP_DOCUMENT_WRITE_SESSION_SLOT,
        value.into(),
    );
}

fn lightweight_popup_document_open_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let document = args.this();
    if let Some((host_ptr, document_handle)) =
        lightweight_popup_document_handle_for_callback(scope, document)
    {
        crate::native_bridge::document::set_detached_html_document_body_html(
            scope,
            host_ptr,
            document_handle,
            "",
        );
        set_lightweight_popup_document_write_session(scope, document, true);
    }
    rv.set(document.into());
}

fn lightweight_popup_document_write_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    lightweight_popup_document_write_or_writeln_callback(scope, args, rv, false);
}

fn lightweight_popup_document_writeln_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    lightweight_popup_document_write_or_writeln_callback(scope, args, rv, true);
}

fn lightweight_popup_document_write_or_writeln_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    append_newline: bool,
) {
    let document = args.this();
    let Some((host_ptr, document_handle)) =
        lightweight_popup_document_handle_for_callback(scope, document)
    else {
        rv.set_undefined();
        return;
    };
    let popup_id = lightweight_popup_id_from_value(scope, args.data());
    let writing_during_load = popup_id.is_some_and(|popup_id| {
        active_lightweight_popup_id(scope) == Some(popup_id)
            && !lightweight_popup_document_write_session_active(scope, document)
    });
    if writing_during_load {
        crate::native_bridge::document::set_detached_html_document_body_html(
            scope,
            host_ptr,
            document_handle,
            "",
        );
        set_lightweight_popup_document_write_session(scope, document, true);
    }
    let mut html = String::new();
    for index in 0..args.length() {
        let Some(value) = args.get(index).to_string(scope) else {
            return;
        };
        html.push_str(&value.to_rust_string_lossy(scope));
    }
    if append_newline {
        html.push('\n');
    }
    crate::native_bridge::document::append_detached_html_document_body_html(
        scope,
        host_ptr,
        document_handle,
        &html,
    );
    rv.set_undefined();
}

fn lightweight_popup_document_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_lightweight_popup_document_write_session(scope, args.this(), false);
    rv.set_undefined();
}

fn lightweight_popup_javascript_url_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    let Some(popup_id) = data
        .get(
            scope,
            v8str(scope, LIGHTWEIGHT_POPUP_JAVASCRIPT_POPUP_ID_SLOT).into(),
        )
        .and_then(|value| lightweight_popup_id_from_value(scope, value))
    else {
        return;
    };
    let Some(document_id) = data
        .get(
            scope,
            v8str(scope, LIGHTWEIGHT_POPUP_JAVASCRIPT_DOCUMENT_ID_SLOT).into(),
        )
        .and_then(|value| u64_from_value(scope, value))
        .map(LightweightPopupDocumentId::new)
    else {
        return;
    };
    let Some(navigation_id) = data
        .get(
            scope,
            v8str(scope, LIGHTWEIGHT_POPUP_JAVASCRIPT_NAVIGATION_ID_SLOT).into(),
        )
        .and_then(|value| u64_from_value(scope, value))
    else {
        return;
    };
    let Some(source) = data
        .get(
            scope,
            v8str(scope, LIGHTWEIGHT_POPUP_JAVASCRIPT_SOURCE_SLOT).into(),
        )
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
    else {
        return;
    };
    let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let task = LightweightPopupNavigationTaskToken::from_parts(
        LightweightPopupDocumentOwner::new(popup_id, document_id),
        LightweightPopupNavigationId::new(navigation_id),
    );
    if !host.lightweight_popup_committed_navigation_task_is_current(task) {
        return;
    }
    if let Err(error) =
        host.execute_lightweight_popup_window_javascript_url_source(scope, task, &source)
    {
        tracing::debug!(
            popup_id,
            navigation_id,
            error = %error,
            "lightweight popup javascript URL execution failed"
        );
    }
}

fn popup_document_response_should_ignore_navigation(
    status: u16,
    headers: &[(String, String)],
) -> bool {
    matches!(status, 204 | 205)
        || moli_web_mime::response_headers_indicate_attachment_download(headers)
}

fn lightweight_popup_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _window: v8::Local<'s, v8::Object>,
    event_type: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    LightweightPopupEventDeclaration::new(v8_string(scope, event_type)?)
        .bind(scope)
        .ok()
}
