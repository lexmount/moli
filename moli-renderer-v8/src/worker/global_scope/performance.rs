//! Worker performance clock and time origin.

use super::*;

#[derive(WebApiObject)]
#[webapi(plain)]
struct WorkerGlobalPerformanceDeclaration<'scope> {
    #[webapi(data_property)]
    performance: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(prototype = "Object", interface = web_api_interfaces::Performance)]
struct WorkerPerformanceObjectDeclaration {
    #[webapi(data_property, readonly)]
    time_origin: f64,
    #[webapi(method, callback = worker_performance_now_callback, data = self.time_origin)]
    now: (),
}

pub(super) fn install_worker_performance<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let time_origin = monotonic_unix_epoch_millis();
    let performance = WorkerPerformanceObjectDeclaration::new(time_origin)
        .bind(scope)
        .map_err(|error| anyhow!("failed to create worker performance: {error}"))?;
    WorkerGlobalPerformanceDeclaration::new(performance)
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize worker performance: {error}"))
}

fn worker_performance_now_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let time_origin = args.data().number_value(scope).unwrap_or(0.0);
    rv.set(
        v8::Number::new(
            scope,
            moli_time::coarsened_dom_time_millis(
                (monotonic_unix_epoch_millis() - time_origin).max(0.0),
            ),
        )
        .into(),
    );
}

fn unix_epoch_millis() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

pub(super) fn monotonic_unix_epoch_millis() -> f64 {
    static BASE: OnceLock<(f64, Instant)> = OnceLock::new();
    let (epoch_millis, instant) = BASE.get_or_init(|| (unix_epoch_millis(), Instant::now()));
    epoch_millis + instant.elapsed().as_secs_f64() * 1000.0
}
