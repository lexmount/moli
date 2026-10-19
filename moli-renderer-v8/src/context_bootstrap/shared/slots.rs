pub(crate) const WINDOW_DOCUMENT_SLOT: &str = "__moliWindowAssociatedDocument";
pub(in crate::context_bootstrap) const WINDOW_LOCATION_SLOT: &str = "__moliWindowLocation";
pub(in crate::context_bootstrap) const WINDOW_LOCATION_HREF_SLOT: &str = "__moliWindowLocationHref";
pub(in crate::context_bootstrap) const WINDOW_CONSOLE_SLOT: &str = "__moliWindowConsole";
pub(in crate::context_bootstrap) const WINDOW_ORIGINAL_CONSOLE_SLOT: &str =
    "__moliWindowOriginalConsole";
pub(in crate::context_bootstrap) const WINDOW_ONERROR_SLOT: &str = "__moliWindowOnError";

pub(in crate::context_bootstrap) const WINDOW_ONUNHANDLEDREJECTION_SLOT: &str =
    "__moliWindowOnUnhandledRejection";
pub(in crate::context_bootstrap) const WINDOW_ONREJECTIONHANDLED_SLOT: &str =
    "__moliWindowOnRejectionHandled";
pub(in crate::context_bootstrap) const WINDOW_NAVIGATOR_SLOT: &str = "__moliWindowNavigator";
pub(in crate::context_bootstrap) const WINDOW_EXTERNAL_SLOT: &str = "__moliWindowExternal";
pub(in crate::context_bootstrap) const NAVIGATOR_RUNTIME_DATA_SLOT: &str =
    "__moliNavigatorRuntimeData";
pub(in crate::context_bootstrap) const WINDOW_HISTORY_SLOT: &str = "__moliWindowHistory";
pub(in crate::context_bootstrap) const WINDOW_NAVIGATION_SLOT: &str = "__moliWindowNavigation";
pub(in crate::context_bootstrap) const WINDOW_SCREEN_SLOT: &str = "__moliWindowScreen";
pub(in crate::context_bootstrap) const WINDOW_CRYPTO_SLOT: &str = "__moliWindowCrypto";
pub(in crate::context_bootstrap) const WINDOW_PERFORMANCE_SLOT: &str = "__moliWindowPerformance";
pub(in crate::context_bootstrap) const WINDOW_VISUAL_VIEWPORT_SLOT: &str =
    "__moliWindowVisualViewport";
pub(in crate::context_bootstrap) const WINDOW_SPEECH_SYNTHESIS_SLOT: &str =
    "__moliWindowSpeechSynthesis";
pub(in crate::context_bootstrap) const WINDOW_SCROLL_X_SLOT: &str = "__moliWindowScrollX";
pub(in crate::context_bootstrap) const WINDOW_SCROLL_Y_SLOT: &str = "__moliWindowScrollY";
pub(in crate::context_bootstrap) const WINDOW_SELECTION_SLOT: &str = "__moliWindowSelection";
pub(in crate::context_bootstrap) const WINDOW_LOCAL_STORAGE_SLOT: &str = "__moliWindowLocalStorage";
pub(in crate::context_bootstrap) const WINDOW_SESSION_STORAGE_SLOT: &str =
    "__moliWindowSessionStorage";
pub(crate) const WINDOW_CUSTOM_ELEMENTS_SLOT: &str = "__moliWindowCustomElements";
pub(crate) const WINDOW_NAME_SLOT: &str = "__moliWindowName";
pub(crate) const CHILD_BROWSING_CONTEXT_HANDLE_SLOT: &str = "__moliChildBrowsingContextHandle";
pub(in crate::context_bootstrap) const WINDOW_SELF_SLOT: &str = "__moliWindowSelf";
pub(in crate::context_bootstrap) const WINDOW_PARENT_SLOT: &str = "__moliWindowParent";
pub(in crate::context_bootstrap) const WINDOW_TOP_SLOT: &str = "__moliWindowTop";
pub(in crate::context_bootstrap) const WINDOW_FRAMES_SLOT: &str = "__moliWindowFrames";
// Each SimpleEventTarget stores its own listener registry directly under this key.
pub(crate) use moli_webapi::event_target::SIMPLE_EVENT_TARGET_LISTENERS_SLOT;
pub(in crate::context_bootstrap) const DOM_IMPLEMENTATION_SINGLETON_SLOT: &str =
    "__moliDOMImplementationSingleton";
pub(in crate::context_bootstrap) const SELECTION_RANGE_SLOT: &str = "__moliSelectionRange";

pub(in crate::context_bootstrap) const MESSAGE_PORT_PEER_SLOT: &str = "__moliMessagePortPeer";
pub(in crate::context_bootstrap) const MESSAGE_PORT_ONMESSAGE_HANDLER_SLOT: &str =
    "__lmMessagePortOnmessageHandler";
pub(in crate::context_bootstrap) const MESSAGE_PORT_ONMESSAGEERROR_HANDLER_SLOT: &str =
    "__lmMessagePortOnmessageerrorHandler";
pub(in crate::context_bootstrap) const MESSAGE_PORT_ONCLOSE_HANDLER_SLOT: &str =
    "__lmMessagePortOncloseHandler";
