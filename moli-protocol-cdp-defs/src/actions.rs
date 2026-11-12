#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub enum AccessibilityAction {
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
    pub fn queries_tree(self) -> bool {
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
pub enum AuditsAction {
    Enable,
    Disable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub enum AutofillAction {
    Trigger,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub enum BrowserAction {
    GetVersion,
    GetWindowForTarget,
    GetWindowBounds,
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
pub enum ConsoleAction {
    Enable,
    Disable,
    ClearMessages,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub enum CssAction {
    Enable,
    Disable,
    GetStyleSheet,
    SetStyleSheetText,
    GetComputedStyleForNode,
    GetInlineStylesForNode,
    GetMatchedStylesForNode,
}

impl CssAction {
    pub fn requires_document_access(self) -> bool {
        !matches!(self, Self::Enable | Self::Disable)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub enum DomAction {
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
pub enum DomDebuggerAction {
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
    pub fn requires_document_access(self) -> bool {
        !matches!(
            self,
            Self::Enable | Self::Disable | Self::DiscardSearchResults | Self::GetNodeForLocation
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub enum DomSnapshotAction {
    Enable,
    Disable,
    CaptureSnapshot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub enum DomStorageAction {
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
pub enum EmulationAction {
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
pub enum FetchAction {
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
pub enum HeapProfilerAction {
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
pub enum InputAction {
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
    pub fn requires_document_access(self) -> bool {
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
pub enum InspectorAction {
    Enable,
    Disable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub enum IoAction {
    Read,
    Close,
    ResolveBlob,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub enum LogAction {
    Clear,
    Enable,
    Disable,
    StartViolationsReport,
    StopViolationsReport,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub enum NetworkAction {
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
pub enum PageAction {
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
pub enum PerformanceAction {
    Enable,
    Disable,
    GetMetrics,
    SetTimeDomain,
}

#[derive(
    Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::IntoStaticStr, strum::VariantNames,
)]
#[strum(serialize_all = "camelCase")]
pub enum RuntimeAction {
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
    pub fn label(self) -> &'static str {
        self.into()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub enum SecurityAction {
    Enable,
    Disable,
    HandleCertificateError,
    SetOverrideCertificateErrors,
    SetIgnoreCertificateErrors,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub enum ServiceWorkerAction {
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
pub enum StorageAction {
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
pub enum SystemInfoAction {
    GetInfo,
    GetProcessInfo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub enum TargetAction {
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
pub enum TracingAction {
    Start,
    End,
    GetCategories,
    RecordClockSyncMarker,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub enum WebAuthnAction {
    Enable,
    Disable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub enum WebMcpAction {
    Enable,
    Disable,
    InvokeTool,
    CancelInvocation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::EnumString, strum::VariantNames)]
#[strum(serialize_all = "camelCase")]
pub enum ProfilerAction {
    Enable,
    Disable,
    SetSamplingInterval,
    Start,
    Stop,
    StartPreciseCoverage,
    TakePreciseCoverage,
    StopPreciseCoverage,
    GetBestEffortCoverage,
}
