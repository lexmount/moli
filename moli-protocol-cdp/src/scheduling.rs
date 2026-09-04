/// How Chromium's generated domain handler completes a CDP command.
///
/// This is independent from the transport lane used to execute the command.
/// A callback command may still enter the renderer main-thread lane, while an
/// interrupt command may be synchronous on the renderer IO lane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CdpCommandCompletionSemantics {
    SynchronousResponse,
    AsyncCallback,
}

/// Chromium DevTools execution lane used by a CDP command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CdpCommandDispatchLane {
    /// The browser-side handler can run independently of the renderer's
    /// ordered DevTools main session.
    OwnerIndependent,
    /// The command enters the renderer's ordered DevTools main session.
    MainThread,
    /// The command enters Chromium's independent renderer IO session.
    Io,
}

/// Immutable frontend scheduling facts derived from a validated CDP method.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CdpCommandSchedulingPolicy {
    completion_semantics: CdpCommandCompletionSemantics,
    dispatch_lane: CdpCommandDispatchLane,
}

impl CdpCommandSchedulingPolicy {
    pub(crate) fn for_method(method: &str) -> Self {
        Self {
            completion_semantics: completion_semantics_for_method(method),
            dispatch_lane: dispatch_lane_for_method(method),
        }
    }

    pub const fn completion_semantics(self) -> CdpCommandCompletionSemantics {
        self.completion_semantics
    }

    pub const fn dispatch_lane(self) -> CdpCommandDispatchLane {
        self.dispatch_lane
    }
}

fn completion_semantics_for_method(method: &str) -> CdpCommandCompletionSemantics {
    if CHROMIUM_ASYNC_CALLBACK_METHODS
        .binary_search(&method)
        .is_ok()
    {
        CdpCommandCompletionSemantics::AsyncCallback
    } else {
        CdpCommandCompletionSemantics::SynchronousResponse
    }
}

fn dispatch_lane_for_method(method: &str) -> CdpCommandDispatchLane {
    if chromium_sends_on_io(method) {
        return CdpCommandDispatchLane::Io;
    }
    let Some((domain, action)) = method.split_once('.') else {
        return CdpCommandDispatchLane::OwnerIndependent;
    };
    match domain {
        // These domains are owned by Blink or V8 Inspector and enter the
        // renderer's ordered DevTools session.
        "Accessibility" | "Console" | "CSS" | "Debugger" | "DOMDebugger" | "DOMSnapshot"
        | "DOMStorage" | "HeapProfiler" | "Log" | "Performance" | "Profiler" | "Runtime" => {
            CdpCommandDispatchLane::MainThread
        }
        // content::protocol::DOMHandler consumes disable in the browser; its
        // other included method, setFileInputFiles, deliberately falls through
        // to Blink after granting file access.
        "DOM" if action == "disable" => CdpCommandDispatchLane::OwnerIndependent,
        "DOM" => CdpCommandDispatchLane::MainThread,
        // Layered domains are first offered to browser handlers. Only methods
        // which fall through (or are renderer-only) join the main session.
        "Page" => page_dispatch_lane(action),
        "Emulation" => emulation_dispatch_lane(action),
        "Network" => network_dispatch_lane(action),
        "IO" if action == "resolveBlob" => CdpCommandDispatchLane::MainThread,
        "Storage" => storage_dispatch_lane(action),
        _ => CdpCommandDispatchLane::OwnerIndependent,
    }
}

fn page_dispatch_lane(action: &str) -> CdpCommandDispatchLane {
    match action {
        // These commands are completed by the content/chrome Page handlers
        // without entering Blink's Page dispatcher.
        "bringToFront"
        | "captureScreenshot"
        | "captureSnapshot"
        | "close"
        | "getAppManifest"
        | "getNavigationHistory"
        | "handleJavaScriptDialog"
        | "navigate"
        | "navigateToHistoryEntry"
        | "printToPDF"
        | "reload"
        | "resetNavigationHistory"
        | "screencastFrameAck"
        | "setDownloadBehavior"
        | "startScreencast"
        | "stopLoading"
        | "stopScreencast" => CdpCommandDispatchLane::OwnerIndependent,
        // Known Page.crash is handled by chromium_sends_on_io above. Unknown
        // or renderer-owned Page methods follow Chromium's FallThrough path.
        _ => CdpCommandDispatchLane::MainThread,
    }
}