pub(in crate::context_bootstrap) const MESSAGE_PORT_STARTED_SLOT: &str = "__moliMessagePortStarted";
pub(in crate::context_bootstrap) const MESSAGE_PORT_CLOSED_SLOT: &str = "__moliMessagePortClosed";
pub(in crate::context_bootstrap) const FILE_READER_PENDING_RESULT_SLOT: &str =
    "__moliFileReaderPendingResult";
pub(in crate::context_bootstrap) const FILE_READER_PENDING_TOTAL_SLOT: &str =
    "__moliFileReaderPendingTotal";
pub(in crate::context_bootstrap) const FILE_READER_READ_ID_SLOT: &str = "__moliFileReaderReadId";
pub(in crate::context_bootstrap) const RESIZE_OBSERVER_CALLBACK_ID_SLOT: &str =
    "__moliResizeObserverCallbackId";
pub(in crate::context_bootstrap) const RESIZE_OBSERVER_CALLBACK_VALUE_SLOT: &str =
    "__moliResizeObserverCallbackValue";
pub(in crate::context_bootstrap) const RESIZE_OBSERVER_CALLBACK_RELEVANT_GLOBAL_SLOT: &str =
    "__moliResizeObserverCallbackRelevantGlobal";
pub(in crate::context_bootstrap) const RESIZE_OBSERVER_CALLBACK_INCUMBENT_GLOBAL_SLOT: &str =
    "__moliResizeObserverCallbackIncumbentGlobal";
pub(in crate::context_bootstrap) const RESIZE_OBSERVER_TARGETS_SLOT: &str =
    "__moliResizeObserverTargets";
pub(in crate::context_bootstrap) const RESIZE_OBSERVER_PENDING_TARGETS_SLOT: &str =
    "__moliResizeObserverPendingTargets";
pub(in crate::context_bootstrap) const RESIZE_OBSERVER_DELIVERY_ACTIVE_SLOT: &str =
    "__moliResizeObserverDeliveryActive";
pub(in crate::context_bootstrap) const PERFORMANCE_OBSERVER_CALLBACK_ID_SLOT: &str =
    "__moliPerformanceObserverCallbackId";
pub(in crate::context_bootstrap) const PERFORMANCE_OBSERVER_CALLBACK_VALUE_SLOT: &str =
    "__moliPerformanceObserverCallbackValue";
pub(in crate::context_bootstrap) const PERFORMANCE_OBSERVER_CALLBACK_RELEVANT_GLOBAL_SLOT: &str =
    "__moliPerformanceObserverCallbackRelevantGlobal";
pub(in crate::context_bootstrap) const PERFORMANCE_OBSERVER_CALLBACK_INCUMBENT_GLOBAL_SLOT: &str =
    "__moliPerformanceObserverCallbackIncumbentGlobal";
pub(in crate::context_bootstrap) const PERFORMANCE_OBSERVER_PENDING_SLOT: &str =
    "__moliPerformanceObserverPending";
pub(in crate::context_bootstrap) const PERFORMANCE_OBSERVER_TYPE_SLOT: &str =
    "__moliPerformanceObserverType";
pub(in crate::context_bootstrap) const PERFORMANCE_OBSERVER_MODE_SLOT: &str =
    "__moliPerformanceObserverMode";
pub(in crate::context_bootstrap) const PERFORMANCE_OBSERVER_ENTRY_TYPES_SLOT: &str =
    "__moliPerformanceObserverEntryTypes";
pub(in crate::context_bootstrap) const PERFORMANCE_OBSERVER_ACTIVE_SLOT: &str =
    "__moliPerformanceObserverActive";
pub(in crate::context_bootstrap) const PERFORMANCE_OBSERVER_SCHEDULED_SLOT: &str =
    "__moliPerformanceObserverScheduled";
pub(in crate::context_bootstrap) const PERFORMANCE_OBSERVER_REQUIRES_DROPPED_ENTRIES_SLOT: &str =
    "__moliPerformanceObserverRequiresDroppedEntries";
pub(in crate::context_bootstrap) const PERFORMANCE_OBSERVER_QUEUE_SLOT: &str =
    "__moliPerformanceObserverQueue";
pub(in crate::context_bootstrap) const PERFORMANCE_OBSERVER_SUPPORTED_ENTRY_TYPES_SLOT: &str =
    "__moliPerformanceObserverSupportedEntryTypes";
pub(in crate::context_bootstrap) const PERFORMANCE_ENTRIES_SLOT: &str = "__moliPerformanceEntries";
pub(in crate::context_bootstrap) use moli_webapi::media_query_list::{
    MEDIA_QUERY_LIST_MATCHES_SLOT, MEDIA_QUERY_LIST_MEDIA_SLOT, MEDIA_QUERY_LIST_ONCHANGE_SLOT,
};
pub(in crate::context_bootstrap) const MEDIA_QUERY_LIST_REGISTRY_SLOT: &str =
    "__moliMediaQueryLists";
pub(in crate::context_bootstrap) use moli_webapi::streams::slots::{
    READABLE_STREAM_CONTROLLER_SLOT, READABLE_STREAM_HWM_SLOT, READABLE_STREAM_PENDING_READS_SLOT,
    STREAM_CONTROLLER_ALGORITHMS_SLOT, TRANSFORM_STREAM_READABLE_SLOT,
    TRANSFORM_STREAM_WRITABLE_SLOT, WRITABLE_STREAM_SINK_SLOT,
};
