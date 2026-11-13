//! Event names generated locally by Moli. V8 notifications are selected from its pinned schema.

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::VariantNames, strum::IntoStaticStr)]
pub enum LocalCdpEvent {
    #[strum(serialize = "Audits.issueAdded")]
    AuditsIssueAdded,
    #[strum(serialize = "Browser.downloadProgress")]
    BrowserDownloadProgress,
    #[strum(serialize = "Browser.downloadWillBegin")]
    BrowserDownloadWillBegin,
    #[strum(serialize = "CSS.styleSheetAdded")]
    CssStyleSheetAdded,
    #[strum(serialize = "CSS.styleSheetRemoved")]
    CssStyleSheetRemoved,
    #[strum(serialize = "Console.messageAdded")]
    ConsoleMessageAdded,
    #[strum(serialize = "DOM.attributeModified")]
    DomAttributeModified,
    #[strum(serialize = "DOM.attributeRemoved")]
    DomAttributeRemoved,
    #[strum(serialize = "DOM.characterDataModified")]
    DomCharacterDataModified,
    #[strum(serialize = "DOM.childNodeCountUpdated")]
    DomChildNodeCountUpdated,
    #[strum(serialize = "DOM.childNodeInserted")]
    DomChildNodeInserted,
    #[strum(serialize = "DOM.childNodeRemoved")]
    DomChildNodeRemoved,
    #[strum(serialize = "DOM.documentUpdated")]
    DomDocumentUpdated,
    #[strum(serialize = "DOM.setChildNodes")]
    DomSetChildNodes,
    #[strum(serialize = "DOMStorage.domStorageItemAdded")]
    DomStorageItemAdded,
    #[strum(serialize = "DOMStorage.domStorageItemRemoved")]
    DomStorageItemRemoved,
    #[strum(serialize = "DOMStorage.domStorageItemUpdated")]
    DomStorageItemUpdated,
    #[strum(serialize = "DOMStorage.domStorageItemsCleared")]
    DomStorageItemsCleared,
    #[strum(serialize = "Fetch.authRequired")]
    FetchAuthRequired,
    #[strum(serialize = "Fetch.requestPaused")]
    FetchRequestPaused,
    #[strum(serialize = "Input.dragIntercepted")]
    InputDragIntercepted,
    #[strum(serialize = "Inspector.detached")]
    InspectorDetached,
    #[strum(serialize = "Inspector.targetCrashed")]
    InspectorTargetCrashed,
    #[strum(serialize = "Inspector.targetReloadedAfterCrash")]
    InspectorTargetReloadedAfterCrash,
    #[strum(serialize = "Inspector.workerScriptLoaded")]
    InspectorWorkerScriptLoaded,
    #[strum(serialize = "Log.entryAdded")]
    LogEntryAdded,
    #[strum(serialize = "Network.dataReceived")]
    NetworkDataReceived,
    #[strum(serialize = "Network.eventSourceMessageReceived")]
    NetworkEventSourceMessageReceived,
    #[strum(serialize = "Network.loadingFailed")]
    NetworkLoadingFailed,
    #[strum(serialize = "Network.loadingFinished")]
    NetworkLoadingFinished,
    #[strum(serialize = "Network.requestServedFromCache")]
    NetworkRequestServedFromCache,
    #[strum(serialize = "Network.requestWillBeSent")]
    NetworkRequestWillBeSent,
    #[strum(serialize = "Network.requestWillBeSentExtraInfo")]
    NetworkRequestWillBeSentExtraInfo,
    #[strum(serialize = "Network.responseReceived")]
    NetworkResponseReceived,
    #[strum(serialize = "Network.responseReceivedExtraInfo")]
    NetworkResponseReceivedExtraInfo,
    #[strum(serialize = "Network.webSocketClosed")]
    NetworkWebSocketClosed,
    #[strum(serialize = "Network.webSocketCreated")]
    NetworkWebSocketCreated,
    #[strum(serialize = "Network.webSocketFrameError")]
    NetworkWebSocketFrameError,
    #[strum(serialize = "Network.webSocketFrameReceived")]
    NetworkWebSocketFrameReceived,
    #[strum(serialize = "Network.webSocketFrameSent")]
    NetworkWebSocketFrameSent,
    #[strum(serialize = "Network.webSocketHandshakeResponseReceived")]
    NetworkWebSocketHandshakeResponseReceived,
    #[strum(serialize = "Network.webSocketWillSendHandshakeRequest")]
    NetworkWebSocketWillSendHandshakeRequest,
    #[strum(serialize = "Page.documentOpened")]
    PageDocumentOpened,
    #[strum(serialize = "Page.domContentEventFired")]
    PageDomContentEventFired,
    #[strum(serialize = "Page.downloadProgress")]
    PageDownloadProgress,
    #[strum(serialize = "Page.downloadWillBegin")]
    PageDownloadWillBegin,
    #[strum(serialize = "Page.fileChooserOpened")]
    PageFileChooserOpened,
    #[strum(serialize = "Page.frameAttached")]
    PageFrameAttached,
    #[strum(serialize = "Page.frameClearedScheduledNavigation")]
    PageFrameClearedScheduledNavigation,
    #[strum(serialize = "Page.frameDetached")]
    PageFrameDetached,
    #[strum(serialize = "Page.frameNavigated")]
    PageFrameNavigated,
    #[strum(serialize = "Page.frameRequestedNavigation")]
    PageFrameRequestedNavigation,
    #[strum(serialize = "Page.frameScheduledNavigation")]
    PageFrameScheduledNavigation,
    #[strum(serialize = "Page.frameStartedLoading")]
    PageFrameStartedLoading,
    #[strum(serialize = "Page.frameStartedNavigating")]
    PageFrameStartedNavigating,
    #[strum(serialize = "Page.frameStoppedLoading")]
    PageFrameStoppedLoading,
    #[strum(serialize = "Page.javascriptDialogClosed")]
    PageJavascriptDialogClosed,
    #[strum(serialize = "Page.javascriptDialogOpening")]
    PageJavascriptDialogOpening,
    #[strum(serialize = "Page.lifecycleEvent")]
    PageLifecycleEvent,
    #[strum(serialize = "Page.loadEventFired")]
    PageLoadEventFired,
    #[strum(serialize = "Page.navigatedWithinDocument")]
    PageNavigatedWithinDocument,
    #[strum(serialize = "Page.screencastFrame")]
    PageScreencastFrame,
    #[strum(serialize = "Page.screencastVisibilityChanged")]
    PageScreencastVisibilityChanged,
    #[strum(serialize = "Page.windowOpen")]
    PageWindowOpen,
    #[strum(serialize = "Runtime.bindingCalled")]
    RuntimeBindingCalled,
    #[strum(serialize = "Runtime.consoleAPICalled")]
    RuntimeConsoleApiCalled,
    #[strum(serialize = "Runtime.exceptionThrown")]
    RuntimeExceptionThrown,
    #[strum(serialize = "Runtime.executionContextCreated")]
    RuntimeExecutionContextCreated,
    #[strum(serialize = "Runtime.executionContextDestroyed")]
    RuntimeExecutionContextDestroyed,
    #[strum(serialize = "Runtime.executionContextsCleared")]
    RuntimeExecutionContextsCleared,
    #[strum(serialize = "ServiceWorker.workerErrorReported")]
    ServiceWorkerErrorReported,
    #[strum(serialize = "ServiceWorker.workerRegistrationUpdated")]
    ServiceWorkerRegistrationUpdated,
    #[strum(serialize = "ServiceWorker.workerVersionUpdated")]
    ServiceWorkerVersionUpdated,
    #[strum(serialize = "Target.attachedToTarget")]
    TargetAttachedToTarget,
    #[strum(serialize = "Target.detachedFromTarget")]
    TargetDetachedFromTarget,
    #[strum(serialize = "Target.receivedMessageFromTarget")]
    TargetReceivedMessageFromTarget,
    #[strum(serialize = "Target.targetCrashed")]
    TargetCrashed,
    #[strum(serialize = "Target.targetCreated")]
    TargetCreated,
    #[strum(serialize = "Target.targetDestroyed")]
    TargetDestroyed,
    #[strum(serialize = "Target.targetInfoChanged")]
    TargetInfoChanged,
    #[strum(serialize = "Tracing.dataCollected")]
    TracingDataCollected,
    #[strum(serialize = "Tracing.tracingComplete")]
    TracingComplete,
    #[strum(serialize = "WebMCP.toolInvoked")]
    WebMcpToolInvoked,
    #[strum(serialize = "WebMCP.toolResponded")]
    WebMcpToolResponded,
    #[strum(serialize = "WebMCP.toolsAdded")]
    WebMcpToolsAdded,
    #[strum(serialize = "WebMCP.toolsRemoved")]
    WebMcpToolsRemoved,
}

impl LocalCdpEvent {
    pub const METHODS: &'static [&'static str] = <Self as strum::VariantNames>::VARIANTS;

    pub fn method(self) -> &'static str {
        self.into()
    }
}

#[cfg(test)]
mod tests {
    use super::LocalCdpEvent;
    use std::collections::BTreeSet;

    #[test]
    fn local_events_have_unique_qualified_wire_names() {
        let names: BTreeSet<_> = LocalCdpEvent::METHODS.iter().copied().collect();
        assert_eq!(names.len(), LocalCdpEvent::METHODS.len());
        assert!(names.iter().all(|name| name.split_once('.').is_some()));
        assert_eq!(
            LocalCdpEvent::RuntimeConsoleApiCalled.method(),
            "Runtime.consoleAPICalled"
        );
        assert_eq!(
            LocalCdpEvent::NetworkWebSocketFrameSent.method(),
            "Network.webSocketFrameSent"
        );
        assert_eq!(
            LocalCdpEvent::TracingComplete.method(),
            "Tracing.tracingComplete"
        );
    }
}