fn emulation_dispatch_lane(action: &str) -> CdpCommandDispatchLane {
    match action {
        // These content handlers finish entirely in the browser process.
        "clearGeolocationOverride"
        | "clearIdleOverride"
        | "disable"
        | "setEmitTouchEventsForMouse"
        | "setGeolocationOverride"
        | "setIdleOverride" => CdpCommandDispatchLane::OwnerIndependent,
        // Known setScriptExecutionDisabled is handled by
        // chromium_sends_on_io above. The remaining implemented methods are
        // renderer-only or return FallThrough from the content handler.
        _ => CdpCommandDispatchLane::MainThread,
    }
}

fn network_dispatch_lane(action: &str) -> CdpCommandDispatchLane {
    match action {
        // Browser-side callback/cache/cookie handlers may complete while the
        // renderer main session is occupied.
        "clearBrowserCache"
        | "clearBrowserCookies"
        | "continueInterceptedRequest"
        | "deleteCookies"
        | "getAllCookies"
        | "getCookies"
        | "getResponseBody"
        | "getResponseBodyForInterception"
        | "loadNetworkResource"
        | "setCookie"
        | "setCookies"
        | "takeResponseBodyForInterceptionAsStream" => CdpCommandDispatchLane::OwnerIndependent,
        // Network enable/configuration methods either live in Blink or return
        // FallThrough after updating their browser-side state.
        _ => CdpCommandDispatchLane::MainThread,
    }
}

fn storage_dispatch_lane(action: &str) -> CdpCommandDispatchLane {
    match action {
        "clearCookies"
        | "clearDataForOrigin"
        | "clearDataForStorageKey"
        | "deleteCookies"
        | "getCookies"
        | "getStorageKeyForFrame"
        | "getUsageAndQuota"
        | "overrideQuotaForOrigin"
        | "runBounceTrackingMitigations"
        | "setCookies" => CdpCommandDispatchLane::OwnerIndependent,
        _ => CdpCommandDispatchLane::MainThread,
    }
}

/// Mirrors content::DevToolsSession::ShouldSendOnIO at Chromium
/// a03603fe9af6230a12f1b2fb2c18a7d003a0d937.
pub(crate) fn chromium_sends_on_io(method: &str) -> bool {
    matches!(
        method,
        "Debugger.getPossibleBreakpoints"
            | "Debugger.getScriptSource"
            | "Debugger.getStackTrace"
            | "Debugger.pause"
            | "Debugger.removeBreakpoint"
            | "Debugger.resume"
            | "Debugger.setBreakpoint"
            | "Debugger.setBreakpointByUrl"
            | "Debugger.setBreakpointsActive"
            | "Emulation.setScriptExecutionDisabled"
            | "Page.crash"
            | "Performance.getMetrics"
            | "Runtime.terminateExecution"
    )
}

