#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum AccessibilityAction {
    Enable,
    Disable,
    #[strum(serialize = "getFullAXTree")]
    GetFullAxTree,
    #[strum(serialize = "getRootAXNode")]
    GetRootAxNode,
    #[strum(serialize = "getChildAXNodes")]
    GetChildAxNodes,
    #[strum(serialize = "getAXNodeAndAncestors")]
    GetAxNodeAndAncestors,
    #[strum(serialize = "queryAXTree")]
    QueryAxTree,
    #[strum(serialize = "getPartialAXTree")]
    GetPartialAxTree,
}

impl AccessibilityAction {
    pub(crate) fn queries_tree(self) -> bool {
        matches!(
            self,
            Self::GetFullAxTree
                | Self::GetRootAxNode
                | Self::GetChildAxNodes
                | Self::GetAxNodeAndAncestors
                | Self::QueryAxTree
                | Self::GetPartialAxTree
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum AuditsAction {
    Enable,
    Disable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum AutofillAction {
    Trigger,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum BrowserAction {
    GetVersion,
    GetWindowForTarget,
    SetWindowBounds,
    SetDownloadBehavior,
    CancelDownload,
    OpenDownloadAsStream,
    SetPermission,
    GrantPermissions,
    ResetPermissions,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum ConsoleAction {
    Enable,
    Disable,
    ClearMessages,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum CssAction {
    Enable,
    Disable,
    GetStyleSheet,
    SetStyleSheetText,
    GetComputedStyleForNode,
    GetInlineStylesForNode,
    GetMatchedStylesForNode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum DomAction {
    Enable,
    Disable,
    GetDocument,
    GetFlattenedDocument,
    QuerySelector,
    QuerySelectorAll,
    RequestChildNodes,
    GetFrameOwner,
    DescribeNode,
    GetAttributes,
    PushNodesByBackendIdsToFrontend,
    RequestNode,
    ResolveNode,
    Focus,
    SetAttributeValue,
    SetAttributesAsText,
    RemoveAttribute,
    RemoveNode,
    MoveTo,
    SetNodeName,
    SetNodeValue,
    #[strum(serialize = "setOuterHTML")]
    SetOuterHtml,
    #[strum(serialize = "getOuterHTML")]
    GetOuterHtml,
    GetBoxModel,
    GetContentQuads,
    GetNodeForLocation,
    ScrollIntoViewIfNeeded,
    SetFileInputFiles,
    PerformSearch,
    DiscardSearchResults,
    GetSearchResults,
    SetNodeStackTracesEnabled,
    GetNodeStackTraces,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum DomDebuggerAction {
    GetEventListeners,
    #[strum(serialize = "removeDOMBreakpoint")]
    RemoveDOMBreakpoint,
    RemoveEventListenerBreakpoint,
    #[strum(serialize = "removeXHRBreakpoint")]
    RemoveXHRBreakpoint,
    SetEventListenerBreakpoint,
    #[strum(serialize = "setDOMBreakpoint")]
    SetDOMBreakpoint,
    #[strum(serialize = "setXHRBreakpoint")]
    SetXHRBreakpoint,
}

impl DomAction {
    pub(crate) fn requires_document_access(self) -> bool {
        !matches!(
            self,
            Self::Enable | Self::Disable | Self::DiscardSearchResults | Self::GetNodeForLocation
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum DomSnapshotAction {
    Enable,
    Disable,
    CaptureSnapshot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum DomStorageAction {
    Enable,
    Disable,
    Clear,
    #[strum(serialize = "getDOMStorageItems")]
    GetDomStorageItems,
    #[strum(serialize = "removeDOMStorageItem")]
    RemoveDomStorageItem,
    #[strum(serialize = "setDOMStorageItem")]
    SetDomStorageItem,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum EmulationAction {
    Enable,
    Disable,
    SetFocusEmulationEnabled,
    SetDeviceMetricsOverride,
    ClearDeviceMetricsOverride,
    #[strum(serialize = "setCPUThrottlingRate")]
    SetCpuThrottlingRate,
    SetTouchEmulationEnabled,
    SetHardwareConcurrencyOverride,
    SetDataSaverOverride,
    SetAutomationOverride,
    SetEmitTouchEventsForMouse,
    SetScriptExecutionDisabled,
    SetScrollbarsHidden,
    SetGeolocationOverride,
    ClearGeolocationOverride,
    SetIdleOverride,
    ClearIdleOverride,
    SetLocaleOverride,
    SetTimezoneOverride,
    SetUserAgentOverride,
    SetEmulatedMedia,
    SetDefaultBackgroundColorOverride,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum FetchAction {
    Enable,
    Disable,
    ContinueRequest,
    ContinueWithAuth,
    FailRequest,
    FulfillRequest,
    ContinueResponse,
    DispatchWebSocketMessage,
    CloseWebSocket,
    GetResponseBody,
    TakeResponseBodyAsStream,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum HeapProfilerAction {
    AddInspectedHeapObject,
    Enable,
    Disable,
    CollectGarbage,
    GetHeapObjectId,
    GetObjectByHeapObjectId,
    GetSamplingProfile,
    StartSampling,
    StartTrackingHeapObjects,
    StopSampling,
    StopTrackingHeapObjects,
    TakeHeapSnapshot,
    MoliDiagnostics,
    MoliResetIdleEngine,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum InputAction {
    CancelDragging,
    SetInterceptDrags,
    SetIgnoreInputEvents,
    DispatchDragEvent,
    DispatchMouseEvent,
    DispatchKeyEvent,
    InsertText,
    DispatchTouchEvent,
    EmulateTouchFromMouseEvent,
    SynthesizeTapGesture,
}

impl InputAction {
    pub(crate) fn requires_document_access(self) -> bool {
        matches!(
            self,
            Self::DispatchDragEvent
                | Self::DispatchMouseEvent
                | Self::DispatchKeyEvent
                | Self::InsertText
                | Self::DispatchTouchEvent
                | Self::EmulateTouchFromMouseEvent
                | Self::SynthesizeTapGesture
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum InspectorAction {
    Enable,
    Disable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum IoAction {
    Read,
    Close,
    ResolveBlob,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum LogAction {
    Clear,
    Enable,
    Disable,
    StartViolationsReport,
    StopViolationsReport,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum NetworkAction {
    Enable,
    Disable,
    SetCacheDisabled,
    SetBypassServiceWorker,
    #[strum(serialize = "setExtraHTTPHeaders")]
    SetExtraHttpHeaders,
    #[strum(serialize = "setBlockedURLs")]
    SetBlockedUrls,
    EmulateNetworkConditions,
    SetUserAgentOverride,
    SetCookie,
    SetCookies,
    ClearBrowserCache,
    DeleteCookies,
    ClearBrowserCookies,
    GetCookies,
    GetAllCookies,
    GetResponseBody,
    GetRequestPostData,
    LoadNetworkResource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum PageAction {
    Enable,
    Disable,
    SetLifecycleEventsEnabled,
    #[strum(serialize = "setBypassCSP")]
    SetBypassCsp,
    SetFontFamilies,
    SetInterceptFileChooserDialog,
    HandleJavaScriptDialog,
    SetDownloadBehavior,
    StartScreencast,
    StopScreencast,
    ScreencastFrameAck,
    GetNavigationHistory,
    ResetNavigationHistory,
    BringToFront,
    CaptureScreenshot,
    CaptureSnapshot,
    #[strum(serialize = "printToPDF")]
    PrintToPdf,
    SetDocumentContent,
    GetFrameTree,
    GetResourceTree,
    GetAppManifest,
    SearchInResource,
    GetLayoutMetrics,
    Navigate,
    NavigateToHistoryEntry,
    Reload,
    StopLoading,
    Crash,
    Close,
    AddScriptToEvaluateOnNewDocument,
    RemoveScriptToEvaluateOnNewDocument,
    CreateIsolatedWorld,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum PerformanceAction {
    Enable,
    Disable,
    GetMetrics,
    SetTimeDomain,
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::IntoStaticStr, strum::VariantNames,
)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum RuntimeAction {
    Enable,
    Disable,
    AddBinding,
    RemoveBinding,
    CompileScript,
    DiscardConsoleEntries,
    RunIfWaitingForDebugger,
    TerminateExecution,
    RunScript,
    Evaluate,
    CallFunctionOn,
    GetProperties,
    AwaitPromise,
    QueryObjects,
    GlobalLexicalScopeNames,
    GetIsolateId,
    GetHeapUsage,
    GetExceptionDetails,
    ReleaseObject,
    ReleaseObjectGroup,
    SetAsyncCallStackDepth,
    SetCustomObjectFormatterEnabled,
    SetMaxCallStackSizeToCapture,
}

impl RuntimeAction {
    pub(crate) fn label(self) -> &'static str {
        self.into()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum SecurityAction {
    Enable,
    Disable,
    HandleCertificateError,
    SetOverrideCertificateErrors,
    SetIgnoreCertificateErrors,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum ServiceWorkerAction {
    DeliverPushMessage,
    Disable,
    DispatchPeriodicSyncEvent,
    DispatchSyncEvent,
    Enable,
    SetForceUpdateOnPageLoad,
    SkipWaiting,
    StartWorker,
    StopAllWorkers,
    StopWorker,
    Unregister,
    UpdateRegistration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum StorageAction {
    ClearDataForOrigin,
    ClearDataForStorageKey,
    GetUsageAndQuota,
    OverrideQuotaForOrigin,
    GetStorageKeyForFrame,
    RunBounceTrackingMitigations,
    ClearCookies,
    GetCookies,
    SetCookies,
    DeleteCookies,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum SystemInfoAction {
    GetInfo,
    GetProcessInfo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum TargetAction {
    GetTargets,
    GetBrowserContexts,
    CreateBrowserContext,
    CreateTarget,
    AttachToTarget,
    AttachToBrowserTarget,
    GetTargetInfo,
    SetDiscoverTargets,
    ActivateTarget,
    SetAutoAttach,
    AutoAttachRelated,
    DetachFromTarget,
    CloseTarget,
    DisposeBrowserContext,
    SendMessageToTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum TracingAction {
    Start,
    End,
    GetCategories,
    RecordClockSyncMarker,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum WebAuthnAction {
    Enable,
    Disable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub(crate) enum WebMcpAction {
    Enable,
    Disable,
    InvokeTool,
    CancelInvocation,
}

/// Domain -> command names the dispatcher accepts, generated from the action
/// enums above (they are what `EnumString` parses incoming methods against,
/// so this list cannot drift from the implementation).
///
/// `/json/protocol` used to serve only V8's `js_protocol` (Runtime, Debugger,
/// Profiler, HeapProfiler, Console, Schema), so clients that gate calls on the
/// advertised protocol refused Page / DOM / Network commands Moli implements.
pub fn implemented_cdp_commands() -> Vec<(&'static str, &'static [&'static str])> {
    use strum::VariantNames;
    vec![
        ("Accessibility", AccessibilityAction::VARIANTS),
        ("Audits", AuditsAction::VARIANTS),
        ("Autofill", AutofillAction::VARIANTS),
        ("Browser", BrowserAction::VARIANTS),
        ("Console", ConsoleAction::VARIANTS),
        ("CSS", CssAction::VARIANTS),
        ("DOM", DomAction::VARIANTS),
        ("DOMDebugger", DomDebuggerAction::VARIANTS),
        ("DOMSnapshot", DomSnapshotAction::VARIANTS),
        ("DOMStorage", DomStorageAction::VARIANTS),
        ("Emulation", EmulationAction::VARIANTS),
        ("Fetch", FetchAction::VARIANTS),
        ("HeapProfiler", HeapProfilerAction::VARIANTS),
        ("Input", InputAction::VARIANTS),
        ("Inspector", InspectorAction::VARIANTS),
        ("IO", IoAction::VARIANTS),
        ("Log", LogAction::VARIANTS),
        ("Network", NetworkAction::VARIANTS),
        ("Page", PageAction::VARIANTS),
        ("Performance", PerformanceAction::VARIANTS),
        ("Runtime", RuntimeAction::VARIANTS),
        ("Security", SecurityAction::VARIANTS),
        ("ServiceWorker", ServiceWorkerAction::VARIANTS),
        ("Storage", StorageAction::VARIANTS),
        ("SystemInfo", SystemInfoAction::VARIANTS),
        ("Target", TargetAction::VARIANTS),
        ("Tracing", TracingAction::VARIANTS),
        ("WebAuthn", WebAuthnAction::VARIANTS),
        ("WebMCP", WebMcpAction::VARIANTS),
    ]
}

#[cfg(test)]
mod implemented_commands_tests {
    use super::implemented_cdp_commands;

    #[test]
    fn variant_names_are_wire_method_names() {
        let all = implemented_cdp_commands();
        let get = |domain: &str| {
            all.iter()
                .find(|(d, _)| *d == domain)
                .map(|(_, c)| *c)
                .unwrap()
        };
        // Explicit `serialize` overrides and camelCase defaults must both come out as the CDP wire names.
        assert!(get("Accessibility").contains(&"getFullAXTree"));
        assert!(get("DOMStorage").contains(&"getDOMStorageItems"));
        assert!(get("Page").contains(&"navigate"));
        assert_eq!(all.len(), 29);
    }
}
