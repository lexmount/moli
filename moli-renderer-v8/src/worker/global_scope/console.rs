//! Worker console bindings and forwarding to captured native console methods.

use super::*;

const WORKER_ORIGINAL_CONSOLE_SLOT: &str = "__moliWorkerOriginalConsole";

#[derive(WebApiObject)]
#[webapi(plain)]
struct WorkerGlobalConsoleDeclaration<'scope> {
    #[webapi(data_property)]
    console: v8::Local<'scope, v8::Object>,
}

#[derive(Default, WebApiObject)]
#[webapi(plain)]
struct WorkerConsoleObjectDeclaration {
    #[webapi(method, enumerable, callback = console_log_callback, data = v8str(scope, "log"))]
    log: (),
    #[webapi(method, enumerable, callback = console_log_callback, data = v8str(scope, "info"))]
    info: (),
    #[webapi(method, enumerable, callback = console_log_callback, data = v8str(scope, "warn"))]
    warn: (),
    #[webapi(method, enumerable, callback = console_log_callback, data = v8str(scope, "error"))]
    error: (),
    #[webapi(method, enumerable, callback = console_log_callback, data = v8str(scope, "debug"))]
    debug: (),
    #[webapi(method, enumerable, callback = console_log_callback, data = v8str(scope, "trace"))]
    trace: (),
    #[webapi(method, enumerable, callback = console_label_callback, data = v8str(scope, "time"))]
    time: (),
    #[webapi(method, enumerable, callback = console_label_callback, data = v8str(scope, "timeLog"))]
    time_log: (),
    #[webapi(method, enumerable, callback = console_label_callback, data = v8str(scope, "timeEnd"))]
    time_end: (),
    #[webapi(method, enumerable, callback = console_label_callback, data = v8str(scope, "count"))]
    count: (),
    #[webapi(method, enumerable, callback = console_label_callback, data = v8str(scope, "countReset"))]
    count_reset: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    dir: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    dirxml: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    table: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    group: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    group_end: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    group_collapsed: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    clear: (),
    #[webapi(method, enumerable, callback = console_noop_callback)]
    assert: (),
    #[webapi(method, enumerable, callback = console_profile_callback)]
    profile: (),
    #[webapi(method, enumerable, callback = console_profile_end_callback)]
    profile_end: (),
}

pub(super) fn install_console<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let console_key = v8str(scope, "console");
    if let Some(original_console) = global.get(scope, console_key.into()) {
        set_private_value(
            scope,
            global,
            WORKER_ORIGINAL_CONSOLE_SLOT,
            original_console,
        );
    }
    let _ = global.delete(scope, console_key.into());

    let console = WorkerConsoleObjectDeclaration::default()
        .bind(scope)
        .map_err(|error| anyhow!("failed to create worker console: {error}"))?;
    crate::context_bootstrap::initialize_console_namespace(scope, console)?;

    WorkerGlobalConsoleDeclaration::new(console)
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize worker console: {error}"))
}

fn format_console_args(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> String {
    let mut parts = Vec::new();
    for i in 0..args.length() {
        let val = args.get(i);
        if let Some(s) = val.to_detail_string(scope) {
            parts.push(s.to_rust_string_lossy(scope));
        }
    }
    parts.join(" ")
}

fn console_arg_snapshots_json(
    scope: &mut v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> Vec<serde_json::Value> {
    let mut snapshots = Vec::with_capacity(args.length().max(0) as usize);
    for index in 0..args.length() {
        snapshots.push(crate::context_bootstrap::console_arg_remote_object_json(
            scope,
            args.get(index),
        ));
    }
    snapshots
}

fn console_log_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(method_name) = v8::Local::<v8::String>::try_from(args.data()) else {
        return;
    };
    let level = method_name.to_rust_string_lossy(scope);
    let text = format_console_args(scope, &args);
    let message = format!("{level}: {text}");
    let arg_snapshots = console_arg_snapshots_json(scope, &args);
    let stack = crate::context_bootstrap::current_console_stack(scope);
    tracing::trace!("{}", message);
    if let Some(state) = get_worker_state(scope) {
        let parent_tx = state.borrow().parent_tx.clone();
        let _ = parent_tx.send(WorkerToParentMessage::Console(WorkerConsoleMessage {
            message,
            args: arg_snapshots,
            stack,
        }));
    }
    call_original_worker_console_method(scope, &args, method_name);
}

fn console_noop_callback(
    _scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
}

fn console_label_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((method, arguments)) = crate::context_bootstrap::console_label_arguments(scope, &args)
    else {
        return;
    };
    crate::context_bootstrap::forward_native_console_arguments(
        scope,
        &arguments,
        method,
        WORKER_ORIGINAL_CONSOLE_SLOT,
    );
    rv.set_undefined();
}

fn console_profile_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let method = v8str(scope, "profile");
    call_original_worker_console_method(scope, &args, method);
    rv.set_undefined();
}

fn console_profile_end_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let method = v8str(scope, "profileEnd");
    call_original_worker_console_method(scope, &args, method);
    rv.set_undefined();
}

fn call_original_worker_console_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    method_name: v8::Local<'s, v8::String>,
) {
    let mut forwarded_args = Vec::with_capacity(args.length().max(0) as usize);
    for index in 0..args.length() {
        forwarded_args.push(args.get(index));
    }
    crate::context_bootstrap::forward_native_console_arguments(
        scope,
        &forwarded_args,
        method_name,
        WORKER_ORIGINAL_CONSOLE_SLOT,
    );
}