/// Union of the generated-handler `async` lists in Chromium's content,
/// Blink, Chrome, headless, UI DevTools, and V8 Inspector protocol configs at
/// a03603fe9af6230a12f1b2fb2c18a7d003a0d937. Commands absent from these lists
/// use the generator's synchronous `protocol::Response` shape.
const CHROMIUM_ASYNC_CALLBACK_METHODS: &[&str] = &[
    "Accessibility.queryAXTree",
    "Autofill.setAddresses",
    "Autofill.trigger",
    "BackgroundService.startObserving",
    "BluetoothEmulation.addCharacteristic",
    "BluetoothEmulation.addDescriptor",
    "BluetoothEmulation.addService",
    "BluetoothEmulation.removeCharacteristic",
    "BluetoothEmulation.removeDescriptor",
    "BluetoothEmulation.removeService",
    "BluetoothEmulation.setSimulatedCentralState",
    "BluetoothEmulation.simulateAdvertisement",
    "BluetoothEmulation.simulateCharacteristicOperationResponse",
    "BluetoothEmulation.simulateDescriptorOperationResponse",
    "BluetoothEmulation.simulateGATTDisconnection",
    "BluetoothEmulation.simulateGATTOperationResponse",
    "BluetoothEmulation.simulatePreconnectedPeripheral",
    "Browser.addPrivacySandboxCoordinatorKeyConfig",
    "Browser.grantPermissions",
    "Browser.resetPermissions",
    "Browser.setPermission",
    "CSS.enable",
    "CSS.takeComputedStyleUpdates",
    "CacheStorage.deleteCache",
    "CacheStorage.deleteEntry",
    "CacheStorage.requestCacheNames",
    "CacheStorage.requestCachedResponse",
    "CacheStorage.requestEntries",
    "Cast.startDesktopMirroring",
    "Cast.startTabMirroring",
    "DeviceOrientation.setDeviceOrientationOverride",
    "Emulation.getOverriddenSensorInformation",
    "Emulation.setPressureDataOverride",
    "Emulation.setPressureStateOverride",
    "Emulation.setSensorOverrideReadings",
    "Extensions.clearStorageItems",
    "Extensions.getStorageItems",
    "Extensions.loadUnpacked",
    "Extensions.removeStorageItems",
    "Extensions.setStorageItems",
    "Extensions.uninstall",
    "Fetch.continueRequest",
    "Fetch.continueResponse",
    "Fetch.continueWithAuth",
    "Fetch.enable",
    "Fetch.failRequest",
    "Fetch.fulfillRequest",
    "Fetch.getResponseBody",
    "Fetch.takeResponseBodyAsStream",
    "FileSystem.getDirectory",
    "HeadlessExperimental.beginFrame",
    "HeapProfiler.collectGarbage",
    "HeapProfiler.takeHeapSnapshot",
    "IO.read",
    "IndexedDB.clearObjectStore",
    "IndexedDB.deleteDatabase",
    "IndexedDB.deleteObjectStoreEntries",
    "IndexedDB.getMetadata",
    "IndexedDB.requestData",
    "IndexedDB.requestDatabase",
    "IndexedDB.requestDatabaseNames",
    "Input.cancelDragging",
    "Input.dispatchDragEvent",
    "Input.dispatchKeyEvent",
    "Input.dispatchMouseEvent",
    "Input.dispatchTouchEvent",
    "Input.imeSetComposition",
    "Input.insertText",
    "Input.synthesizePinchGesture",
    "Input.synthesizeScrollGesture",
    "Input.synthesizeTapGesture",
    "Memory.getDOMCountersForLeakDetection",
    "Memory.prepareForLeakDetection",
    "NativeProfiling.dumpProfilingDataOfAllProcesses",
    "Network.clearBrowserCache",
    "Network.clearBrowserCookies",
    "Network.configureDurableMessages",
    "Network.continueInterceptedRequest",
    "Network.deleteCookies",
    "Network.getAllCookies",
    "Network.getCookies",
    "Network.getRequestPostData",
    "Network.getResponseBody",
    "Network.getResponseBodyForInterception",
    "Network.loadNetworkResource",
    "Network.setCookie",
    "Network.setCookies",
    "Network.takeResponseBodyForInterceptionAsStream",
    "PWA.changeAppUserSettings",
    "PWA.getOsAppState",
    "PWA.install",
    "PWA.launch",
    "PWA.launchFilesInApp",
    "PWA.uninstall",
    "Page.captureScreenshot",
    "Page.captureSnapshot",
    "Page.createIsolatedWorld",
    "Page.getAnnotatedPageContent",
    "Page.getAppId",
    "Page.getAppManifest",
    "Page.getInstallabilityErrors",
    "Page.getManifestIcons",
    "Page.getResourceContent",
    "Page.navigate",
    "Page.printToPDF",
    "Page.reload",
    "Page.searchInResource",
    "Runtime.awaitPromise",
    "Runtime.callFunctionOn",
    "Runtime.evaluate",
    "Runtime.runScript",
    "Runtime.terminateExecution",
    "ServiceWorker.stopAllWorkers",
    "Storage.clearCookies",
    "Storage.clearDataForOrigin",
    "Storage.clearDataForStorageKey",
    "Storage.clearSharedStorageEntries",
    "Storage.clearTrustTokens",
    "Storage.deleteSharedStorageEntry",
    "Storage.getCookies",
    "Storage.getInterestGroupDetails",
    "Storage.getRelatedWebsiteSets",
    "Storage.getSharedStorageEntries",
    "Storage.getSharedStorageMetadata",
    "Storage.getStorageBucketList",
    "Storage.getTrustTokens",
    "Storage.getUsageAndQuota",
    "Storage.overrideQuotaForOrigin",
    "Storage.resetSharedStorageBudget",
    "Storage.runBounceTrackingMitigations",
    "Storage.sendPendingAttributionReports",
    "Storage.setAttributionReportingLocalTestingMode",
    "Storage.setCookies",
    "Storage.setSharedStorageEntry",
    "SystemInfo.getInfo",
    "SystemInfo.getProcessInfo",
    "Target.autoAttachRelated",
    "Target.createBrowserContext",
    "Target.disposeBrowserContext",
    "Target.exposeDevToolsProtocol",
    "Target.setAutoAttach",
    "Tethering.bind",
    "Tethering.unbind",
    "Tracing.getCategories",
    "Tracing.requestMemoryDump",
    "Tracing.start",
    "WebAuthn.addCredential",
    "WebAuthn.getCredential",
    "WebAuthn.getCredentials",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chromium_async_callback_catalog_is_sorted_and_unique() {
        assert!(
            CHROMIUM_ASYNC_CALLBACK_METHODS
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
    }

    #[test]
    fn completion_semantics_match_chromium_generator_contracts() {
        for method in [
            "Page.getFrameTree",
            "Page.setDocumentContent",
            "DOM.getDocument",
            "Emulation.setCPUThrottlingRate",
            "Runtime.getProperties",
        ] {
            assert_eq!(
                completion_semantics_for_method(method),
                CdpCommandCompletionSemantics::SynchronousResponse,
                "{method} must retain Chromium's synchronous handler contract"
            );
        }
        for method in [
            "Page.navigate",
            "Page.createIsolatedWorld",
            "Runtime.evaluate",
            "Input.dispatchMouseEvent",
            "Fetch.continueRequest",
        ] {
            assert_eq!(
                completion_semantics_for_method(method),
                CdpCommandCompletionSemantics::AsyncCallback,
                "{method} must retain Chromium's callback handler contract"
            );
        }
    }

    #[test]
    fn dispatch_lanes_are_independent_from_completion_semantics() {
        for method in [
            "Page.getFrameTree",
            "Page.setDocumentContent",
            "DOM.getDocument",
            "Emulation.setCPUThrottlingRate",
            "Network.enable",
        ] {
            assert_eq!(
                dispatch_lane_for_method(method),
                CdpCommandDispatchLane::MainThread,
                "{method} must enter Chromium's ordered renderer session"
            );
        }
        for method in [
            "Debugger.getScriptSource",
            "Runtime.terminateExecution",
            "Page.crash",
        ] {
            assert_eq!(
                dispatch_lane_for_method(method),
                CdpCommandDispatchLane::Io,
                "{method} must bypass the renderer main session"
            );
        }
        for method in [
            "Page.navigate",
            "Page.captureScreenshot",
            "DOM.disable",
            "Target.attachToTarget",
            "Browser.getVersion",
        ] {
            assert_eq!(
                dispatch_lane_for_method(method),
                CdpCommandDispatchLane::OwnerIndependent,
                "{method} must remain independent from the renderer main session"
            );
        }

        let callback_on_main = CdpCommandSchedulingPolicy::for_method("Page.createIsolatedWorld");
        assert_eq!(
            callback_on_main.completion_semantics(),
            CdpCommandCompletionSemantics::AsyncCallback
        );
        assert_eq!(
            callback_on_main.dispatch_lane(),
            CdpCommandDispatchLane::MainThread
        );

        let synchronous_on_io = CdpCommandSchedulingPolicy::for_method("Debugger.getScriptSource");
        assert_eq!(
            synchronous_on_io.completion_semantics(),
            CdpCommandCompletionSemantics::SynchronousResponse
        );
        assert_eq!(
            synchronous_on_io.dispatch_lane(),
            CdpCommandDispatchLane::Io
        );
    }
}
