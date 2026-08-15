//! Protocol-neutral DevTools runtime types.
//!
//! This module is the shared data-shape boundary for CDP, WebDriver Classic,
//! and WebDriver BiDi. It intentionally contains data shapes only; ownership
//! and behavior stay in the DevTools protocol owner layer.

use std::{fmt, str::FromStr, sync::Arc};

use moli_cookie_jar::{StoredCookiePartitionKey, StoredCookieQueryReport};
use moli_core::page::{
    DomScrollIntoViewRect, SelectedFile, SubresourceResourceType, is_renderer_backend_node_id,
};
use moli_protocol_cdp::{CdpTargetKindWire, CdpTargetType};
use serde_json::{Value, json};

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl FromStr for $name {
            type Err = std::convert::Infallible;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Ok(Self::new(value))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

string_id!(DevToolsBrowserContextId);
string_id!(DevToolsTargetId);
string_id!(DevToolsFrameId);
string_id!(DevToolsSessionId);
string_id!(DevToolsNavigationId);
string_id!(DevToolsRequestId);
string_id!(DevToolsLoaderId);
string_id!(DevToolsRealmId);
string_id!(DevToolsRemoteHandleId);
string_id!(DevToolsPreloadScriptId);
string_id!(DevToolsNetworkInterceptId);
string_id!(DevToolsNetworkDataCollectorId);
string_id!(DevToolsFetchRequestId);

pub fn webdriver_bidi_navigation_id_from_loader_id(loader_id: &str) -> DevToolsNavigationId {
    DevToolsNavigationId::from(format!("navigation-{loader_id}"))
}

const WEBDRIVER_BIDI_RENDERER_NODE_SHARED_ID_PREFIX: &str = "moli:bidi-renderer-node:";

pub fn webdriver_bidi_node_shared_id_for_backend_node_id(
    backend_node_id: u32,
) -> DevToolsRemoteHandleId {
    debug_assert!(is_renderer_backend_node_id(backend_node_id));
    DevToolsRemoteHandleId::from(format!(
        "{WEBDRIVER_BIDI_RENDERER_NODE_SHARED_ID_PREFIX}{backend_node_id}"
    ))
}

pub fn is_webdriver_bidi_node_shared_id(shared_id: &str) -> bool {
    shared_id
        .strip_prefix(WEBDRIVER_BIDI_RENDERER_NODE_SHARED_ID_PREFIX)
        .and_then(|id| id.parse::<u32>().ok())
        .is_some_and(is_renderer_backend_node_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webdriver_bidi_node_shared_id_uses_renderer_backend_identity() {
        let backend_node_id = moli_core::page::RENDERER_BACKEND_NODE_ID_START + 7;
        let shared_id = webdriver_bidi_node_shared_id_for_backend_node_id(backend_node_id);
        assert_eq!(
            shared_id.as_str(),
            format!("moli:bidi-renderer-node:{backend_node_id}")
        );
        assert!(is_webdriver_bidi_node_shared_id(shared_id.as_str()));
    }

    #[test]
    fn legacy_webdriver_bidi_node_shared_id_shape_is_not_internal() {
        assert!(!is_webdriver_bidi_node_shared_id("moli:bidi-node:_top:7"));
        assert!(!is_webdriver_bidi_node_shared_id(
            "moli:bidi-node:child-frame:11"
        ));
        assert!(!is_webdriver_bidi_node_shared_id("not-a-bidi-node"));
    }

    #[test]
    fn devtools_target_kind_maps_tab_cdp_wire_value() {
        assert_eq!(
            DevToolsTargetKind::from_cdp_type(Some("tab")),
            DevToolsTargetKind::Tab
        );
        assert_eq!(DevToolsTargetKind::Tab.as_cdp_type(), "tab");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DevToolsProtocol {
    Cdp,
    WebDriverClassic,
    WebDriverBidi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DevToolsTargetKind {
    Browser,
    Tab,
    Page,
    Frame,
    Worker,
    SharedWorker,
    ServiceWorker,
    Other,
}

impl DevToolsTargetKind {
    pub fn from_cdp_type(target_type: Option<&str>) -> Self {
        match CdpTargetType::from_wire_value(target_type) {
            CdpTargetType::Browser => Self::Browser,
            CdpTargetType::Tab => Self::Tab,
            CdpTargetType::Page => Self::Page,
            CdpTargetType::Frame => Self::Frame,
            CdpTargetType::Worker => Self::Worker,
            CdpTargetType::SharedWorker => Self::SharedWorker,
            CdpTargetType::ServiceWorker => Self::ServiceWorker,
            CdpTargetType::Other => Self::Other,
        }
    }

    pub fn as_cdp_type(self) -> &'static str {
        self.to_cdp_target_type().as_wire_value()
    }
}

impl CdpTargetKindWire for DevToolsTargetKind {
    fn to_cdp_target_type(self) -> CdpTargetType {
        match self {
            Self::Browser => CdpTargetType::Browser,
            Self::Tab => CdpTargetType::Tab,
            Self::Page => CdpTargetType::Page,
            Self::Frame => CdpTargetType::Frame,
            Self::Worker => CdpTargetType::Worker,
            Self::SharedWorker => CdpTargetType::SharedWorker,
            Self::ServiceWorker => CdpTargetType::ServiceWorker,
            Self::Other => CdpTargetType::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DevToolsNavigationWait {
    None,
    ResponseHead,
    DocumentInstalled,
    DomContentLoaded,
    Load,
    NetworkIdle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsCommandContext {
    pub protocol: DevToolsProtocol,
    pub session_id: Option<DevToolsSessionId>,
    pub target_id: Option<DevToolsTargetId>,
    pub browser_context_id: Option<DevToolsBrowserContextId>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DevToolsCommand {
    Navigate(DevToolsNavigateCommand),
    Reload(DevToolsReloadCommand),
    GetNavigationHistory(DevToolsGetNavigationHistoryCommand),
    TraverseHistory(DevToolsTraverseHistoryCommand),
    GetRealms(DevToolsGetRealmsCommand),
    EvaluateScript(DevToolsEvaluateScriptCommand),
    CallFunction(DevToolsCallFunctionCommand),
    TerminateExecution(DevToolsTerminateExecutionCommand),
    ReleaseObjects(DevToolsReleaseObjectsCommand),
    CreateTarget(DevToolsCreateTargetCommand),
    CloseTarget(DevToolsCloseTargetCommand),
    ActivateTarget(DevToolsActivateTargetCommand),
    GetTargets(DevToolsGetTargetsCommand),
    GetServiceWorkerLogs(DevToolsGetServiceWorkerLogsCommand),
    GetClientWindows(DevToolsGetClientWindowsCommand),
    SetClientWindowState(DevToolsSetClientWindowStateCommand),
    CreateBrowserContext(DevToolsCreateBrowserContextCommand),
    GetBrowserContexts(DevToolsGetBrowserContextsCommand),
    RemoveBrowserContext(DevToolsRemoveBrowserContextCommand),
    SetDownloadBehavior(DevToolsSetDownloadBehaviorCommand),
    SetPermission(DevToolsSetPermissionCommand),
    GetTargetInfo(DevToolsGetTargetInfoCommand),
    GetFrameTree(DevToolsGetFrameTreeCommand),
    GetFrameTrees(DevToolsGetFrameTreesCommand),
    GetLayoutMetrics(DevToolsGetLayoutMetricsCommand),
    GetJavaScriptDialog(DevToolsGetJavaScriptDialogCommand),
    SetJavaScriptDialogPromptText(DevToolsSetJavaScriptDialogPromptTextCommand),
    HandleJavaScriptDialog(DevToolsHandleJavaScriptDialogCommand),
    CaptureScreenshot(DevToolsCaptureScreenshotCommand),
    PrintToPdf(DevToolsPrintToPdfCommand),
    SetViewport(DevToolsSetViewportCommand),
    SetWindowState(DevToolsSetWindowStateCommand),
    SetUserAgentOverride(DevToolsSetUserAgentOverrideCommand),
    SetLocaleOverride(DevToolsSetLocaleOverrideCommand),
    SetTimezoneOverride(DevToolsSetTimezoneOverrideCommand),
    SetGeolocationOverride(DevToolsSetGeolocationOverrideCommand),
    SetNetworkConditions(DevToolsSetNetworkConditionsCommand),
    GetFrameOwner(DevToolsGetFrameOwnerCommand),
    LocateNodes(DevToolsLocateNodesCommand),
    GetDocument(DevToolsGetDocumentCommand),
    RequestChildNodes(DevToolsRequestChildNodesCommand),
    QuerySelector(DevToolsQuerySelectorCommand),
    PerformSearch(DevToolsPerformSearchCommand),
    GetSearchResults(DevToolsGetSearchResultsCommand),
    DiscardSearchResults(DevToolsDiscardSearchResultsCommand),
    GetNodeForLocation(DevToolsGetNodeForLocationCommand),
    ResolveNode(DevToolsResolveNodeCommand),
    GetAttributes(DevToolsGetAttributesCommand),
    GetText(DevToolsGetTextCommand),
    GetProperty(DevToolsGetPropertyCommand),
    PushNodesByBackendIds(DevToolsPushNodesByBackendIdsCommand),
    DescribeNode(DevToolsDescribeNodeCommand),
    DomObjectReference(DevToolsDomObjectReferenceCommand),
    SetFileInputFiles(DevToolsSetFileInputFilesCommand),
    GetOuterHtml(DevToolsGetOuterHtmlCommand),
    ScrollIntoViewIfNeeded(DevToolsScrollIntoViewIfNeededCommand),
    DomGeometry(DevToolsDomGeometryCommand),
    RemoveNode(DevToolsRemoveNodeCommand),
    AddPreloadScript(DevToolsAddPreloadScriptCommand),
    RemovePreloadScript(DevToolsRemovePreloadScriptCommand),
    DispatchMouseEvent(DevToolsDispatchMouseEventCommand),
    DispatchKeyEvent(DevToolsDispatchKeyEventCommand),
    DispatchTouchEvent(DevToolsDispatchTouchEventCommand),
    DispatchDragEvent(DevToolsDispatchDragEventCommand),
    SynthesizeTapGesture(DevToolsSynthesizeTapGestureCommand),
    GetCookies(DevToolsGetCookiesCommand),
    DeleteCookies(DevToolsDeleteCookiesCommand),
    SetCookies(DevToolsSetCookiesCommand),
    AddNetworkIntercept(DevToolsAddNetworkInterceptCommand),
    RemoveNetworkIntercept(DevToolsRemoveNetworkInterceptCommand),
    AddNetworkDataCollector(DevToolsAddNetworkDataCollectorCommand),
    RemoveNetworkDataCollector(DevToolsRemoveNetworkDataCollectorCommand),
    DisownNetworkData(DevToolsDisownNetworkDataCommand),
    SetCacheBehavior(DevToolsSetCacheBehaviorCommand),
    SetExtraHeaders(DevToolsSetExtraHeadersCommand),
    GetNetworkData(DevToolsGetNetworkDataCommand),
    ContinueInterceptedRequest(DevToolsContinueInterceptedRequestCommand),
    ContinueInterceptedResponse(DevToolsContinueInterceptedResponseCommand),
    ContinueWithAuth(DevToolsContinueWithAuthCommand),
    FailInterceptedRequest(DevToolsFailInterceptedRequestCommand),
    FulfillInterceptedRequest(DevToolsFulfillInterceptedRequestCommand),
}

impl DevToolsCommand {
    pub fn context(&self) -> &DevToolsCommandContext {
        match self {
            DevToolsCommand::Navigate(command) => &command.context,
            DevToolsCommand::Reload(command) => &command.context,
            DevToolsCommand::GetNavigationHistory(command) => &command.context,
            DevToolsCommand::TraverseHistory(command) => &command.context,
            DevToolsCommand::GetRealms(command) => &command.context,
            DevToolsCommand::EvaluateScript(command) => &command.context,
            DevToolsCommand::CallFunction(command) => &command.context,
            DevToolsCommand::TerminateExecution(command) => &command.context,
            DevToolsCommand::ReleaseObjects(command) => &command.context,
            DevToolsCommand::CreateTarget(command) => &command.context,
            DevToolsCommand::CloseTarget(command) => &command.context,
            DevToolsCommand::ActivateTarget(command) => &command.context,
            DevToolsCommand::GetTargets(command) => &command.context,
            DevToolsCommand::GetServiceWorkerLogs(command) => &command.context,
            DevToolsCommand::GetClientWindows(command) => &command.context,
            DevToolsCommand::SetClientWindowState(command) => &command.context,
            DevToolsCommand::CreateBrowserContext(command) => &command.context,
            DevToolsCommand::GetBrowserContexts(command) => &command.context,
            DevToolsCommand::RemoveBrowserContext(command) => &command.context,
            DevToolsCommand::SetDownloadBehavior(command) => &command.context,
            DevToolsCommand::SetPermission(command) => &command.context,
            DevToolsCommand::GetTargetInfo(command) => &command.context,
            DevToolsCommand::GetFrameTree(command) => &command.context,
            DevToolsCommand::GetFrameTrees(command) => &command.context,
            DevToolsCommand::GetLayoutMetrics(command) => &command.context,
            DevToolsCommand::GetJavaScriptDialog(command) => &command.context,
            DevToolsCommand::SetJavaScriptDialogPromptText(command) => &command.context,
            DevToolsCommand::HandleJavaScriptDialog(command) => &command.context,
            DevToolsCommand::CaptureScreenshot(command) => &command.context,
            DevToolsCommand::PrintToPdf(command) => &command.context,
            DevToolsCommand::SetViewport(command) => &command.context,
            DevToolsCommand::SetWindowState(command) => &command.context,
            DevToolsCommand::SetUserAgentOverride(command) => &command.context,
            DevToolsCommand::SetLocaleOverride(command) => &command.context,
            DevToolsCommand::SetTimezoneOverride(command) => &command.context,
            DevToolsCommand::SetGeolocationOverride(command) => &command.context,
            DevToolsCommand::SetNetworkConditions(command) => &command.context,
            DevToolsCommand::GetFrameOwner(command) => &command.context,
            DevToolsCommand::LocateNodes(command) => &command.context,
            DevToolsCommand::GetDocument(command) => &command.context,
            DevToolsCommand::RequestChildNodes(command) => &command.context,
            DevToolsCommand::QuerySelector(command) => &command.context,
            DevToolsCommand::PerformSearch(command) => &command.context,
            DevToolsCommand::GetSearchResults(command) => &command.context,
            DevToolsCommand::DiscardSearchResults(command) => &command.context,
            DevToolsCommand::GetNodeForLocation(command) => &command.context,
            DevToolsCommand::ResolveNode(command) => &command.context,
            DevToolsCommand::GetAttributes(command) => &command.context,
            DevToolsCommand::GetText(command) => &command.context,
            DevToolsCommand::GetProperty(command) => &command.context,
            DevToolsCommand::PushNodesByBackendIds(command) => &command.context,
            DevToolsCommand::DescribeNode(command) => &command.context,
            DevToolsCommand::DomObjectReference(command) => &command.context,
            DevToolsCommand::SetFileInputFiles(command) => &command.context,
            DevToolsCommand::GetOuterHtml(command) => &command.context,
            DevToolsCommand::ScrollIntoViewIfNeeded(command) => &command.context,
            DevToolsCommand::DomGeometry(command) => &command.context,
            DevToolsCommand::RemoveNode(command) => &command.context,
            DevToolsCommand::AddPreloadScript(command) => &command.context,
            DevToolsCommand::RemovePreloadScript(command) => &command.context,
            DevToolsCommand::DispatchMouseEvent(command) => &command.context,
            DevToolsCommand::DispatchKeyEvent(command) => &command.context,
            DevToolsCommand::DispatchTouchEvent(command) => &command.context,
            DevToolsCommand::DispatchDragEvent(command) => &command.context,
            DevToolsCommand::SynthesizeTapGesture(command) => &command.context,
            DevToolsCommand::GetCookies(command) => &command.context,
            DevToolsCommand::DeleteCookies(command) => &command.context,
            DevToolsCommand::SetCookies(command) => &command.context,
            DevToolsCommand::AddNetworkIntercept(command) => &command.context,
            DevToolsCommand::RemoveNetworkIntercept(command) => &command.context,
            DevToolsCommand::AddNetworkDataCollector(command) => &command.context,
            DevToolsCommand::RemoveNetworkDataCollector(command) => &command.context,
            DevToolsCommand::DisownNetworkData(command) => &command.context,
            DevToolsCommand::SetCacheBehavior(command) => &command.context,
            DevToolsCommand::SetExtraHeaders(command) => &command.context,
            DevToolsCommand::GetNetworkData(command) => &command.context,
            DevToolsCommand::ContinueInterceptedRequest(command) => &command.context,
            DevToolsCommand::ContinueInterceptedResponse(command) => &command.context,
            DevToolsCommand::ContinueWithAuth(command) => &command.context,
            DevToolsCommand::FailInterceptedRequest(command) => &command.context,
            DevToolsCommand::FulfillInterceptedRequest(command) => &command.context,
        }
    }

    pub fn set_webdriver_bidi_file_prompt_handler_for_script_command(
        &mut self,
        handler: Option<&str>,
    ) {
        let handler = handler.map(str::to_owned);
        match self {
            DevToolsCommand::EvaluateScript(command) => {
                command.webdriver_bidi_file_prompt_handler = handler;
            }
            DevToolsCommand::CallFunction(command) => {
                command.webdriver_bidi_file_prompt_handler = handler;
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsNavigateCommand {
    pub context: DevToolsCommandContext,
    pub url: String,
    pub referrer: Option<String>,
    pub wait: DevToolsNavigationWait,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsReloadCommand {
    pub context: DevToolsCommandContext,
    pub ignore_cache: bool,
    pub script_to_evaluate_on_load: Option<String>,
    pub wait: DevToolsNavigationWait,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetNavigationHistoryCommand {
    pub context: DevToolsCommandContext,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsTraverseHistoryCommand {
    pub context: DevToolsCommandContext,
    pub destination: DevToolsHistoryTraversalDestination,
    pub wait: DevToolsNavigationWait,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevToolsHistoryTraversalDestination {
    Entry { entry_id: i32, url: String },
    Delta(i64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetRealmsCommand {
    pub context: DevToolsCommandContext,
    pub realm_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsEvaluateScriptCommand {
    pub context: DevToolsCommandContext,
    pub realm_id: Option<DevToolsRealmId>,
    pub world_name: Option<String>,
    pub expression: String,
    pub await_promise: bool,
    pub user_gesture: bool,
    pub webdriver_bidi_file_prompt_handler: Option<String>,
    pub result_ownership: DevToolsResultOwnership,
    pub preserve_remote_metadata: bool,
    pub serialization_options: Option<DevToolsSerializationOptions>,
    pub materialize_bidi_script_result: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsCallFunctionCommand {
    pub context: DevToolsCommandContext,
    pub realm_id: Option<DevToolsRealmId>,
    pub world_name: Option<String>,
    pub object_id: Option<DevToolsRemoteHandleId>,
    pub this_parameter: Option<Value>,
    pub function_declaration: String,
    pub arguments: Vec<Value>,
    pub await_promise: bool,
    pub user_gesture: bool,
    pub webdriver_bidi_file_prompt_handler: Option<String>,
    pub result_ownership: DevToolsResultOwnership,
    pub object_group: Option<String>,
    pub preserve_remote_metadata: bool,
    pub serialization_options: Option<DevToolsSerializationOptions>,
    pub materialize_bidi_script_result: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsTerminateExecutionCommand {
    pub context: DevToolsCommandContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsSerializationOptions {
    pub max_object_depth: Option<u64>,
    pub max_dom_depth: Option<u64>,
    pub include_shadow_tree: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsReleaseObjectsCommand {
    pub context: DevToolsCommandContext,
    pub realm_id: Option<DevToolsRealmId>,
    pub world_name: Option<String>,
    pub handles: Vec<DevToolsRemoteHandleId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsCreateTargetCommand {
    pub context: DevToolsCommandContext,
    pub url: String,
    pub browser_context_id: Option<DevToolsBrowserContextId>,
    pub activate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsCloseTargetCommand {
    pub context: DevToolsCommandContext,
    pub target_id: DevToolsTargetId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsActivateTargetCommand {
    pub context: DevToolsCommandContext,
    pub target_id: DevToolsTargetId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetTargetsCommand {
    pub context: DevToolsCommandContext,
    pub root: Option<DevToolsTargetId>,
    pub max_depth: Option<u32>,
    pub filter: Option<Vec<DevToolsTargetFilterEntry>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsTargetFilterEntry {
    pub exclude: bool,
    pub target_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetServiceWorkerLogsCommand {
    pub context: DevToolsCommandContext,
    pub target_id: Option<DevToolsTargetId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetClientWindowsCommand {
    pub context: DevToolsCommandContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsCreateBrowserContextCommand {
    pub context: DevToolsCommandContext,
    pub browser_context_id: Option<DevToolsBrowserContextId>,
    pub accept_insecure_certs: Option<bool>,
    pub proxy_server: Option<String>,
    pub proxy_bypass_list: Option<String>,
    pub proxy_autoconfig_url: Option<String>,
    pub proxy_socks_version: Option<u8>,
    pub persistent_partition_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetBrowserContextsCommand {
    pub context: DevToolsCommandContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsRemoveBrowserContextCommand {
    pub context: DevToolsCommandContext,
    pub browser_context_id: DevToolsBrowserContextId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsSetDownloadBehaviorCommand {
    pub context: DevToolsCommandContext,
    pub behavior: Option<DevToolsDownloadBehaviorSetting>,
    pub user_contexts: Option<Vec<DevToolsBrowserContextId>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsSetPermissionCommand {
    pub context: DevToolsCommandContext,
    pub permission: Value,
    pub setting: String,
    pub origin: String,
    pub embedded_origin: Option<String>,
    pub browser_context_id: Option<DevToolsBrowserContextId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsDownloadBehaviorSetting {
    pub behavior: String,
    pub download_path: Option<String>,
    pub events_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetTargetInfoCommand {
    pub context: DevToolsCommandContext,
    pub target_id: Option<DevToolsTargetId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetFrameTreeCommand {
    pub context: DevToolsCommandContext,
    pub max_depth: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetFrameTreesCommand {
    pub context: DevToolsCommandContext,
    pub max_depth: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetLayoutMetricsCommand {
    pub context: DevToolsCommandContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetJavaScriptDialogCommand {
    pub context: DevToolsCommandContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsSetJavaScriptDialogPromptTextCommand {
    pub context: DevToolsCommandContext,
    pub prompt_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsHandleJavaScriptDialogCommand {
    pub context: DevToolsCommandContext,
    pub accept: bool,
    pub prompt_text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsCaptureScreenshotCommand {
    pub context: DevToolsCommandContext,
    pub format: Option<String>,
    pub quality: Option<u8>,
    pub clip: Option<DevToolsCaptureScreenshotClip>,
    pub capture_beyond_viewport: bool,
    pub optimize_for_speed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DevToolsCaptureScreenshotClip {
    Box(DevToolsScreenshotClip),
    Element(DevToolsScreenshotElementClip),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsScreenshotClip {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsScreenshotElementClip {
    pub shared_id: DevToolsRemoteHandleId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsPrintToPdfCommand {
    pub context: DevToolsCommandContext,
    pub landscape: Option<bool>,
    pub print_background: Option<bool>,
    pub scale: Option<f64>,
    pub paper_width: Option<f64>,
    pub paper_height: Option<f64>,
    pub margin_top: Option<f64>,
    pub margin_bottom: Option<f64>,
    pub margin_left: Option<f64>,
    pub margin_right: Option<f64>,
    pub page_ranges: Option<String>,
    pub shrink_to_fit: Option<bool>,
    pub transfer_mode: Option<DevToolsPrintToPdfTransferMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevToolsPrintToPdfTransferMode {
    ReturnAsBase64,
    ReturnAsStream,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsSetViewportCommand {
    pub context: DevToolsCommandContext,
    pub browser_context_ids: Vec<DevToolsBrowserContextId>,
    pub viewport: DevToolsViewportSetting,
    pub device_pixel_ratio: DevToolsDevicePixelRatioSetting,
    pub screen_width: Option<u32>,
    pub screen_height: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevToolsWindowState {
    Normal,
    Maximized,
    Minimized,
    Fullscreen,
}

impl DevToolsWindowState {
    pub fn as_bidi_value(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Maximized => "maximized",
            Self::Minimized => "minimized",
            Self::Fullscreen => "fullscreen",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsSetWindowStateCommand {
    pub context: DevToolsCommandContext,
    pub state: DevToolsWindowState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsSetClientWindowStateCommand {
    pub context: DevToolsCommandContext,
    pub client_window: DevToolsTargetId,
    pub state: DevToolsWindowState,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub x: Option<i32>,
    pub y: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsSetUserAgentOverrideCommand {
    pub context: DevToolsCommandContext,
    pub target_ids: Vec<DevToolsTargetId>,
    pub browser_context_ids: Vec<DevToolsBrowserContextId>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsSetLocaleOverrideCommand {
    pub context: DevToolsCommandContext,
    pub target_ids: Vec<DevToolsTargetId>,
    pub browser_context_ids: Vec<DevToolsBrowserContextId>,
    pub locale: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsSetTimezoneOverrideCommand {
    pub context: DevToolsCommandContext,
    pub target_ids: Vec<DevToolsTargetId>,
    pub browser_context_ids: Vec<DevToolsBrowserContextId>,
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DevToolsGeolocationOverride {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: f64,
    pub altitude: Option<f64>,
    pub altitude_accuracy: Option<f64>,
    pub heading: Option<f64>,
    pub speed: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DevToolsGeolocationOverrideState {
    Position(DevToolsGeolocationOverride),
    PositionUnavailable,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsSetGeolocationOverrideCommand {
    pub context: DevToolsCommandContext,
    pub target_ids: Vec<DevToolsTargetId>,
    pub browser_context_ids: Vec<DevToolsBrowserContextId>,
    pub override_state: Option<DevToolsGeolocationOverrideState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DevToolsNetworkConditions {
    pub offline: bool,
}

impl DevToolsNetworkConditions {
    pub fn offline() -> Self {
        Self { offline: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsSetNetworkConditionsCommand {
    pub context: DevToolsCommandContext,
    pub target_ids: Vec<DevToolsTargetId>,
    pub browser_context_ids: Vec<DevToolsBrowserContextId>,
    pub network_conditions: Option<DevToolsNetworkConditions>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevToolsViewportSetting {
    Unchanged,
    Default,
    Dimensions { width: u32, height: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DevToolsDevicePixelRatioSetting {
    Unchanged,
    Default,
    Scale(f64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetCookiesCommand {
    pub context: DevToolsCommandContext,
    pub browser_context_id: Option<DevToolsBrowserContextId>,
    pub urls: Option<Vec<String>>,
    pub filter: Option<DevToolsCookieFilter>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsDeleteCookiesCommand {
    pub context: DevToolsCommandContext,
    pub browser_context_id: Option<DevToolsBrowserContextId>,
    pub name: Option<String>,
    pub url: Option<String>,
    pub domain: Option<String>,
    pub path: Option<String>,
    pub partition_key: Option<StoredCookiePartitionKey>,
    pub filter: Option<DevToolsCookieFilter>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsSetCookiesCommand {
    pub context: DevToolsCommandContext,
    pub browser_context_id: Option<DevToolsBrowserContextId>,
    pub cookies: Vec<DevToolsCookieParam>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsCookieParam {
    pub name: String,
    pub value: String,
    pub url: Option<String>,
    pub domain: Option<String>,
    pub path: Option<String>,
    pub secure: Option<bool>,
    pub http_only: bool,
    pub same_site: Option<String>,
    pub priority: Option<String>,
    pub source_scheme: Option<String>,
    pub source_port: Option<i32>,
    pub partition_key: Option<Value>,
    pub partition_key_opaque: Option<bool>,
    pub expires: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsCookieFilter {
    pub name: Option<String>,
    pub value: Option<String>,
    pub domain: Option<String>,
    pub path: Option<String>,
    pub secure: Option<bool>,
    pub http_only: Option<bool>,
    pub same_site: Option<String>,
    pub size: Option<u64>,
    pub expires: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetFrameOwnerCommand {
    pub context: DevToolsCommandContext,
    pub frame_id: DevToolsFrameId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsLocateNodesCommand {
    pub context: DevToolsCommandContext,
    pub locator: DevToolsLocateNodesLocator,
    pub max_node_count: Option<u64>,
    pub start_nodes: Vec<Value>,
    pub start_node_references: Vec<DevToolsDomNodeReference>,
    pub serialization_options: Option<DevToolsSerializationOptions>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevToolsLocateNodesLocator {
    Css(String),
    XPath(String),
    TagName(String),
    LinkText {
        value: String,
        match_type: DevToolsLocateNodesTextMatch,
    },
    Context(DevToolsFrameId),
    InnerText {
        value: String,
        ignore_case: bool,
        match_type: DevToolsLocateNodesTextMatch,
        max_depth: u64,
    },
    Accessibility {
        role: Option<String>,
        name: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevToolsLocateNodesTextMatch {
    Full,
    Partial,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetDocumentCommand {
    pub context: DevToolsCommandContext,
    pub depth: Option<i32>,
    pub pierce: bool,
    pub flattened: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DevToolsDomNodeReference {
    FrontendNodeId(u32),
    BackendNodeId(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsRequestChildNodesCommand {
    pub context: DevToolsCommandContext,
    pub reference: DevToolsDomNodeReference,
    pub depth: i32,
    pub pierce: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsQuerySelectorCommand {
    pub context: DevToolsCommandContext,
    pub root: Option<DevToolsDomNodeReference>,
    pub selector: String,
    pub multiple: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsPerformSearchCommand {
    pub context: DevToolsCommandContext,
    pub query: String,
    pub include_user_agent_shadow_dom: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetSearchResultsCommand {
    pub context: DevToolsCommandContext,
    pub search_id: String,
    pub from_index: usize,
    pub to_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsDiscardSearchResultsCommand {
    pub context: DevToolsCommandContext,
    pub search_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsGetNodeForLocationCommand {
    pub context: DevToolsCommandContext,
    pub x: f64,
    pub y: f64,
    pub include_user_agent_shadow_dom: bool,
    pub ignore_pointer_events_none: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetNodeForLocationResult {
    pub backend_node_id: u32,
    pub frame_id: DevToolsFrameId,
    pub node_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsResolveNodeCommand {
    pub context: DevToolsCommandContext,
    pub reference: DevToolsDomNodeReference,
    pub execution_context_id: Option<i64>,
    pub object_group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetAttributesCommand {
    pub context: DevToolsCommandContext,
    pub reference: DevToolsDomNodeReference,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetTextCommand {
    pub context: DevToolsCommandContext,
    pub reference: DevToolsDomNodeReference,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetPropertyCommand {
    pub context: DevToolsCommandContext,
    pub reference: DevToolsDomNodeReference,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsPushNodesByBackendIdsCommand {
    pub context: DevToolsCommandContext,
    pub backend_node_ids: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsPushNodesByBackendIdsResult {
    pub node_ids: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsDescribeNodeCommand {
    pub context: DevToolsCommandContext,
    pub reference: Option<DevToolsDomNodeReference>,
    pub depth: i32,
    pub pierce: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DevToolsDomObjectReferenceOperation {
    RequestNode,
    GetOuterHtml { include_shadow_dom: bool },
    GetBoxModel,
    GetContentQuads,
    ScrollIntoViewIfNeeded { rect: Option<DomScrollIntoViewRect> },
    DescribeNode { depth: i32, pierce: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsDomObjectReferenceCommand {
    pub context: DevToolsCommandContext,
    pub object_id: DevToolsRemoteHandleId,
    pub operation: DevToolsDomObjectReferenceOperation,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsSetFileInputFilesCommand {
    pub context: DevToolsCommandContext,
    pub object_id: DevToolsRemoteHandleId,
    pub files: Vec<SelectedFile>,
    pub append: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetOuterHtmlCommand {
    pub context: DevToolsCommandContext,
    pub reference: Option<DevToolsDomNodeReference>,
    pub include_shadow_dom: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsScrollIntoViewIfNeededCommand {
    pub context: DevToolsCommandContext,
    pub reference: Option<DevToolsDomNodeReference>,
    pub rect: Option<DomScrollIntoViewRect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevToolsDomGeometryOperation {
    GetBoxModel,
    GetContentQuads,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsDomGeometryCommand {
    pub context: DevToolsCommandContext,
    pub reference: DevToolsDomNodeReference,
    pub operation: DevToolsDomGeometryOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsRemoveNodeCommand {
    pub context: DevToolsCommandContext,
    pub reference: DevToolsDomNodeReference,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsAddPreloadScriptCommand {
    pub context: DevToolsCommandContext,
    pub source: DevToolsPreloadScriptSource,
    pub world_name: Option<String>,
    pub target_ids: Option<Vec<DevToolsTargetId>>,
    pub browser_context_ids: Vec<DevToolsBrowserContextId>,
    pub run_immediately: bool,
    pub include_command_line_api: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DevToolsPreloadScriptSource {
    RawScript(String),
    FunctionDeclaration {
        function_declaration: String,
        arguments: Vec<Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsRemovePreloadScriptCommand {
    pub context: DevToolsCommandContext,
    pub script_id: DevToolsPreloadScriptId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DevToolsMouseEventType {
    Pressed,
    Released,
    Moved,
    Wheel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DevToolsPointerType {
    Mouse,
    Pen,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsDispatchMouseEventCommand {
    pub context: DevToolsCommandContext,
    pub event_type: DevToolsMouseEventType,
    pub pointer_type: DevToolsPointerType,
    pub x: f64,
    pub y: f64,
    pub button: i32,
    pub buttons: Option<i32>,
    pub click_count: i32,
    pub delta_x: f64,
    pub delta_y: f64,
    pub force: f64,
    pub tangential_pressure: f64,
    pub tilt_x: f64,
    pub tilt_y: f64,
    pub twist: f64,
    pub modifiers: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DevToolsKeyEventType {
    KeyDown,
    KeyUp,
    KeyPress,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsDispatchKeyEventCommand {
    pub context: DevToolsCommandContext,
    pub event_type: DevToolsKeyEventType,
    pub key: String,
    pub code: String,
    pub text: String,
    pub modifiers: u8,
    pub auto_repeat: bool,
    pub should_insert_text: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DevToolsTouchEventType {
    Start,
    Move,
    End,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DevToolsTouchPoint {
    pub id: i32,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsDispatchTouchEventCommand {
    pub context: DevToolsCommandContext,
    pub event_type: DevToolsTouchEventType,
    pub touch_points: Vec<DevToolsTouchPoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DevToolsDragEventType {
    Enter,
    Over,
    Drop,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsDragDataItem {
    pub mime_type: String,
    pub data: String,
    pub title: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsDragData {
    pub items: Vec<DevToolsDragDataItem>,
    pub files: Vec<String>,
    pub drag_operations_mask: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsDispatchDragEventCommand {
    pub context: DevToolsCommandContext,
    pub event_type: DevToolsDragEventType,
    pub x: f64,
    pub y: f64,
    pub data: DevToolsDragData,
    pub modifiers: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsSynthesizeTapGestureCommand {
    pub context: DevToolsCommandContext,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DevToolsResultOwnership {
    ByValue,
    Root,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevToolsNetworkInterceptPhase {
    BeforeRequestSent,
    ResponseStarted,
    AuthRequired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsNetworkInterceptPattern {
    pub url_pattern: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsAddNetworkInterceptCommand {
    pub context: DevToolsCommandContext,
    pub intercept_id: DevToolsNetworkInterceptId,
    pub phases: Vec<DevToolsNetworkInterceptPhase>,
    pub url_patterns: Vec<DevToolsNetworkInterceptPattern>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsRemoveNetworkInterceptCommand {
    pub context: DevToolsCommandContext,
    pub intercept_id: DevToolsNetworkInterceptId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsSetCacheBehaviorCommand {
    pub context: DevToolsCommandContext,
    pub target_ids: Vec<DevToolsTargetId>,
    pub cache_disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsSetExtraHeadersCommand {
    pub context: DevToolsCommandContext,
    pub target_ids: Vec<DevToolsTargetId>,
    pub browser_context_ids: Vec<DevToolsBrowserContextId>,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DevToolsNetworkDataType {
    Request,
    Response,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsAddNetworkDataCollectorCommand {
    pub context: DevToolsCommandContext,
    pub collector_id: DevToolsNetworkDataCollectorId,
    pub data_types: Vec<DevToolsNetworkDataType>,
    pub max_encoded_data_size: u64,
    pub target_ids: Vec<DevToolsTargetId>,
    pub browser_context_ids: Vec<DevToolsBrowserContextId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsRemoveNetworkDataCollectorCommand {
    pub context: DevToolsCommandContext,
    pub collector_id: DevToolsNetworkDataCollectorId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsDisownNetworkDataCommand {
    pub context: DevToolsCommandContext,
    pub request_id: DevToolsRequestId,
    pub data_type: DevToolsNetworkDataType,
    pub collector_id: DevToolsNetworkDataCollectorId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetNetworkDataCommand {
    pub context: DevToolsCommandContext,
    pub request_id: DevToolsRequestId,
    pub data_type: DevToolsNetworkDataType,
    pub collector: Option<DevToolsNetworkDataCollectorId>,
    pub disown: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsContinueInterceptedRequestCommand {
    pub context: DevToolsCommandContext,
    pub request_id: DevToolsRequestId,
    pub url: Option<String>,
    pub method: Option<String>,
    pub post_data: Option<String>,
    pub headers: Option<Vec<(String, String)>>,
    pub intercept_response: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsContinueInterceptedResponseCommand {
    pub context: DevToolsCommandContext,
    pub request_id: DevToolsRequestId,
    pub response_code: Option<u16>,
    pub response_headers: Option<Vec<(String, String)>>,
    pub response_phrase: Option<String>,
    pub auth_credentials: Option<DevToolsAuthCredentials>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsAuthCredentials {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevToolsAuthChallengeAction {
    Default,
    Cancel,
    ProvideCredentials,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsContinueWithAuthCommand {
    pub context: DevToolsCommandContext,
    pub request_id: DevToolsRequestId,
    pub action: DevToolsAuthChallengeAction,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsFailInterceptedRequestCommand {
    pub context: DevToolsCommandContext,
    pub request_id: DevToolsRequestId,
    pub error_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsFulfillInterceptedRequestCommand {
    pub context: DevToolsCommandContext,
    pub request_id: DevToolsRequestId,
    pub response_code: u16,
    pub response_headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub response_phrase: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DevToolsCommandResult {
    Empty,
    Navigate(DevToolsNavigateResult),
    TraverseHistory(DevToolsTraverseHistoryResult),
    GetNavigationHistory(DevToolsGetNavigationHistoryResult),
    GetFrameTree(DevToolsGetFrameTreeResult),
    GetFrameTrees(DevToolsGetFrameTreesResult),
    CreateTarget(DevToolsCreateTargetResult),
    CloseTarget(DevToolsCloseTargetResult),
    GetTargets(DevToolsGetTargetsResult),
    ServiceWorkerLogs(DevToolsGetServiceWorkerLogsResult),
    ClientWindows(DevToolsGetClientWindowsResult),
    ClientWindow(DevToolsSetClientWindowStateResult),
    CreateBrowserContext(DevToolsCreateBrowserContextResult),
    GetBrowserContexts(DevToolsGetBrowserContextsResult),
    GetTargetInfo(DevToolsGetTargetInfoResult),
    GetCookies(DevToolsGetCookiesResult),
    DeleteCookies(DevToolsDeleteCookiesResult),
    SetCookies(DevToolsSetCookiesResult),
    AddPreloadScript(DevToolsAddPreloadScriptResult),
    AddNetworkIntercept(DevToolsAddNetworkInterceptResult),
    AddNetworkDataCollector(DevToolsAddNetworkDataCollectorResult),
    NetworkData(DevToolsNetworkDataResult),
    Realms(DevToolsGetRealmsResult),
    Script(Box<DevToolsScriptResult>),
    LocateNodes(DevToolsLocateNodesResult),
    DescribeNode(DevToolsDescribeNodeResult),
    GetFrameOwner(DevToolsGetFrameOwnerResult),
    QuerySelector(DevToolsQuerySelectorResult),
    ResolveNode(DevToolsResolveNodeResult),
    GetAttributes(DevToolsGetAttributesResult),
    GetText(DevToolsGetTextResult),
    GetProperty(DevToolsGetPropertyResult),
    PushNodesByBackendIds(DevToolsPushNodesByBackendIdsResult),
    GetOuterHtml(DevToolsGetOuterHtmlResult),
    GetNodeForLocation(DevToolsGetNodeForLocationResult),
    DomGeometry(DevToolsDomGeometryResult),
    LayoutMetrics(DevToolsLayoutMetricsResult),
    JavaScriptDialog(DevToolsJavaScriptDialogResult),
    CaptureScreenshot(DevToolsCaptureScreenshotResult),
}

/// Protocol-neutral result of rasterizing one page viewport.
///
/// The renderer and core layers retain owned encoded bytes. Protocol-specific
/// transports decide how those bytes are projected (CDP uses standard base64).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsCaptureScreenshotResult {
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub bytes: Arc<[u8]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsNavigateResult {
    pub navigation_id: Option<DevToolsNavigationId>,
    pub frame_id: Option<DevToolsFrameId>,
    pub loader_id: Option<DevToolsLoaderId>,
    pub url: String,
    pub error_text: Option<String>,
    pub is_download: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsTraverseHistoryResult {
    pub same_document: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsNavigationHistoryEntry {
    pub id: i32,
    pub url: String,
    pub user_typed_url: String,
    pub title: String,
    pub transition_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetNavigationHistoryResult {
    pub current_index: usize,
    pub entries: Vec<DevToolsNavigationHistoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsJavaScriptDialogResult {
    pub dialog_type: String,
    pub message: String,
    pub default_prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsCreateTargetResult {
    pub target_id: DevToolsTargetId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsCloseTargetResult {
    pub success: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetTargetsResult {
    pub targets: Vec<DevToolsTargetInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsGetServiceWorkerLogsResult {
    pub entries: Vec<RuntimeConsoleEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsClientWindowInfo {
    pub client_window: DevToolsTargetId,
    pub active: bool,
    pub state: DevToolsWindowState,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetClientWindowsResult {
    pub client_windows: Vec<DevToolsClientWindowInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsSetClientWindowStateResult {
    pub client_window: DevToolsClientWindowInfo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsCreateBrowserContextResult {
    pub browser_context_id: DevToolsBrowserContextId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetBrowserContextsResult {
    pub browser_context_ids: Vec<DevToolsBrowserContextId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetTargetInfoResult {
    pub target_info: DevToolsTargetInfo,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsGetFrameTreeResult {
    pub frame_tree: Value,
    pub target_info: Option<DevToolsTargetInfo>,
    pub max_depth: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsGetFrameTreesResult {
    pub frame_trees: Vec<DevToolsGetFrameTreeResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsTargetInfo {
    pub target_id: Option<DevToolsTargetId>,
    pub kind: DevToolsTargetKind,
    pub title: String,
    pub url: String,
    pub attached: bool,
    pub opener_id: Option<DevToolsTargetId>,
    pub opener_frame_id: Option<DevToolsFrameId>,
    /// Whether the target's Window can script its opener.
    ///
    /// This is deliberately independent from `opener_id`. Chromium retains
    /// the DevTools creator relationship for an implicit-noopener
    /// `target=_blank` target while reporting `canAccessOpener: false`.
    pub can_access_opener: bool,
    pub browser_context_id: Option<DevToolsBrowserContextId>,
    pub moli_popup_id: Option<u64>,
}

impl DevToolsTargetInfo {
    pub(crate) fn into_cdp_value(self) -> Value {
        let mut payload = json!({
            "type": self.kind.as_cdp_type(),
            "title": self.title,
            "url": self.url,
            "attached": self.attached,
            "canAccessOpener": self.can_access_opener,
        });
        if let Some(target_id) = self.target_id {
            payload["targetId"] = json!(target_id.into_string());
        }
        if let Some(opener_id) = self.opener_id {
            payload["openerId"] = json!(opener_id.into_string());
        }
        if let Some(opener_frame_id) = self.opener_frame_id {
            payload["openerFrameId"] = json!(opener_frame_id.into_string());
        }
        if let Some(browser_context_id) = self.browser_context_id {
            payload["browserContextId"] = json!(browser_context_id.into_string());
        }
        payload
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsGetCookiesResult {
    pub cookies: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsDeleteCookiesResult {
    pub partition_key: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsSetCookiesResult {
    pub success: bool,
    pub cookie_reports: Vec<Value>,
    pub partition_key: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsQuerySelectorResult {
    pub node_ids: Vec<u32>,
    pub multiple: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsResolveNodeResult {
    pub object: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsLocateNodesResult {
    pub nodes: Vec<DevToolsRemoteValue>,
    pub node_ids: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsDescribeNodeResult {
    pub node: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetFrameOwnerResult {
    pub node_id: u32,
    pub backend_node_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsDomAttribute {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetAttributesResult {
    pub attributes: Vec<DevToolsDomAttribute>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetTextResult {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsGetPropertyResult {
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsGetOuterHtmlResult {
    pub outer_html: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsDomQuad {
    pub points: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsDomBoxModel {
    pub content: DevToolsDomQuad,
    pub padding: DevToolsDomQuad,
    pub border: DevToolsDomQuad,
    pub margin: DevToolsDomQuad,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsDomGeometryResult {
    pub box_model: Option<DevToolsDomBoxModel>,
    pub quads: Vec<DevToolsDomQuad>,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsLayoutMetricsResult {
    pub layout_viewport_width: u32,
    pub layout_viewport_height: u32,
    pub page_x: f64,
    pub page_y: f64,
    pub content_width: f64,
    pub content_height: f64,
    pub device_pixel_ratio: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsAddPreloadScriptResult {
    pub script_id: DevToolsPreloadScriptId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsAddNetworkInterceptResult {
    pub intercept_id: DevToolsNetworkInterceptId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsAddNetworkDataCollectorResult {
    pub collector_id: DevToolsNetworkDataCollectorId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevToolsNetworkDataBytesType {
    String,
    Base64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsNetworkDataResult {
    pub bytes_type: DevToolsNetworkDataBytesType,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsGetRealmsResult {
    pub realms: Vec<RuntimeExecutionContextEvent>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DevToolsScriptResult {
    Value(DevToolsRemoteValue),
    Exception(DevToolsScriptException),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsRemoteValue {
    pub value: Value,
    pub handle: Option<DevToolsRemoteHandleId>,
    pub shared_id: Option<DevToolsRemoteHandleId>,
    pub node_id: Option<u32>,
    pub backend_node_id: Option<u32>,
    pub window_context: Option<Box<DevToolsTargetId>>,
    pub realm: Option<DevToolsRealmId>,
    pub remote_type: Option<String>,
    pub remote_subtype: Option<String>,
    pub unserializable_value: Option<String>,
    pub description: Option<String>,
    pub class_name: Option<String>,
    pub deep_serialized_value: Option<Value>,
    pub node_value: Option<Value>,
}

impl DevToolsRemoteValue {
    pub fn from_json_value(value: Value) -> Self {
        Self {
            value,
            handle: None,
            shared_id: None,
            node_id: None,
            backend_node_id: None,
            window_context: None,
            realm: None,
            remote_type: None,
            remote_subtype: None,
            unserializable_value: None,
            description: None,
            class_name: None,
            deep_serialized_value: None,
            node_value: None,
        }
    }

    pub(crate) fn from_cdp_remote_object(
        remote: &Value,
        retain_handle: bool,
        realm: Option<DevToolsRealmId>,
    ) -> Self {
        let object_id = remote.get("objectId").and_then(Value::as_str);
        Self {
            value: cdp_remote_object_value(remote),
            handle: retain_handle
                .then(|| object_id.map(DevToolsRemoteHandleId::from))
                .flatten(),
            shared_id: object_id.map(DevToolsRemoteHandleId::from),
            node_id: None,
            backend_node_id: None,
            window_context: None,
            realm,
            remote_type: remote
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_owned),
            remote_subtype: remote
                .get("subtype")
                .and_then(Value::as_str)
                .map(str::to_owned),
            unserializable_value: remote
                .get("unserializableValue")
                .and_then(Value::as_str)
                .map(str::to_owned),
            description: remote
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_owned),
            class_name: remote
                .get("className")
                .and_then(Value::as_str)
                .map(str::to_owned),
            deep_serialized_value: remote.get("deepSerializedValue").cloned(),
            node_value: None,
        }
    }
}

fn cdp_remote_object_value(remote: &Value) -> Value {
    if let Some(value) = remote.get("value") {
        return value.clone();
    }
    if remote
        .get("subtype")
        .and_then(Value::as_str)
        .is_some_and(|subtype| subtype == "null")
    {
        return Value::Null;
    }
    if let Some(unserializable) = remote.get("unserializableValue").and_then(Value::as_str) {
        return Value::String(unserializable.to_owned());
    }
    match remote.get("type").and_then(Value::as_str) {
        Some("undefined") => Value::Null,
        Some("boolean") => Value::Bool(false),
        Some("number") => json!(0),
        Some("string") => Value::String(String::new()),
        Some("object") | Some("function") => json!({}),
        _ => Value::Null,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsBidiChannelProperties {
    pub channel: String,
    pub ownership: DevToolsResultOwnership,
    pub serialization_options: Option<DevToolsSerializationOptions>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsStackTrace {
    pub call_frames: Vec<DevToolsStackCallFrame>,
}

impl DevToolsStackTrace {
    pub fn from_cdp_value(value: &Value) -> Option<Self> {
        let call_frames = value
            .get("callFrames")
            .and_then(Value::as_array)?
            .iter()
            .map(|frame| DevToolsStackCallFrame {
                function_name: frame
                    .get("functionName")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                script_id: frame
                    .get("scriptId")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                url: frame
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                line_number: frame
                    .get("lineNumber")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                column_number: frame
                    .get("columnNumber")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
            })
            .collect();
        Some(Self { call_frames })
    }

    pub fn to_cdp_value(&self) -> Value {
        json!({
            "callFrames": self
                .call_frames
                .iter()
                .map(DevToolsStackCallFrame::to_cdp_value)
                .collect::<Vec<_>>()
        })
    }

    pub fn into_cdp_value(self) -> Value {
        json!({
            "callFrames": self
                .call_frames
                .into_iter()
                .map(DevToolsStackCallFrame::into_cdp_value)
                .collect::<Vec<_>>()
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsStackCallFrame {
    pub function_name: String,
    pub script_id: Option<String>,
    pub url: String,
    pub line_number: u64,
    pub column_number: u64,
}

impl DevToolsStackCallFrame {
    pub fn to_cdp_value(&self) -> Value {
        json!({
            "functionName": self.function_name,
            "scriptId": self.script_id.as_deref().unwrap_or_default(),
            "url": self.url,
            "lineNumber": self.line_number,
            "columnNumber": self.column_number,
        })
    }

    pub fn into_cdp_value(self) -> Value {
        json!({
            "functionName": self.function_name,
            "scriptId": self.script_id.unwrap_or_default(),
            "url": self.url,
            "lineNumber": self.line_number,
            "columnNumber": self.column_number,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DevToolsScriptException {
    pub exception_id: Option<u64>,
    pub script_id: Option<String>,
    pub text: String,
    pub value: Option<DevToolsRemoteValue>,
    pub realm: Option<DevToolsRealmId>,
    pub line_number: Option<u64>,
    pub column_number: Option<u64>,
    pub stack_trace: Option<DevToolsStackTrace>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevToolsErrorKind {
    InvalidArgument,
    InvalidSelector,
    NoSuchAlert,
    NoSuchHandle,
    NoSuchHistoryEntry,
    NoSuchNode,
    NoSuchNetworkCollector,
    NoSuchNetworkData,
    NoSuchRequest,
    NoSuchScript,
    NoSuchTarget,
    NoSuchSession,
    NavigationChangingDocument,
    Timeout,
    UnableToCaptureScreen,
    UnableToSetFileInput,
    Unsupported,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsError {
    pub kind: DevToolsErrorKind,
    pub message: String,
}

impl DevToolsError {
    pub fn new(kind: DevToolsErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AutomationEvent {
    TargetCreated(TargetLifecycleEvent),
    TargetDestroyed(TargetLifecycleEvent),
    TargetAttached(TargetAttachmentEvent),
    TargetDetached(TargetDetachmentEvent),
    NavigationFrame(NavigationFrameEvent),
    NavigationStarted(NavigationLifecycleEvent),
    DomContentLoaded(NavigationLifecycleEvent),
    Load(NavigationLifecycleEvent),
    PageLifecycle(PageLifecycleEvent),
    SameDocumentNavigation(SameDocumentNavigationEvent),
    LogEntryAdded(LogEntryEvent),
    RuntimeConsoleApiCalled(RuntimeConsoleEvent),
    RuntimeExecutionContextCreated(RuntimeExecutionContextEvent),
    RuntimeExecutionContextDestroyed(RuntimeExecutionContextEvent),
    RuntimeExecutionContextsCleared(RuntimeExecutionContextsClearedEvent),
    UserPromptClosed(UserPromptClosedEvent),
    PageJavaScriptDialogOpening(PageJavaScriptDialogOpeningEvent),
    PageFileChooserOpened(PageFileChooserOpenedEvent),
    BrowserDownloadWillBegin(BrowserDownloadWillBeginEvent),
    BrowserDownloadProgress(BrowserDownloadProgressEvent),
    DomSetChildNodes(DomSetChildNodesEvent),
    ScriptMessage(ScriptMessageEvent),
    ScriptException(ScriptExceptionEvent),
    NetworkBeforeRequestSent(NetworkRequestEvent),
    NetworkResponseStarted(NetworkRequestEvent),
    NetworkResponseCompleted(NetworkRequestEvent),
    NetworkFetchError(NetworkRequestEvent),
    NetworkAuthRequired(NetworkRequestEvent),
    RequestPaused(NetworkRequestEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageJavaScriptDialogOpeningEvent {
    pub frame_id: Option<DevToolsFrameId>,
    pub url: String,
    pub message: String,
    pub dialog_type: String,
    pub has_browser_handler: bool,
    pub default_prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageFileChooserOpenedEvent {
    pub frame_id: DevToolsFrameId,
    pub mode: String,
    pub backend_node_id: u32,
    pub element_shared_id: Option<DevToolsRemoteHandleId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserDownloadWillBeginEvent {
    pub frame_id: DevToolsFrameId,
    pub guid: String,
    pub url: String,
    pub suggested_filename: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserDownloadProgressEvent {
    pub guid: String,
    pub state: String,
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub file_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DomSetChildNodesEvent {
    pub parent_node_id: u32,
    pub nodes: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserPromptClosedEvent {
    pub target_id: Option<DevToolsTargetId>,
    pub frame_id: DevToolsFrameId,
    pub prompt_type: String,
    pub accepted: bool,
    pub user_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetLifecycleEvent {
    pub target_id: DevToolsTargetId,
    pub browser_context_id: Option<DevToolsBrowserContextId>,
    pub kind: DevToolsTargetKind,
    pub url: String,
    pub target_info: Option<DevToolsTargetInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TargetAttachmentEvent {
    pub target_id: DevToolsTargetId,
    pub session_id: DevToolsSessionId,
    pub parent_session_id: Option<DevToolsSessionId>,
    pub target_info: DevToolsTargetInfo,
    pub waiting_for_debugger: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetDetachmentEvent {
    pub target_id: DevToolsTargetId,
    pub session_id: DevToolsSessionId,
    pub parent_session_id: Option<DevToolsSessionId>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NavigationFrameEventKind {
    Scheduled,
    Requested,
    StartedNavigating,
    StartedLoading,
    ClearedScheduled,
    Navigated,
    DocumentUpdated,
    StoppedLoading,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationFrameEvent {
    pub target_id: DevToolsTargetId,
    pub frame_id: DevToolsFrameId,
    pub parent_frame_id: Option<DevToolsFrameId>,
    pub loader_id: Option<DevToolsLoaderId>,
    pub url: String,
    pub kind: NavigationFrameEventKind,
    pub frame_name: Option<String>,
    pub security_origin: Option<String>,
    pub secure_context_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NavigationLifecycleEvent {
    pub target_id: DevToolsTargetId,
    pub frame_id: DevToolsFrameId,
    pub navigation_id: Option<DevToolsNavigationId>,
    pub loader_id: Option<DevToolsLoaderId>,
    pub url: String,
    pub timestamp: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PageLifecycleEvent {
    pub target_id: DevToolsTargetId,
    pub frame_id: DevToolsFrameId,
    pub loader_id: DevToolsLoaderId,
    pub name: String,
    pub timestamp: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SameDocumentNavigationEvent {
    pub target_id: DevToolsTargetId,
    pub frame_id: DevToolsFrameId,
    pub url: String,
    pub navigation_type: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogEntryEvent {
    pub target_id: Option<DevToolsTargetId>,
    pub source: String,
    pub level: String,
    pub text: String,
    pub url: Option<String>,
    pub timestamp: Option<f64>,
    pub network_request_id: Option<String>,
    pub args: Vec<DevToolsRemoteValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeConsoleEvent {
    pub target_id: Option<DevToolsTargetId>,
    pub console_type: String,
    pub text: String,
    pub args: Vec<Value>,
    pub stack: Option<String>,
    pub stack_trace: Option<DevToolsStackTrace>,
    pub execution_context_id: Option<i64>,
    pub timestamp: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeExecutionContextEvent {
    pub target_id: Option<DevToolsTargetId>,
    pub context_id: Option<i64>,
    pub realm_id: Option<DevToolsRealmId>,
    pub frame_id: Option<DevToolsFrameId>,
    pub origin: Option<String>,
    pub name: Option<String>,
    pub is_default: Option<bool>,
    pub context_type: Option<String>,
    pub grant_universal_access: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeExecutionContextsClearedEvent {
    pub target_id: Option<DevToolsTargetId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScriptMessageEvent {
    pub target_id: Option<DevToolsTargetId>,
    pub realm_id: Option<DevToolsRealmId>,
    pub channel: String,
    pub data: DevToolsRemoteValue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScriptExceptionEvent {
    pub target_id: Option<DevToolsTargetId>,
    pub url: Option<String>,
    pub execution_context_id: Option<i64>,
    pub exception_index: Option<usize>,
    pub timestamp: Option<f64>,
    pub exception: Box<DevToolsScriptException>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NetworkRedirectResponseEvent {
    pub url: String,
    pub status: u16,
    pub status_text: Option<String>,
    pub response_headers: Vec<(String, String)>,
    pub encoded_data_length: usize,
    pub from_cache: bool,
    pub response_protocol: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAuthChallengeEvent {
    pub origin: String,
    pub source: String,
    pub scheme: String,
    pub realm: String,
}

/// The resource classification carried by protocol-neutral network events.
///
/// Renderer and fetch internals keep their more precise loader-facing enums.
/// This enum represents the observable DevTools classification and is only
/// converted to a CDP token when an event is serialized at the protocol edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DevToolsNetworkResourceType {
    Document,
    Stylesheet,
    Image,
    Media,
    Font,
    Script,
    TextTrack,
    Xhr,
    Fetch,
    Prefetch,
    EventSource,
    WebSocket,
    Manifest,
    SignedExchange,
    Ping,
    CspViolationReport,
    Preflight,
    FedCm,
    Other,
}

impl DevToolsNetworkResourceType {
    pub const fn as_cdp_type(self) -> &'static str {
        match self {
            Self::Document => "Document",
            Self::Stylesheet => "Stylesheet",
            Self::Image => "Image",
            Self::Media => "Media",
            Self::Font => "Font",
            Self::Script => "Script",
            Self::TextTrack => "TextTrack",
            Self::Xhr => "XHR",
            Self::Fetch => "Fetch",
            Self::Prefetch => "Prefetch",
            Self::EventSource => "EventSource",
            Self::WebSocket => "WebSocket",
            Self::Manifest => "Manifest",
            Self::SignedExchange => "SignedExchange",
            Self::Ping => "Ping",
            Self::CspViolationReport => "CSPViolationReport",
            Self::Preflight => "Preflight",
            Self::FedCm => "FedCM",
            Self::Other => "Other",
        }
    }

    pub fn from_cdp_type(value: &str) -> Option<Self> {
        Some(match value {
            "Document" => Self::Document,
            "Stylesheet" => Self::Stylesheet,
            "Image" => Self::Image,
            "Media" => Self::Media,
            "Font" => Self::Font,
            "Script" => Self::Script,
            "TextTrack" => Self::TextTrack,
            "XHR" => Self::Xhr,
            "Fetch" => Self::Fetch,
            "Prefetch" => Self::Prefetch,
            "EventSource" => Self::EventSource,
            "WebSocket" => Self::WebSocket,
            "Manifest" => Self::Manifest,
            "SignedExchange" => Self::SignedExchange,
            "Ping" => Self::Ping,
            "CSPViolationReport" => Self::CspViolationReport,
            "Preflight" => Self::Preflight,
            "FedCM" => Self::FedCm,
            "Other" => Self::Other,
            _ => return None,
        })
    }

    pub fn from_fetch_interception_type(value: SubresourceResourceType) -> Self {
        match value {
            SubresourceResourceType::Fetch
            | SubresourceResourceType::EventSource
            | SubresourceResourceType::Xhr => Self::Xhr,
            _ => value.into(),
        }
    }
}

impl From<SubresourceResourceType> for DevToolsNetworkResourceType {
    fn from(value: SubresourceResourceType) -> Self {
        match value {
            SubresourceResourceType::Script => Self::Script,
            SubresourceResourceType::Stylesheet => Self::Stylesheet,
            SubresourceResourceType::Image => Self::Image,
            SubresourceResourceType::Font => Self::Font,
            SubresourceResourceType::Audio
            | SubresourceResourceType::Video
            | SubresourceResourceType::Media => Self::Media,
            SubresourceResourceType::TextTrack => Self::TextTrack,
            SubresourceResourceType::Fetch => Self::Fetch,
            SubresourceResourceType::EventSource => Self::EventSource,
            SubresourceResourceType::Xhr => Self::Xhr,
            SubresourceResourceType::Ping => Self::Ping,
            SubresourceResourceType::CspReport => Self::CspViolationReport,
            SubresourceResourceType::Dictionary => Self::Other,
            SubresourceResourceType::Manifest => Self::Manifest,
            SubresourceResourceType::WebSocket => Self::WebSocket,
        }
    }
}

#[cfg(test)]
mod network_resource_type_tests {
    use super::{DevToolsNetworkResourceType, SubresourceResourceType};

    #[test]
    fn devtools_network_resource_types_round_trip_cdp_tokens() {
        for (resource_type, token) in [
            (DevToolsNetworkResourceType::Document, "Document"),
            (DevToolsNetworkResourceType::Stylesheet, "Stylesheet"),
            (DevToolsNetworkResourceType::Image, "Image"),
            (DevToolsNetworkResourceType::Media, "Media"),
            (DevToolsNetworkResourceType::Font, "Font"),
            (DevToolsNetworkResourceType::Script, "Script"),
            (DevToolsNetworkResourceType::TextTrack, "TextTrack"),
            (DevToolsNetworkResourceType::Xhr, "XHR"),
            (DevToolsNetworkResourceType::Fetch, "Fetch"),
            (DevToolsNetworkResourceType::Prefetch, "Prefetch"),
            (DevToolsNetworkResourceType::EventSource, "EventSource"),
            (DevToolsNetworkResourceType::WebSocket, "WebSocket"),
            (DevToolsNetworkResourceType::Manifest, "Manifest"),
            (
                DevToolsNetworkResourceType::SignedExchange,
                "SignedExchange",
            ),
            (DevToolsNetworkResourceType::Ping, "Ping"),
            (
                DevToolsNetworkResourceType::CspViolationReport,
                "CSPViolationReport",
            ),
            (DevToolsNetworkResourceType::Preflight, "Preflight"),
            (DevToolsNetworkResourceType::FedCm, "FedCM"),
            (DevToolsNetworkResourceType::Other, "Other"),
        ] {
            assert_eq!(resource_type.as_cdp_type(), token);
            assert_eq!(
                DevToolsNetworkResourceType::from_cdp_type(token),
                Some(resource_type)
            );
        }
        assert_eq!(
            DevToolsNetworkResourceType::from_cdp_type("FutureResourceType"),
            None
        );
    }

    #[test]
    fn subresource_resource_types_convert_without_stringly_typed_intermediate_state() {
        assert_eq!(
            DevToolsNetworkResourceType::from(SubresourceResourceType::Audio),
            DevToolsNetworkResourceType::Media
        );
        assert_eq!(
            DevToolsNetworkResourceType::from(SubresourceResourceType::Dictionary),
            DevToolsNetworkResourceType::Other
        );
        for resource_type in [
            SubresourceResourceType::Fetch,
            SubresourceResourceType::EventSource,
            SubresourceResourceType::Xhr,
        ] {
            assert_eq!(
                DevToolsNetworkResourceType::from_fetch_interception_type(resource_type),
                DevToolsNetworkResourceType::Xhr
            );
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NetworkRequestEvent {
    pub target_id: DevToolsTargetId,
    pub frame_id: Option<DevToolsFrameId>,
    pub request_id: DevToolsRequestId,
    pub loader_id: Option<DevToolsLoaderId>,
    pub network_id: Option<DevToolsRequestId>,
    pub url: String,
    pub document_url: Option<String>,
    pub method: Option<String>,
    pub request_headers: Vec<(String, String)>,
    pub request_body: Option<String>,
    pub request_initiator_type: Option<String>,
    pub bidi_request_initiator_type: Option<String>,
    pub redirect_response: Option<NetworkRedirectResponseEvent>,
    pub redirect_has_extra_info: bool,
    pub request_cookie_report: Option<StoredCookieQueryReport>,
    pub resource_type: Option<DevToolsNetworkResourceType>,
    pub timestamp: Option<f64>,
    pub wall_time: Option<f64>,
    pub status: Option<u16>,
    pub status_text: Option<String>,
    pub response_headers: Vec<(String, String)>,
    pub response_mime_type: Option<String>,
    pub response_protocol: Option<String>,
    pub encoded_data_length: Option<usize>,
    pub from_cache: bool,
    pub has_extra_info: bool,
    pub error_text: Option<String>,
    pub loading_failed_canceled: bool,
    pub blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
    pub fetch_request_id: Option<DevToolsFetchRequestId>,
    pub auth_challenge: Option<NetworkAuthChallengeEvent>,
}
