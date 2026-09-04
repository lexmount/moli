use super::{ConstructorKind, ConstructorSpec};
use crate::context_bootstrap::bridge_descriptor::node_bridge_descriptors;
use std::collections::HashSet;

const CONSTRUCTOR_SPECS_BEFORE_STREAMS: &[ConstructorSpec] = &[
    ConstructorSpec {
        name: "NodeList",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "HTMLCollection",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "HTMLFormControlsCollection",
        parent: Some("HTMLCollection"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "HTMLOptionsCollection",
        parent: Some("HTMLCollection"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "RadioNodeList",
        parent: Some("NodeList"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "ValidityState",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "DOMTokenList",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "DOMStringMap",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "DOMStringList",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "PluginArray",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "MimeTypeArray",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "Plugin",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "MimeType",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "CustomElementRegistry",
        parent: None,
        kind: ConstructorKind::CustomElementRegistry,
    },
    ConstructorSpec {
        name: "ElementInternals",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "CustomStateSet",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "Attr",
        parent: Some("Node"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "Option",
        parent: None,
        kind: ConstructorKind::Option,
    },
    ConstructorSpec {
        name: "NamedNodeMap",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "SVGLength",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "SVGNumber",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "SVGAnimatedLength",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "SVGLengthList",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "SVGAnimatedLengthList",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "SVGAnimatedNumber",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "SVGNumberList",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "SVGAnimatedNumberList",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "SVGAnimatedEnumeration",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "SVGAnimatedTransformList",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "SVGTransformList",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "SVGTransform",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "SVGMatrix",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "CSSStyleDeclaration",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "StylePropertyMapReadOnly",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "CSSStyleValue",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "CSSKeywordValue",
        parent: Some("CSSStyleValue"),
        kind: ConstructorKind::CssKeywordValue,
    },
    ConstructorSpec {
        name: "CSSNumericValue",
        parent: Some("CSSStyleValue"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "CSSUnitValue",
        parent: Some("CSSNumericValue"),
        kind: ConstructorKind::CssUnitValue,
    },
    ConstructorSpec {
        name: "CSSStyleProperties",
        parent: Some("CSSStyleDeclaration"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "CSSFontFaceDescriptors",
        parent: Some("CSSStyleDeclaration"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "CSSPageDescriptors",
        parent: Some("CSSStyleDeclaration"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "CSSFontFeatureValuesMap",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "HTMLAllCollection",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "Event",
        parent: None,
        kind: ConstructorKind::Event,
    },
    ConstructorSpec {
        name: "UIEvent",
        parent: Some("Event"),
        kind: ConstructorKind::UiEvent,
    },
    ConstructorSpec {
        name: "FocusEvent",
        parent: Some("UIEvent"),
        kind: ConstructorKind::FocusEvent,
    },
    ConstructorSpec {
        name: "TextEvent",
        parent: Some("UIEvent"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "CompositionEvent",
        parent: Some("UIEvent"),
        kind: ConstructorKind::CompositionEvent,
    },
    ConstructorSpec {
        name: "CustomEvent",
        parent: Some("Event"),
        kind: ConstructorKind::CustomEvent,
    },
    ConstructorSpec {
        name: "MouseEvent",
        parent: Some("UIEvent"),
        kind: ConstructorKind::MouseEvent,
    },
    ConstructorSpec {
        name: "CapturedMouseEvent",
        parent: Some("Event"),
        kind: ConstructorKind::CapturedMouseEvent,
    },
    ConstructorSpec {
        name: "DragEvent",
        parent: Some("MouseEvent"),
        kind: ConstructorKind::DragEvent,
    },
    ConstructorSpec {
        name: "ClipboardEvent",
        parent: Some("Event"),
        kind: ConstructorKind::ClipboardEvent,
    },
    ConstructorSpec {
        name: "KeyboardEvent",
        parent: Some("UIEvent"),
        kind: ConstructorKind::KeyboardEvent,
    },
    ConstructorSpec {
        name: "InputEvent",
        parent: Some("UIEvent"),
        kind: ConstructorKind::InputEvent,
    },
    ConstructorSpec {
        name: "WheelEvent",
        parent: Some("MouseEvent"),
        kind: ConstructorKind::WheelEvent,
    },
    ConstructorSpec {
        name: "PointerEvent",
        parent: Some("MouseEvent"),
        kind: ConstructorKind::PointerEvent,
    },
    ConstructorSpec {
        name: "TouchEvent",
        parent: Some("UIEvent"),
        kind: ConstructorKind::TouchEvent,
    },
    ConstructorSpec {
        name: "MessageEvent",
        parent: Some("Event"),
        kind: ConstructorKind::MessageEvent,
    },
    ConstructorSpec {
        name: "StorageEvent",
        parent: Some("Event"),
        kind: ConstructorKind::StorageEvent,
    },
    ConstructorSpec {
        name: "ErrorEvent",
        parent: Some("Event"),
        kind: ConstructorKind::ErrorEvent,
    },
    ConstructorSpec {
        name: "PromiseRejectionEvent",
        parent: Some("Event"),
        kind: ConstructorKind::PromiseRejectionEvent,
    },
    ConstructorSpec {
        name: "NavigationCurrentEntryChangeEvent",
        parent: Some("Event"),
        kind: ConstructorKind::NavigationCurrentEntryChangeEvent,
    },
    ConstructorSpec {
        name: "NavigateEvent",
        parent: Some("Event"),
        kind: ConstructorKind::NavigateEvent,
    },
    ConstructorSpec {
        name: "CloseEvent",
        parent: Some("Event"),
        kind: ConstructorKind::CloseEvent,
    },
    ConstructorSpec {
        name: "SubmitEvent",
        parent: Some("Event"),
        kind: ConstructorKind::SubmitEvent,
    },
    ConstructorSpec {
        name: "FormDataEvent",
        parent: Some("Event"),
        kind: ConstructorKind::FormDataEvent,
    },
    ConstructorSpec {
        name: "PopStateEvent",
        parent: Some("Event"),
        kind: ConstructorKind::PopStateEvent,
    },
    ConstructorSpec {
        name: "PageTransitionEvent",
        parent: Some("Event"),
        kind: ConstructorKind::PageTransitionEvent,
    },
    ConstructorSpec {
        name: "BeforeUnloadEvent",
        parent: Some("Event"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "HashChangeEvent",
        parent: Some("Event"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "MediaQueryListEvent",
        parent: Some("Event"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "SecurityPolicyViolationEvent",
        parent: Some("Event"),
        kind: ConstructorKind::SecurityPolicyViolationEvent,
    },
    ConstructorSpec {
        name: "ToggleEvent",
        parent: Some("Event"),
        kind: ConstructorKind::ToggleEvent,
    },
    ConstructorSpec {
        name: "CommandEvent",
        parent: Some("Event"),
        kind: ConstructorKind::CommandEvent,
    },
    ConstructorSpec {
        name: "InterestEvent",
        parent: Some("Event"),
        kind: ConstructorKind::InterestEvent,
    },
    ConstructorSpec {
        name: "ContentVisibilityAutoStateChangeEvent",
        parent: Some("Event"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "DOMException",
        parent: None,
        kind: ConstructorKind::DomException,
    },
    ConstructorSpec {
        name: "DOMError",
        parent: None,
        kind: ConstructorKind::DomError,
    },
    ConstructorSpec {
        name: "QuotaExceededError",
        parent: Some("DOMException"),
        kind: ConstructorKind::QuotaExceededError,
    },
    ConstructorSpec {
        name: "AbortSignal",
        parent: Some("EventTarget"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "AbortController",
        parent: None,
        kind: ConstructorKind::AbortController,
    },
    ConstructorSpec {
        name: "BroadcastChannel",
        parent: Some("EventTarget"),
        kind: ConstructorKind::BroadcastChannel,
    },
    ConstructorSpec {
        name: "EventSource",
        parent: Some("EventTarget"),
        kind: ConstructorKind::EventSource,
    },
    ConstructorSpec {
        name: "IdleDetector",
        parent: Some("EventTarget"),
        kind: ConstructorKind::IdleDetector,
    },
    ConstructorSpec {
        name: "Notification",
        parent: Some("EventTarget"),
        kind: ConstructorKind::Notification,
    },
    ConstructorSpec {
        name: "MessageChannel",
        parent: None,
        kind: ConstructorKind::MessageChannel,
    },
    ConstructorSpec {
        name: "MessagePort",
        parent: Some("EventTarget"),
        kind: ConstructorKind::MessagePort,
    },
    ConstructorSpec {
        name: "Worker",
        parent: Some("EventTarget"),
        kind: ConstructorKind::Worker,
    },
    ConstructorSpec {
        name: "SharedWorker",
        parent: Some("EventTarget"),
        kind: ConstructorKind::SharedWorker,
    },
    ConstructorSpec {
        name: "WorkerNavigator",
        parent: None,
        kind: ConstructorKind::WorkerNavigator,
    },
    ConstructorSpec {
        name: "WorkerLocation",
        parent: None,
        kind: ConstructorKind::WorkerLocation,
    },
    ConstructorSpec {
        name: "WebSocket",
        parent: Some("EventTarget"),
        kind: ConstructorKind::WebSocket,
    },
    ConstructorSpec {
        name: "WebSocketError",
        parent: Some("DOMException"),
        kind: ConstructorKind::WebSocketError,
    },
    ConstructorSpec {
        name: "WebSocketStream",
        parent: None,
        kind: ConstructorKind::WebSocketStream,
    },
    ConstructorSpec {
        name: "Performance",
        parent: Some("EventTarget"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "PerformanceTiming",
        parent: None,
        kind: ConstructorKind::PerformanceTiming,
    },
    ConstructorSpec {
        name: "PerformanceNavigation",
        parent: None,
        kind: ConstructorKind::PerformanceNavigation,
    },
    ConstructorSpec {
        name: "Animation",
        parent: None,
        kind: ConstructorKind::Animation,
    },
    ConstructorSpec {
        name: "AnimationEffect",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "KeyframeEffect",
        parent: Some("AnimationEffect"),
        kind: ConstructorKind::KeyframeEffect,
    },
    ConstructorSpec {
        name: "AnimationTimeline",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "DocumentTimeline",
        parent: Some("AnimationTimeline"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "AnimationPlaybackEvent",
        parent: Some("Event"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "ViewTransition",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "ViewTransitionTypeSet",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "Crypto",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "SubtleCrypto",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "CryptoKey",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "VisualViewport",
        parent: Some("EventTarget"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "Navigator",
        parent: None,
        kind: ConstructorKind::Navigator,
    },
    ConstructorSpec {
        name: "Permissions",
        parent: None,
        kind: ConstructorKind::Permissions,
    },
    ConstructorSpec {
        name: "PermissionStatus",
        parent: None,
        kind: ConstructorKind::PermissionStatus,
    },
    ConstructorSpec {
        name: "NavigatorUAData",
        parent: None,
        kind: ConstructorKind::NavigatorUAData,
    },
    ConstructorSpec {
        name: "StorageManager",
        parent: None,
        kind: ConstructorKind::StorageManager,
    },
    ConstructorSpec {
        name: "StorageEstimate",
        parent: None,
        kind: ConstructorKind::StorageEstimate,
    },
    ConstructorSpec {
        name: "StorageAccessHandle",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "StorageBucketManager",
        parent: None,
        kind: ConstructorKind::StorageBucketManager,
    },
    ConstructorSpec {
        name: "StorageBucket",
        parent: None,
        kind: ConstructorKind::StorageBucket,
    },
    ConstructorSpec {
        name: "FileSystemHandle",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "FileSystemFileHandle",
        parent: Some("FileSystemHandle"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "FileSystemDirectoryHandle",
        parent: Some("FileSystemHandle"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "FileSystemWritableFileStream",
        parent: Some("WritableStream"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "FileSystemSyncAccessHandle",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "MediaDevices",
        parent: Some("EventTarget"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "MediaCapabilities",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "Touch",
        parent: None,
        kind: ConstructorKind::Touch,
    },
    ConstructorSpec {
        name: "TouchList",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "Screen",
        parent: None,
        kind: ConstructorKind::Screen,
    },
    ConstructorSpec {
        name: "ScreenOrientation",
        parent: Some("EventTarget"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "SpeechSynthesis",
        parent: Some("EventTarget"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "SpeechSynthesisUtterance",
        parent: Some("EventTarget"),
        kind: ConstructorKind::SpeechSynthesisUtterance,
    },
    ConstructorSpec {
        name: "SpeechSynthesisVoice",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "Selection",
        parent: None,
        kind: ConstructorKind::Selection,
    },
    ConstructorSpec {
        name: "History",
        parent: None,
        kind: ConstructorKind::History,
    },
    ConstructorSpec {
        name: "Location",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "Navigation",
        parent: Some("EventTarget"),
        kind: ConstructorKind::Navigation,
    },
    ConstructorSpec {
        name: "NavigationHistoryEntry",
        parent: Some("EventTarget"),
        kind: ConstructorKind::NavigationHistoryEntry,
    },
    ConstructorSpec {
        name: "NavigationActivation",
        parent: None,
        kind: ConstructorKind::NavigationActivation,
    },
    ConstructorSpec {
        name: "NavigationTransition",
        parent: None,
        kind: ConstructorKind::NavigationTransition,
    },
    ConstructorSpec {
        name: "MutationObserver",
        parent: None,
        kind: ConstructorKind::MutationObserver,
    },
    ConstructorSpec {
        name: "MutationRecord",
        parent: None,
        kind: ConstructorKind::MutationRecord,
    },
    ConstructorSpec {
        name: "IntersectionObserver",
        parent: None,
        kind: ConstructorKind::IntersectionObserver,
    },
    ConstructorSpec {
        name: "IntersectionObserverEntry",
        parent: None,
        kind: ConstructorKind::IntersectionObserverEntry,
    },
    ConstructorSpec {
        name: "EventTarget",
        parent: None,
        kind: ConstructorKind::EventTarget,
    },
    ConstructorSpec {
        name: "IdleDeadline",
        parent: None,
        kind: ConstructorKind::IdleDeadline,
    },
    ConstructorSpec {
        name: "Window",
        parent: Some("EventTarget"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "CharacterData",
        parent: Some("Node"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "DOMImplementation",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "NodeIterator",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "TreeWalker",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "XPathEvaluator",
        parent: None,
        kind: ConstructorKind::XPathEvaluator,
    },
    ConstructorSpec {
        name: "XPathResult",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "StyleSheet",
        parent: None,
        kind: ConstructorKind::StyleSheet,
    },
    ConstructorSpec {
        name: "StyleSheetList",
        parent: None,
        kind: ConstructorKind::StyleSheetList,
    },
    ConstructorSpec {
        name: "MediaList",
        parent: None,
        kind: ConstructorKind::MediaList,
    },
    ConstructorSpec {
        name: "CSSRuleList",
        parent: None,
        kind: ConstructorKind::CssRuleList,
    },
    ConstructorSpec {
        name: "CSSRule",
        parent: None,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        name: "CSSGroupingRule",
        parent: Some("CSSRule"),
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        name: "CSSConditionRule",
        parent: Some("CSSGroupingRule"),
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        name: "CSSMediaRule",
        parent: Some("CSSConditionRule"),
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        name: "CSSSupportsRule",
        parent: Some("CSSConditionRule"),
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        name: "CSSContainerRule",
        parent: Some("CSSConditionRule"),
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        name: "CSSLayerBlockRule",
        parent: Some("CSSGroupingRule"),
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        name: "CSSLayerStatementRule",
        parent: Some("CSSRule"),
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        name: "CSSScopeRule",
        parent: Some("CSSGroupingRule"),
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        name: "CSSImportRule",
        parent: Some("CSSRule"),
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        name: "CSSFontFaceRule",
        parent: Some("CSSRule"),
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        name: "CSSFontFeatureValuesRule",
        parent: Some("CSSRule"),
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        name: "CSSPropertyRule",
        parent: Some("CSSRule"),
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        name: "CSSKeyframesRule",
        parent: Some("CSSRule"),
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        name: "CSSKeyframeRule",
        parent: Some("CSSRule"),
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        name: "CSSPageRule",
        parent: Some("CSSGroupingRule"),
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        name: "CSSMarginRule",
        parent: Some("CSSRule"),
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        name: "CSSNamespaceRule",
        parent: Some("CSSRule"),
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        name: "CSSCounterStyleRule",
        parent: Some("CSSRule"),
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        name: "CSSStyleRule",
        parent: Some("CSSGroupingRule"),
        kind: ConstructorKind::CssStyleRule,
    },
    ConstructorSpec {
        name: "CSSNestedDeclarations",
        parent: Some("CSSRule"),
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        name: "CSSStyleSheet",
        parent: Some("StyleSheet"),
        kind: ConstructorKind::CssStyleSheet,
    },
    ConstructorSpec {
        name: "FontFace",
        parent: None,
        kind: ConstructorKind::FontFace,
    },
    ConstructorSpec {
        name: "FontFaceSet",
        parent: None,
        kind: ConstructorKind::FontFaceSet,
    },
    ConstructorSpec {
        name: "FontFaceSetLoadEvent",
        parent: Some("Event"),
        kind: ConstructorKind::FontFaceSetLoadEvent,
    },
    ConstructorSpec {
        name: "Audio",
        parent: None,
        kind: ConstructorKind::Audio,
    },
    ConstructorSpec {
        name: "Image",
        parent: None,
        kind: ConstructorKind::Image,
    },
    ConstructorSpec {
        name: "XMLHttpRequestEventTarget",
        parent: Some("EventTarget"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "XMLHttpRequestUpload",
        parent: Some("XMLHttpRequestEventTarget"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "XMLHttpRequest",
        parent: Some("XMLHttpRequestEventTarget"),
        kind: ConstructorKind::XmlHttpRequest,
    },
    ConstructorSpec {
        name: "Headers",
        parent: None,
        kind: ConstructorKind::Headers,
    },
    ConstructorSpec {
        name: "Request",
        parent: None,
        kind: ConstructorKind::Request,
    },
    ConstructorSpec {
        name: "Response",
        parent: None,
        kind: ConstructorKind::Response,
    },
    ConstructorSpec {
        name: "ProgressEvent",
        parent: Some("Event"),
        kind: ConstructorKind::ProgressEvent,
    },
    ConstructorSpec {
        name: "DOMParser",
        parent: None,
        kind: ConstructorKind::DomParser,
    },
    ConstructorSpec {
        name: "TextEncoder",
        parent: None,
        kind: ConstructorKind::TextEncoder,
    },
    ConstructorSpec {
        name: "TextDecoder",
        parent: None,
        kind: ConstructorKind::TextDecoder,
    },
];

const CONSTRUCTOR_SPECS_AFTER_STREAMS: &[ConstructorSpec] = &[
    ConstructorSpec {
        name: "Geolocation",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "GeolocationPosition",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "GeolocationCoordinates",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "GeolocationPositionError",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "Blob",
        parent: None,
        kind: ConstructorKind::Blob,
    },
    ConstructorSpec {
        name: "ImageData",
        parent: None,
        kind: ConstructorKind::ImageData,
    },
    ConstructorSpec {
        name: "ImageBitmap",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "RTCPeerConnection",
        parent: Some("EventTarget"),
        kind: ConstructorKind::RtcPeerConnection,
    },
    ConstructorSpec {
        name: "RTCRtpReceiver",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "RTCDataChannel",
        parent: Some("EventTarget"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "CanvasGradient",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "CanvasPattern",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "TextMetrics",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "Path2D",
        parent: None,
        kind: ConstructorKind::Unsupported,
    },
    ConstructorSpec {
        name: "OffscreenCanvas",
        parent: None,
        kind: ConstructorKind::OffscreenCanvas,
    },
    ConstructorSpec {
        name: "CanvasRenderingContext2D",
        parent: None,
        kind: ConstructorKind::CanvasRenderingContext2D,
    },
    ConstructorSpec {
        name: "OffscreenCanvasRenderingContext2D",
        parent: None,
        kind: ConstructorKind::OffscreenCanvasRenderingContext2D,
    },
    ConstructorSpec {
        name: "WebGLRenderingContext",
        parent: None,
        kind: ConstructorKind::WebGLRenderingContext,
    },
    ConstructorSpec {
        name: "WebGL2RenderingContext",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "WebGLObject",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "WebGLBuffer",
        parent: Some("WebGLObject"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "WebGLFramebuffer",
        parent: Some("WebGLObject"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "WebGLProgram",
        parent: Some("WebGLObject"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "WebGLRenderbuffer",
        parent: Some("WebGLObject"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "WebGLShader",
        parent: Some("WebGLObject"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "WebGLUniformLocation",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "WEBGL_debug_renderer_info",
        parent: None,
        kind: ConstructorKind::WebGlDebugRendererInfo,
    },
    ConstructorSpec {
        name: "WEBGL_lose_context",
        parent: None,
        kind: ConstructorKind::WebGlLoseContext,
    },
    ConstructorSpec {
        name: "File",
        parent: Some("Blob"),
        kind: ConstructorKind::File,
    },
    ConstructorSpec {
        name: "DataTransfer",
        parent: None,
        kind: ConstructorKind::DataTransfer,
    },
    ConstructorSpec {
        name: "DataTransferItem",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "DataTransferItemList",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "FileList",
        parent: None,
        kind: ConstructorKind::FileList,
    },
    ConstructorSpec {
        name: "FileSystem",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "FileSystemEntry",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "FileSystemFileEntry",
        parent: Some("FileSystemEntry"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "FileSystemDirectoryEntry",
        parent: Some("FileSystemEntry"),
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "FileSystemDirectoryReader",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "FileReader",
        parent: Some("EventTarget"),
        kind: ConstructorKind::FileReader,
    },
    ConstructorSpec {
        name: "FileReaderSync",
        parent: None,
        kind: ConstructorKind::FileReaderSync,
    },
    ConstructorSpec {
        name: "DOMRectReadOnly",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "DOMRect",
        parent: Some("DOMRectReadOnly"),
        kind: ConstructorKind::DomRect,
    },
    ConstructorSpec {
        name: "DOMPointReadOnly",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "DOMPoint",
        parent: Some("DOMPointReadOnly"),
        kind: ConstructorKind::DomPoint,
    },
    ConstructorSpec {
        name: "CaretPosition",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "DOMQuad",
        parent: None,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        name: "DOMMatrixReadOnly",
        parent: None,
        kind: ConstructorKind::DomMatrix,
    },
    ConstructorSpec {
        name: "DOMMatrix",
        parent: Some("DOMMatrixReadOnly"),
        kind: ConstructorKind::DomMatrix,
    },
    ConstructorSpec {
        name: "XMLSerializer",
        parent: None,
        kind: ConstructorKind::XmlSerializer,
    },
    ConstructorSpec {
        name: "ResizeObserver",
        parent: None,
        kind: ConstructorKind::ResizeObserver,
    },
    ConstructorSpec {
        name: "PerformanceObserver",
        parent: None,
        kind: ConstructorKind::PerformanceObserver,
    },
    ConstructorSpec {
        name: "PerformanceObserverEntryList",
        parent: None,
        kind: ConstructorKind::PerformanceObserverEntryList,
    },
    ConstructorSpec {
        name: "PerformanceEntry",
        parent: None,
        kind: ConstructorKind::PerformanceEntry,
    },
    ConstructorSpec {
        name: "PerformanceNavigationTiming",
        parent: Some("PerformanceEntry"),
        kind: ConstructorKind::PerformanceNavigationTiming,
    },
    ConstructorSpec {
        name: "PerformanceMark",
        parent: Some("PerformanceEntry"),
        kind: ConstructorKind::PerformanceMark,
    },
    ConstructorSpec {
        name: "PerformanceMeasure",
        parent: Some("PerformanceEntry"),
        kind: ConstructorKind::PerformanceMeasure,
    },
    ConstructorSpec {
        name: "PerformanceResourceTiming",
        parent: Some("PerformanceEntry"),
        kind: ConstructorKind::PerformanceResourceTiming,
    },
    ConstructorSpec {
        name: "EventCounts",
        parent: None,
        kind: ConstructorKind::EventCounts,
    },
    ConstructorSpec {
        name: "MediaQueryList",
        parent: None,
        kind: ConstructorKind::MediaQueryList,
    },
    ConstructorSpec {
        name: "MediaSource",
        parent: Some("EventTarget"),
        kind: ConstructorKind::MediaSource,
    },
    ConstructorSpec {
        name: "MediaError",
        parent: None,
        kind: ConstructorKind::MediaError,
    },
    ConstructorSpec {
        name: "TextTrack",
        parent: Some("EventTarget"),
        kind: ConstructorKind::TextTrack,
    },
    ConstructorSpec {
        name: "TextTrackList",
        parent: Some("EventTarget"),
        kind: ConstructorKind::TextTrackList,
    },
    ConstructorSpec {
        name: "TextTrackCue",
        parent: Some("EventTarget"),
        kind: ConstructorKind::TextTrackCue,
    },
    ConstructorSpec {
        name: "TextTrackCueList",
        parent: None,
        kind: ConstructorKind::TextTrackCueList,
    },
    ConstructorSpec {
        name: "TrackEvent",
        parent: Some("Event"),
        kind: ConstructorKind::TrackEvent,
    },
    ConstructorSpec {
        name: "VTTCue",
        parent: Some("TextTrackCue"),
        kind: ConstructorKind::VTTCue,
    },
    ConstructorSpec {
        name: "AudioContext",
        parent: Some("BaseAudioContext"),
        kind: ConstructorKind::AudioContext,
    },
    ConstructorSpec {
        name: "AudioWorkletNode",
        parent: None,
        kind: ConstructorKind::AudioWorkletNode,
    },
    ConstructorSpec {
        name: "BaseAudioContext",
        parent: Some("EventTarget"),
        kind: ConstructorKind::BaseAudioContext,
    },
    ConstructorSpec {
        name: "OfflineAudioContext",
        parent: Some("BaseAudioContext"),
        kind: ConstructorKind::OfflineAudioContext,
    },
    ConstructorSpec {
        name: "AudioDestinationNode",
        parent: None,
        kind: ConstructorKind::AudioDestinationNode,
    },
    ConstructorSpec {
        name: "OscillatorNode",
        parent: None,
        kind: ConstructorKind::OscillatorNode,
    },
    ConstructorSpec {
        name: "DynamicsCompressorNode",
        parent: None,
        kind: ConstructorKind::DynamicsCompressorNode,
    },
    ConstructorSpec {
        name: "AnalyserNode",
        parent: None,
        kind: ConstructorKind::AnalyserNode,
    },
    ConstructorSpec {
        name: "AudioParam",
        parent: None,
        kind: ConstructorKind::AudioParam,
    },
    ConstructorSpec {
        name: "AudioBuffer",
        parent: None,
        kind: ConstructorKind::AudioBuffer,
    },
    ConstructorSpec {
        name: "AbstractRange",
        parent: None,
        kind: ConstructorKind::AbstractRange,
    },
    ConstructorSpec {
        name: "Range",
        parent: Some("AbstractRange"),
        kind: ConstructorKind::Range,
    },
    ConstructorSpec {
        name: "StaticRange",
        parent: Some("AbstractRange"),
        kind: ConstructorKind::StaticRange,
    },
    ConstructorSpec {
        name: "URL",
        parent: None,
        kind: ConstructorKind::Url,
    },
    ConstructorSpec {
        name: "URLSearchParams",
        parent: None,
        kind: ConstructorKind::UrlSearchParams,
    },
    ConstructorSpec {
        name: "FormData",
        parent: None,
        kind: ConstructorKind::FormData,
    },
    ConstructorSpec {
        name: "IDBFactory",
        parent: None,
        kind: ConstructorKind::IndexedDb,
    },
    ConstructorSpec {
        name: "IDBRequest",
        parent: Some("EventTarget"),
        kind: ConstructorKind::IndexedDb,
    },
    ConstructorSpec {
        name: "IDBOpenDBRequest",
        parent: Some("IDBRequest"),
        kind: ConstructorKind::IndexedDb,
    },
    ConstructorSpec {
        name: "IDBDatabase",
        parent: Some("EventTarget"),
        kind: ConstructorKind::IndexedDb,
    },
    ConstructorSpec {
        name: "IDBTransaction",
        parent: Some("EventTarget"),
        kind: ConstructorKind::IndexedDb,
    },
    ConstructorSpec {
        name: "IDBObjectStore",
        parent: None,
        kind: ConstructorKind::IndexedDb,
    },
    ConstructorSpec {
        name: "IDBIndex",
        parent: None,
        kind: ConstructorKind::IndexedDb,
    },
    ConstructorSpec {
        name: "IDBCursor",
        parent: None,
        kind: ConstructorKind::IndexedDb,
    },
    ConstructorSpec {
        name: "IDBCursorWithValue",
        parent: Some("IDBCursor"),
        kind: ConstructorKind::IndexedDb,
    },
    ConstructorSpec {
        name: "IDBKeyRange",
        parent: None,
        kind: ConstructorKind::IndexedDb,
    },
    ConstructorSpec {
        name: "IDBVersionChangeEvent",
        parent: Some("Event"),
        kind: ConstructorKind::IndexedDbVersionChangeEvent,
    },
];

pub(in crate::context_bootstrap) fn constructor_specs() -> Vec<ConstructorSpec> {
    let mut seen = HashSet::new();
    CONSTRUCTOR_SPECS_BEFORE_STREAMS
        .iter()
        .copied()
        .chain(crate::context_bootstrap::streams::stream_constructor_specs())
        .chain(CONSTRUCTOR_SPECS_AFTER_STREAMS.iter().copied())
        .chain(
            node_bridge_descriptors()
                .iter()
                .map(|descriptor| ConstructorSpec {
                    name: descriptor.constructor_name,
                    parent: descriptor.parent_constructor,
                    kind: constructor_kind_for_bridge_descriptor(descriptor.constructor_name),
                }),
        )
        .filter(|spec| seen.insert(spec.name))
        .collect()
}

fn constructor_kind_for_bridge_descriptor(name: &str) -> ConstructorKind {
    match name {
        "Document" => ConstructorKind::Document,
        "DocumentFragment" => ConstructorKind::DocumentFragment,
        "Text" => ConstructorKind::Text,
        "Comment" => ConstructorKind::Comment,
        _ if is_html_element_constructor_name(name) => ConstructorKind::HtmlElement,
        _ => ConstructorKind::Illegal,
    }
}

fn is_html_element_constructor_name(name: &str) -> bool {
    name == "HTMLElement" || name.starts_with("HTML") && name.ends_with("Element")
}
