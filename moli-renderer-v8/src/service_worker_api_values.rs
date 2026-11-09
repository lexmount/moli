use crate::webidl;

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "BackgroundSyncOptions")]
pub(crate) struct BackgroundSyncOptions {
    #[webidl(
        name = "minInterval",
        converter = "enforce_range_unsigned_long_long",
        default = 0
    )]
    pub(crate) min_interval: u64,
}

pub(crate) fn service_worker_periodic_sync_options<'s>(
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

pub(crate) fn service_worker_navigation_preload_header_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<String, webidl::WebIdlError> {
    let context = webidl::Context::argument("NavigationPreloadManager.setHeaderValue", 1);
    if args.length() <= 0 {
        return Err(webidl::WebIdlError::missing_required(context));
    }
    webidl::convert::<webidl::ByteString>(scope, args.get(0), context).map(Into::into)
}
