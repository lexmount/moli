use super::{ConstructorKind, ConstructorSpec};
use crate::context_bootstrap::bridge_descriptor::node_bridge_descriptors;
use crate::web_api_interfaces;
use std::collections::HashSet;

const CONSTRUCTOR_SPECS_BEFORE_STREAMS: &[ConstructorSpec] = &[
    ConstructorSpec {
        interface: web_api_interfaces::NodeList::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::HTMLCollection::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::HTMLFormControlsCollection::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::HTMLOptionsCollection::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::RadioNodeList::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::ValidityState::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DOMTokenList::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DOMStringMap::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DOMStringList::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DOMRectList::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::PluginArray::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::MimeTypeArray::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Plugin::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::MimeType::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CustomElementRegistry::DESCRIPTOR,
        kind: ConstructorKind::CustomElementRegistry,
    },
    ConstructorSpec {
        interface: web_api_interfaces::ElementInternals::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CustomStateSet::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Attr::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Option::DESCRIPTOR,
        kind: ConstructorKind::Option,
    },
    ConstructorSpec {
        interface: web_api_interfaces::NamedNodeMap::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGLength::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGAngle::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGNumber::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGRect::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGAnimatedString::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGAnimatedLength::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGAnimatedAngle::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGAnimatedRect::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGPreserveAspectRatio::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGAnimatedPreserveAspectRatio::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGLengthList::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGAnimatedLengthList::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGAnimatedNumber::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGAnimatedInteger::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGNumberList::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGStringList::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGAnimatedNumberList::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGAnimatedBoolean::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGAnimatedEnumeration::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGUnitTypes::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGAnimatedTransformList::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGTransformList::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGTransform::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SVGMatrix::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSStyleDeclaration::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::StylePropertyMapReadOnly::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSStyleValue::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSKeywordValue::DESCRIPTOR,
        kind: ConstructorKind::CssKeywordValue,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSNumericValue::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSUnitValue::DESCRIPTOR,
        kind: ConstructorKind::CssUnitValue,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSStyleProperties::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSFontFaceDescriptors::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSPageDescriptors::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSFontFeatureValuesMap::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::HTMLAllCollection::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Event::DESCRIPTOR,
        kind: ConstructorKind::Event,
    },
    ConstructorSpec {
        interface: web_api_interfaces::UIEvent::DESCRIPTOR,
        kind: ConstructorKind::UiEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::FocusEvent::DESCRIPTOR,
        kind: ConstructorKind::FocusEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::TextEvent::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CompositionEvent::DESCRIPTOR,
        kind: ConstructorKind::CompositionEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CustomEvent::DESCRIPTOR,
        kind: ConstructorKind::CustomEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::MouseEvent::DESCRIPTOR,
        kind: ConstructorKind::MouseEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CapturedMouseEvent::DESCRIPTOR,
        kind: ConstructorKind::CapturedMouseEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DragEvent::DESCRIPTOR,
        kind: ConstructorKind::DragEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::ClipboardEvent::DESCRIPTOR,
        kind: ConstructorKind::ClipboardEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::KeyboardEvent::DESCRIPTOR,
        kind: ConstructorKind::KeyboardEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::InputEvent::DESCRIPTOR,
        kind: ConstructorKind::InputEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::WheelEvent::DESCRIPTOR,
        kind: ConstructorKind::WheelEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::PointerEvent::DESCRIPTOR,
        kind: ConstructorKind::PointerEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::TouchEvent::DESCRIPTOR,
        kind: ConstructorKind::TouchEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::MessageEvent::DESCRIPTOR,
        kind: ConstructorKind::MessageEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::StorageEvent::DESCRIPTOR,
        kind: ConstructorKind::StorageEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::ErrorEvent::DESCRIPTOR,
        kind: ConstructorKind::ErrorEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::PromiseRejectionEvent::DESCRIPTOR,
        kind: ConstructorKind::PromiseRejectionEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::NavigationCurrentEntryChangeEvent::DESCRIPTOR,
        kind: ConstructorKind::NavigationCurrentEntryChangeEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::NavigateEvent::DESCRIPTOR,
        kind: ConstructorKind::NavigateEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CloseEvent::DESCRIPTOR,
        kind: ConstructorKind::CloseEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SubmitEvent::DESCRIPTOR,
        kind: ConstructorKind::SubmitEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::FormDataEvent::DESCRIPTOR,
        kind: ConstructorKind::FormDataEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::PopStateEvent::DESCRIPTOR,
        kind: ConstructorKind::PopStateEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::PageTransitionEvent::DESCRIPTOR,
        kind: ConstructorKind::PageTransitionEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::BeforeUnloadEvent::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::HashChangeEvent::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::MediaQueryListEvent::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SecurityPolicyViolationEvent::DESCRIPTOR,
        kind: ConstructorKind::SecurityPolicyViolationEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::ToggleEvent::DESCRIPTOR,
        kind: ConstructorKind::ToggleEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CommandEvent::DESCRIPTOR,
        kind: ConstructorKind::CommandEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::InterestEvent::DESCRIPTOR,
        kind: ConstructorKind::InterestEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::ContentVisibilityAutoStateChangeEvent::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DOMException::DESCRIPTOR,
        kind: ConstructorKind::DomException,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DOMError::DESCRIPTOR,
        kind: ConstructorKind::DomError,
    },
    ConstructorSpec {
        interface: web_api_interfaces::QuotaExceededError::DESCRIPTOR,
        kind: ConstructorKind::QuotaExceededError,
    },
    ConstructorSpec {
        interface: web_api_interfaces::AbortSignal::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::AbortController::DESCRIPTOR,
        kind: ConstructorKind::AbortController,
    },
    ConstructorSpec {
        interface: web_api_interfaces::BroadcastChannel::DESCRIPTOR,
        kind: ConstructorKind::BroadcastChannel,
    },
    ConstructorSpec {
        interface: web_api_interfaces::EventSource::DESCRIPTOR,
        kind: ConstructorKind::EventSource,
    },
    ConstructorSpec {
        interface: web_api_interfaces::IdleDetector::DESCRIPTOR,
        kind: ConstructorKind::IdleDetector,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Notification::DESCRIPTOR,
        kind: ConstructorKind::Notification,
    },
    ConstructorSpec {
        interface: web_api_interfaces::MessageChannel::DESCRIPTOR,
        kind: ConstructorKind::MessageChannel,
    },
    ConstructorSpec {
        interface: web_api_interfaces::MessagePort::DESCRIPTOR,
        kind: ConstructorKind::MessagePort,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Worker::DESCRIPTOR,
        kind: ConstructorKind::Worker,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SharedWorker::DESCRIPTOR,
        kind: ConstructorKind::SharedWorker,
    },
    ConstructorSpec {
        interface: web_api_interfaces::WorkerNavigator::DESCRIPTOR,
        kind: ConstructorKind::WorkerNavigator,
    },
    ConstructorSpec {
        interface: web_api_interfaces::WorkerLocation::DESCRIPTOR,
        kind: ConstructorKind::WorkerLocation,
    },
    ConstructorSpec {
        interface: web_api_interfaces::WebSocket::DESCRIPTOR,
        kind: ConstructorKind::WebSocket,
    },
    ConstructorSpec {
        interface: web_api_interfaces::WebSocketError::DESCRIPTOR,
        kind: ConstructorKind::WebSocketError,
    },
    ConstructorSpec {
        interface: web_api_interfaces::WebSocketStream::DESCRIPTOR,
        kind: ConstructorKind::WebSocketStream,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Performance::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::PerformanceTiming::DESCRIPTOR,
        kind: ConstructorKind::PerformanceTiming,
    },
    ConstructorSpec {
        interface: web_api_interfaces::PerformanceNavigation::DESCRIPTOR,
        kind: ConstructorKind::PerformanceNavigation,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Animation::DESCRIPTOR,
        kind: ConstructorKind::Animation,
    },
    ConstructorSpec {
        interface: web_api_interfaces::AnimationEffect::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::KeyframeEffect::DESCRIPTOR,
        kind: ConstructorKind::KeyframeEffect,
    },
    ConstructorSpec {
        interface: web_api_interfaces::AnimationTimeline::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DocumentTimeline::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::AnimationPlaybackEvent::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::ViewTransition::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::ViewTransitionTypeSet::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Crypto::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SubtleCrypto::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CryptoKey::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::VisualViewport::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Navigator::DESCRIPTOR,
        kind: ConstructorKind::Navigator,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Permissions::DESCRIPTOR,
        kind: ConstructorKind::Permissions,
    },
    ConstructorSpec {
        interface: web_api_interfaces::PermissionStatus::DESCRIPTOR,
        kind: ConstructorKind::PermissionStatus,
    },
    ConstructorSpec {
        interface: web_api_interfaces::NavigatorUAData::DESCRIPTOR,
        kind: ConstructorKind::NavigatorUAData,
    },
    ConstructorSpec {
        interface: web_api_interfaces::StorageManager::DESCRIPTOR,
        kind: ConstructorKind::StorageManager,
    },
    ConstructorSpec {
        interface: web_api_interfaces::StorageEstimate::DESCRIPTOR,
        kind: ConstructorKind::StorageEstimate,
    },
    ConstructorSpec {
        interface: web_api_interfaces::StorageAccessHandle::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::StorageBucketManager::DESCRIPTOR,
        kind: ConstructorKind::StorageBucketManager,
    },
    ConstructorSpec {
        interface: web_api_interfaces::StorageBucket::DESCRIPTOR,
        kind: ConstructorKind::StorageBucket,
    },
    ConstructorSpec {
        interface: web_api_interfaces::FileSystemHandle::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::FileSystemFileHandle::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::FileSystemDirectoryHandle::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::FileSystemWritableFileStream::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::FileSystemSyncAccessHandle::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::MediaDevices::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Clipboard::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::ClipboardItem::DESCRIPTOR,
        kind: ConstructorKind::ClipboardItem,
    },
    ConstructorSpec {
        interface: web_api_interfaces::MediaCapabilities::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Touch::DESCRIPTOR,
        kind: ConstructorKind::Touch,
    },
    ConstructorSpec {
        interface: web_api_interfaces::TouchList::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Screen::DESCRIPTOR,
        kind: ConstructorKind::Screen,
    },
    ConstructorSpec {
        interface: web_api_interfaces::ScreenOrientation::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SpeechSynthesis::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SpeechSynthesisUtterance::DESCRIPTOR,
        kind: ConstructorKind::SpeechSynthesisUtterance,
    },
    ConstructorSpec {
        interface: web_api_interfaces::SpeechSynthesisVoice::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Selection::DESCRIPTOR,
        kind: ConstructorKind::Selection,
    },
    ConstructorSpec {
        interface: web_api_interfaces::History::DESCRIPTOR,
        kind: ConstructorKind::History,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Location::DESCRIPTOR,
        kind: ConstructorKind::Location,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Navigation::DESCRIPTOR,
        kind: ConstructorKind::Navigation,
    },
    ConstructorSpec {
        interface: web_api_interfaces::NavigationHistoryEntry::DESCRIPTOR,
        kind: ConstructorKind::NavigationHistoryEntry,
    },
    ConstructorSpec {
        interface: web_api_interfaces::NavigationActivation::DESCRIPTOR,
        kind: ConstructorKind::NavigationActivation,
    },
    ConstructorSpec {
        interface: web_api_interfaces::NavigationTransition::DESCRIPTOR,
        kind: ConstructorKind::NavigationTransition,
    },
    ConstructorSpec {
        interface: web_api_interfaces::MutationObserver::DESCRIPTOR,
        kind: ConstructorKind::MutationObserver,
    },
    ConstructorSpec {
        interface: web_api_interfaces::MutationRecord::DESCRIPTOR,
        kind: ConstructorKind::MutationRecord,
    },
    ConstructorSpec {
        interface: web_api_interfaces::IntersectionObserver::DESCRIPTOR,
        kind: ConstructorKind::IntersectionObserver,
    },
    ConstructorSpec {
        interface: web_api_interfaces::IntersectionObserverEntry::DESCRIPTOR,
        kind: ConstructorKind::IntersectionObserverEntry,
    },
    ConstructorSpec {
        interface: web_api_interfaces::EventTarget::DESCRIPTOR,
        kind: ConstructorKind::EventTarget,
    },
    ConstructorSpec {
        interface: web_api_interfaces::IdleDeadline::DESCRIPTOR,
        kind: ConstructorKind::IdleDeadline,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Window::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CharacterData::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DOMImplementation::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::NodeIterator::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::TreeWalker::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::XPathEvaluator::DESCRIPTOR,
        kind: ConstructorKind::XPathEvaluator,
    },
    ConstructorSpec {
        interface: web_api_interfaces::XPathResult::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::StyleSheet::DESCRIPTOR,
        kind: ConstructorKind::StyleSheet,
    },
    ConstructorSpec {
        interface: web_api_interfaces::StyleSheetList::DESCRIPTOR,
        kind: ConstructorKind::StyleSheetList,
    },
    ConstructorSpec {
        interface: web_api_interfaces::MediaList::DESCRIPTOR,
        kind: ConstructorKind::MediaList,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSRuleList::DESCRIPTOR,
        kind: ConstructorKind::CssRuleList,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSRule::DESCRIPTOR,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSGroupingRule::DESCRIPTOR,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSConditionRule::DESCRIPTOR,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSMediaRule::DESCRIPTOR,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSSupportsRule::DESCRIPTOR,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSContainerRule::DESCRIPTOR,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSLayerBlockRule::DESCRIPTOR,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSLayerStatementRule::DESCRIPTOR,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSScopeRule::DESCRIPTOR,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSImportRule::DESCRIPTOR,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSFontFaceRule::DESCRIPTOR,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSFontFeatureValuesRule::DESCRIPTOR,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSPropertyRule::DESCRIPTOR,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSKeyframesRule::DESCRIPTOR,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSKeyframeRule::DESCRIPTOR,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSPageRule::DESCRIPTOR,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSMarginRule::DESCRIPTOR,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSNamespaceRule::DESCRIPTOR,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSCounterStyleRule::DESCRIPTOR,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSStyleRule::DESCRIPTOR,
        kind: ConstructorKind::CssStyleRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSNestedDeclarations::DESCRIPTOR,
        kind: ConstructorKind::CssRule,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CSSStyleSheet::DESCRIPTOR,
        kind: ConstructorKind::CssStyleSheet,
    },
    ConstructorSpec {
        interface: web_api_interfaces::FontFace::DESCRIPTOR,
        kind: ConstructorKind::FontFace,
    },
    ConstructorSpec {
        interface: web_api_interfaces::FontFaceSet::DESCRIPTOR,
        kind: ConstructorKind::FontFaceSet,
    },
    ConstructorSpec {
        interface: web_api_interfaces::FontFaceSetLoadEvent::DESCRIPTOR,
        kind: ConstructorKind::FontFaceSetLoadEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Audio::DESCRIPTOR,
        kind: ConstructorKind::Audio,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Image::DESCRIPTOR,
        kind: ConstructorKind::Image,
    },
    ConstructorSpec {
        interface: web_api_interfaces::XMLHttpRequestEventTarget::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::XMLHttpRequestUpload::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::XMLHttpRequest::DESCRIPTOR,
        kind: ConstructorKind::XmlHttpRequest,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Headers::DESCRIPTOR,
        kind: ConstructorKind::Headers,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Request::DESCRIPTOR,
        kind: ConstructorKind::Request,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Response::DESCRIPTOR,
        kind: ConstructorKind::Response,
    },
    ConstructorSpec {
        interface: web_api_interfaces::ProgressEvent::DESCRIPTOR,
        kind: ConstructorKind::ProgressEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DOMParser::DESCRIPTOR,
        kind: ConstructorKind::DomParser,
    },
    ConstructorSpec {
        interface: web_api_interfaces::TextEncoder::DESCRIPTOR,
        kind: ConstructorKind::TextEncoder,
    },
    ConstructorSpec {
        interface: web_api_interfaces::TextDecoder::DESCRIPTOR,
        kind: ConstructorKind::TextDecoder,
    },
];

