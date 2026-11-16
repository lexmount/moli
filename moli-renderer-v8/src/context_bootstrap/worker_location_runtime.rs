use super::*;
use anyhow::Result;
pub(crate) fn install_worker_location_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    script_url: &url::Url,
) -> Result<()> {
    install_worker_constructor(scope, global, "WorkerLocation")?;
    if let Some(prototype) = global_constructor_prototype(scope, "WorkerLocation") {
        moli_webapi::worker_location::install_worker_location_prototype_bindings(scope, prototype)?;
    }
    moli_webapi::worker_location::install_worker_location_value(scope, global, script_url)
}
