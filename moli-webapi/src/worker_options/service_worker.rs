use super::WorkerScriptKind;
use moli_webidl as webidl;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, webidl::WebIdlEnum)]
#[webidl(
    name = "ServiceWorkerUpdateViaCache",
    parse_with = Self::parse_webidl_token
)]
pub enum ServiceWorkerUpdateViaCache {
    #[default]
    Imports,
    All,
    None,
}

impl ServiceWorkerUpdateViaCache {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Imports => "imports",
            Self::All => "all",
            Self::None => "none",
        }
    }

    pub fn parse_webidl_token(value: &str) -> Option<Self> {
        match value {
            "imports" => Some(Self::Imports),
            "all" => Some(Self::All),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RegistrationOptions")]
pub struct ServiceWorkerRegistrationOptions {
    #[webidl(name = "scope", converter = "usv_string")]
    pub requested_scope: Option<String>,

    #[webidl(
        name = "type",
        converter = "enum",
        default = WorkerScriptKind::Classic
    )]
    pub script_kind: WorkerScriptKind,

    #[webidl(
        name = "updateViaCache",
        converter = "enum",
        default = ServiceWorkerUpdateViaCache::Imports
    )]
    pub update_via_cache: ServiceWorkerUpdateViaCache,
}

impl Default for ServiceWorkerRegistrationOptions {
    fn default() -> Self {
        Self {
            requested_scope: None,
            script_kind: WorkerScriptKind::Classic,
            update_via_cache: ServiceWorkerUpdateViaCache::Imports,
        }
    }
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "BackgroundSyncOptions")]
pub struct BackgroundSyncOptions {
    #[webidl(
        name = "minInterval",
        converter = "enforce_range_unsigned_long_long",
        default = 0
    )]
    pub min_interval: u64,
}

pub fn service_worker_periodic_sync_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<BackgroundSyncOptions> {
    match webidl::parse_dictionary::<BackgroundSyncOptions>(
        scope,
        value,
        webidl::Context::argument("PeriodicSyncManager.register", 2),
    ) {
        Ok(options) => Some(options.unwrap_or_default()),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

pub fn service_worker_navigation_preload_header_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<String, webidl::WebIdlError> {
    let context = webidl::Context::argument("NavigationPreloadManager.setHeaderValue", 1);
    if args.length() <= 0 {
        return Err(webidl::WebIdlError::missing_required(context));
    }
    webidl::convert::<webidl::ByteString>(scope, args.get(0), context).map(Into::into)
}