const CONSTRUCTOR_SPECS_AFTER_STREAMS: &[ConstructorSpec] = &[
    ConstructorSpec {
        interface: web_api_interfaces::Geolocation::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::GeolocationPosition::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::GeolocationCoordinates::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::GeolocationPositionError::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Blob::DESCRIPTOR,
        kind: ConstructorKind::Blob,
    },
    ConstructorSpec {
        interface: web_api_interfaces::ImageData::DESCRIPTOR,
        kind: ConstructorKind::ImageData,
    },
    ConstructorSpec {
        interface: web_api_interfaces::ImageBitmap::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::RTCPeerConnection::DESCRIPTOR,
        kind: ConstructorKind::RtcPeerConnection,
    },
    ConstructorSpec {
        interface: web_api_interfaces::RTCIceCandidate::DESCRIPTOR,
        kind: ConstructorKind::RtcIceCandidate,
    },
    ConstructorSpec {
        interface: web_api_interfaces::RTCSessionDescription::DESCRIPTOR,
        kind: ConstructorKind::RtcSessionDescription,
    },
    ConstructorSpec {
        interface: web_api_interfaces::RTCRtpReceiver::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::RTCDataChannel::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CanvasGradient::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CanvasPattern::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::TextMetrics::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Path2D::DESCRIPTOR,
        kind: ConstructorKind::Unsupported,
    },
    ConstructorSpec {
        interface: web_api_interfaces::OffscreenCanvas::DESCRIPTOR,
        kind: ConstructorKind::OffscreenCanvas,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CanvasRenderingContext2D::DESCRIPTOR,
        kind: ConstructorKind::CanvasRenderingContext2D,
    },
    ConstructorSpec {
        interface: web_api_interfaces::OffscreenCanvasRenderingContext2D::DESCRIPTOR,
        kind: ConstructorKind::OffscreenCanvasRenderingContext2D,
    },
    ConstructorSpec {
        interface: web_api_interfaces::WebGLRenderingContext::DESCRIPTOR,
        kind: ConstructorKind::WebGLRenderingContext,
    },
    ConstructorSpec {
        interface: web_api_interfaces::WebGL2RenderingContext::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::WebGLObject::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::WebGLBuffer::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::WebGLFramebuffer::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::WebGLProgram::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::WebGLRenderbuffer::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::WebGLShader::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::WebGLUniformLocation::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::WEBGLDebugRendererInfo::DESCRIPTOR,
        kind: ConstructorKind::WebGlDebugRendererInfo,
    },
    ConstructorSpec {
        interface: web_api_interfaces::WEBGLLoseContext::DESCRIPTOR,
        kind: ConstructorKind::WebGlLoseContext,
    },
    ConstructorSpec {
        interface: web_api_interfaces::File::DESCRIPTOR,
        kind: ConstructorKind::File,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DataTransfer::DESCRIPTOR,
        kind: ConstructorKind::DataTransfer,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DataTransferItem::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DataTransferItemList::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::FileList::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::FileSystem::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::FileSystemEntry::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::FileSystemFileEntry::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::FileSystemDirectoryEntry::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::FileSystemDirectoryReader::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::FileReader::DESCRIPTOR,
        kind: ConstructorKind::FileReader,
    },
    ConstructorSpec {
        interface: web_api_interfaces::FileReaderSync::DESCRIPTOR,
        kind: ConstructorKind::FileReaderSync,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DOMRectReadOnly::DESCRIPTOR,
        kind: ConstructorKind::DomRectReadOnly,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DOMRect::DESCRIPTOR,
        kind: ConstructorKind::DomRect,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DOMPointReadOnly::DESCRIPTOR,
        kind: ConstructorKind::DomPointReadOnly,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DOMPoint::DESCRIPTOR,
        kind: ConstructorKind::DomPoint,
    },
    ConstructorSpec {
        interface: web_api_interfaces::CaretPosition::DESCRIPTOR,
        kind: ConstructorKind::Illegal,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DOMQuad::DESCRIPTOR,
        kind: ConstructorKind::DomQuad,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DOMMatrixReadOnly::DESCRIPTOR,
        kind: ConstructorKind::DomMatrixReadOnly,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DOMMatrix::DESCRIPTOR,
        kind: ConstructorKind::DomMatrix,
    },
    ConstructorSpec {
        interface: web_api_interfaces::XMLSerializer::DESCRIPTOR,
        kind: ConstructorKind::XmlSerializer,
    },
    ConstructorSpec {
        interface: web_api_interfaces::ResizeObserver::DESCRIPTOR,
        kind: ConstructorKind::ResizeObserver,
    },
    ConstructorSpec {
        interface: web_api_interfaces::PerformanceObserver::DESCRIPTOR,
        kind: ConstructorKind::PerformanceObserver,
    },
    ConstructorSpec {
        interface: web_api_interfaces::PerformanceObserverEntryList::DESCRIPTOR,
        kind: ConstructorKind::PerformanceObserverEntryList,
    },
    ConstructorSpec {
        interface: web_api_interfaces::PerformanceEntry::DESCRIPTOR,
        kind: ConstructorKind::PerformanceEntry,
    },
    ConstructorSpec {
        interface: web_api_interfaces::PerformanceNavigationTiming::DESCRIPTOR,
        kind: ConstructorKind::PerformanceNavigationTiming,
    },
    ConstructorSpec {
        interface: web_api_interfaces::PerformanceMark::DESCRIPTOR,
        kind: ConstructorKind::PerformanceMark,
    },
    ConstructorSpec {
        interface: web_api_interfaces::PerformanceMeasure::DESCRIPTOR,
        kind: ConstructorKind::PerformanceMeasure,
    },
    ConstructorSpec {
        interface: web_api_interfaces::PerformanceResourceTiming::DESCRIPTOR,
        kind: ConstructorKind::PerformanceResourceTiming,
    },
    ConstructorSpec {
        interface: web_api_interfaces::EventCounts::DESCRIPTOR,
        kind: ConstructorKind::EventCounts,
    },
    ConstructorSpec {
        interface: web_api_interfaces::MediaQueryList::DESCRIPTOR,
        kind: ConstructorKind::MediaQueryList,
    },
    ConstructorSpec {
        interface: web_api_interfaces::MediaSource::DESCRIPTOR,
        kind: ConstructorKind::MediaSource,
    },
    ConstructorSpec {
        interface: web_api_interfaces::MediaError::DESCRIPTOR,
        kind: ConstructorKind::MediaError,
    },
    ConstructorSpec {
        interface: web_api_interfaces::TextTrack::DESCRIPTOR,
        kind: ConstructorKind::TextTrack,
    },
    ConstructorSpec {
        interface: web_api_interfaces::TextTrackList::DESCRIPTOR,
        kind: ConstructorKind::TextTrackList,
    },
    ConstructorSpec {
        interface: web_api_interfaces::TextTrackCue::DESCRIPTOR,
        kind: ConstructorKind::TextTrackCue,
    },
    ConstructorSpec {
        interface: web_api_interfaces::TextTrackCueList::DESCRIPTOR,
        kind: ConstructorKind::TextTrackCueList,
    },
    ConstructorSpec {
        interface: web_api_interfaces::TrackEvent::DESCRIPTOR,
        kind: ConstructorKind::TrackEvent,
    },
    ConstructorSpec {
        interface: web_api_interfaces::VTTCue::DESCRIPTOR,
        kind: ConstructorKind::VTTCue,
    },
    ConstructorSpec {
        interface: web_api_interfaces::AudioContext::DESCRIPTOR,
        kind: ConstructorKind::AudioContext,
    },
    ConstructorSpec {
        interface: web_api_interfaces::AudioWorkletNode::DESCRIPTOR,
        kind: ConstructorKind::AudioWorkletNode,
    },
    ConstructorSpec {
        interface: web_api_interfaces::BaseAudioContext::DESCRIPTOR,
        kind: ConstructorKind::BaseAudioContext,
    },
    ConstructorSpec {
        interface: web_api_interfaces::OfflineAudioContext::DESCRIPTOR,
        kind: ConstructorKind::OfflineAudioContext,
    },
    ConstructorSpec {
        interface: web_api_interfaces::AudioDestinationNode::DESCRIPTOR,
        kind: ConstructorKind::AudioDestinationNode,
    },
    ConstructorSpec {
        interface: web_api_interfaces::OscillatorNode::DESCRIPTOR,
        kind: ConstructorKind::OscillatorNode,
    },
    ConstructorSpec {
        interface: web_api_interfaces::DynamicsCompressorNode::DESCRIPTOR,
        kind: ConstructorKind::DynamicsCompressorNode,
    },
    ConstructorSpec {
        interface: web_api_interfaces::AnalyserNode::DESCRIPTOR,
        kind: ConstructorKind::AnalyserNode,
    },
    ConstructorSpec {
        interface: web_api_interfaces::AudioParam::DESCRIPTOR,
        kind: ConstructorKind::AudioParam,
    },
    ConstructorSpec {
        interface: web_api_interfaces::BiquadFilterNode::DESCRIPTOR,
        kind: ConstructorKind::BiquadFilterNode,
    },
    ConstructorSpec {
        interface: web_api_interfaces::AudioBuffer::DESCRIPTOR,
        kind: ConstructorKind::AudioBuffer,
    },
    ConstructorSpec {
        interface: web_api_interfaces::AbstractRange::DESCRIPTOR,
        kind: ConstructorKind::AbstractRange,
    },
    ConstructorSpec {
        interface: web_api_interfaces::Range::DESCRIPTOR,
        kind: ConstructorKind::Range,
    },
    ConstructorSpec {
        interface: web_api_interfaces::StaticRange::DESCRIPTOR,
        kind: ConstructorKind::StaticRange,
    },
    ConstructorSpec {
        interface: web_api_interfaces::URL::DESCRIPTOR,
        kind: ConstructorKind::Url,
    },
    ConstructorSpec {
        interface: web_api_interfaces::URLSearchParams::DESCRIPTOR,
        kind: ConstructorKind::UrlSearchParams,
    },
    ConstructorSpec {
        interface: web_api_interfaces::FormData::DESCRIPTOR,
        kind: ConstructorKind::FormData,
    },
    ConstructorSpec {
        interface: web_api_interfaces::IDBFactory::DESCRIPTOR,
        kind: ConstructorKind::IndexedDb,
    },
    ConstructorSpec {
        interface: web_api_interfaces::IDBRequest::DESCRIPTOR,
        kind: ConstructorKind::IndexedDb,
    },
    ConstructorSpec {
        interface: web_api_interfaces::IDBOpenDBRequest::DESCRIPTOR,
        kind: ConstructorKind::IndexedDb,
    },
    ConstructorSpec {
        interface: web_api_interfaces::IDBDatabase::DESCRIPTOR,
        kind: ConstructorKind::IndexedDb,
    },
    ConstructorSpec {
        interface: web_api_interfaces::IDBTransaction::DESCRIPTOR,
        kind: ConstructorKind::IndexedDb,
    },
    ConstructorSpec {
        interface: web_api_interfaces::IDBObjectStore::DESCRIPTOR,
        kind: ConstructorKind::IndexedDb,
    },
    ConstructorSpec {
        interface: web_api_interfaces::IDBIndex::DESCRIPTOR,
        kind: ConstructorKind::IndexedDb,
    },
    ConstructorSpec {
        interface: web_api_interfaces::IDBCursor::DESCRIPTOR,
        kind: ConstructorKind::IndexedDb,
    },
    ConstructorSpec {
        interface: web_api_interfaces::IDBCursorWithValue::DESCRIPTOR,
        kind: ConstructorKind::IndexedDb,
    },
    ConstructorSpec {
        interface: web_api_interfaces::IDBKeyRange::DESCRIPTOR,
        kind: ConstructorKind::IndexedDb,
    },
    ConstructorSpec {
        interface: web_api_interfaces::IDBVersionChangeEvent::DESCRIPTOR,
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
                    interface: descriptor.interface,
                    kind: constructor_kind_for_bridge_descriptor(descriptor.interface.name()),
                }),
        )
        .filter(|spec| seen.insert(spec.interface.name()))
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
