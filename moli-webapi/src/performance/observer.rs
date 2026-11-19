use moli_webapi_declare::WebApiObject;
use moli_webidl::{self as webidl, SnapshotPrototype, WebIdlDictionarySerialize};

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PerformanceObserver.observe")]
pub struct PerformanceObserverObserveArgs {
    #[webidl(required, dictionary)]
    pub options: PerformanceObserverInit,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "PerformanceObserverInit")]
pub struct PerformanceObserverInit {
    pub buffered: Option<bool>,
    #[webidl(converter = "double")]
    pub duration_threshold: Option<f64>,
    #[webidl(converter = "raw")]
    pub entry_types: Option<webidl::Sequence<webidl::DomString>>,
    #[webidl(name = "type")]
    pub observed_type: Option<String>,
}

#[derive(WebApiObject)]
#[webapi(fragment, prototype = "PerformanceObserver")]
struct PerformanceObserverConstructorDeclaration {
    #[webapi(data_property = "supportedEntryTypes")]
    supported_entry_types: &'static [&'static str],
}

pub fn finalize_performance_observer_realm_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor: v8::Local<'s, v8::Object>,
    supported_entry_types: &'static [&'static str],
) -> anyhow::Result<()> {
    PerformanceObserverConstructorDeclaration::new(supported_entry_types)
        .initialize(scope, constructor)
        .map_err(|error| anyhow::anyhow!("failed to finalize PerformanceObserver: {error}"))
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PerformanceObserverCallbackOptions", snapshot)]
struct PerformanceObserverCallbackOptions {
    #[webidl(converter = "unsigned_long_long")]
    dropped_entries_count: Option<u64>,
}

pub fn build_performance_observer_callback_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    dropped_entries_count: Option<u64>,
) -> v8::Local<'s, v8::Object> {
    PerformanceObserverCallbackOptions {
        dropped_entries_count,
    }
    .to_js_snapshot(scope, SnapshotPrototype::Ordinary)
    .expect("PerformanceObserver callback options should serialize")
}
