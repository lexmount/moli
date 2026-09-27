use super::*;
#[cfg(test)]
use crate::local_executor::JsOwnerLocalRuntimeAccessPath;
#[cfg(test)]
use crate::local_executor::JsOwnerLocalRuntimeEntryPath;
use tokio::sync::oneshot;

pub(super) async fn run_named_owner_local_task<R, F>(
    local_executor: JsLocalExecutor,
    canceled_message: &'static str,
    future: F,
) -> Result<R>
where
    R: 'static,
    F: std::future::Future<Output = Result<R>> + 'static,
{
    // Page task futures carry script preparation state. Keep that state on the
    // heap before wrapping it in executor/TLS and spawn futures, so nested owner
    // turns do not repeatedly reserve large copies on the caller's stack.
    let future = Box::pin(future);
    let (reply_tx, reply_rx) = oneshot::channel();
    let timing_enabled = moli_trace::cdp_nav_timing_enabled();
    let started = timing_enabled.then(std::time::Instant::now);
    if timing_enabled {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            task = canceled_message,
            stage = "owner_local_task_spawned",
        );
    }
    tokio::task::spawn_local(async move {
        let result = local_executor.scope_on_current_thread(future).await;
        let _ = reply_tx.send(result);
    });
    let result = reply_rx.await.map_err(|_| anyhow!(canceled_message))?;
    if let Some(started) = started {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            task = canceled_message,
            elapsed_ms = started.elapsed().as_millis(),
            stage = "owner_local_task_finished",
        );
    }
    result
}

#[cfg(test)]
pub(super) type OwnerLocalRuntimeAccessPath = JsOwnerLocalRuntimeAccessPath;
#[cfg(test)]
pub(super) type OwnerLocalRuntimeEntryPath = JsOwnerLocalRuntimeEntryPath;

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScriptExecutionDomainPath {
    DirectNamedLane,
    DirectScaffoldLane,
    CurrentThreadFallback,
    Inaccessible,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScriptExecutionLanePath {
    DirectNamedLane,
    DirectScaffoldLane,
    Inaccessible,
}

#[cfg(test)]
pub(super) fn owner_local_runtime_access_path(
    local_executor: &JsLocalExecutor,
) -> OwnerLocalRuntimeAccessPath {
    local_executor.current_owner_local_runtime_access_path()
}

#[cfg(test)]
pub(super) fn owner_local_runtime_entry_path(
    local_executor: &JsLocalExecutor,
) -> OwnerLocalRuntimeEntryPath {
    local_executor.current_owner_local_runtime_entry_path()
}

#[cfg(test)]
pub(super) fn script_execution_domain_path(
    local_executor: &JsLocalExecutor,
) -> ScriptExecutionDomainPath {
    match local_executor.current_access_context() {
        crate::local_executor::JsLocalExecutorAccessContext::MatchingNamedLane => {
            ScriptExecutionDomainPath::DirectNamedLane
        }
        crate::local_executor::JsLocalExecutorAccessContext::DifferentNamedLane => {
            ScriptExecutionDomainPath::Inaccessible
        }
        crate::local_executor::JsLocalExecutorAccessContext::ScaffoldLane => {
            ScriptExecutionDomainPath::DirectScaffoldLane
        }
        crate::local_executor::JsLocalExecutorAccessContext::CurrentThreadOutsideLane => {
            ScriptExecutionDomainPath::CurrentThreadFallback
        }
        crate::local_executor::JsLocalExecutorAccessContext::Outside => {
            ScriptExecutionDomainPath::Inaccessible
        }
    }
}

#[cfg(test)]
pub(super) fn script_execution_lane_path(
    local_executor: &JsLocalExecutor,
) -> ScriptExecutionLanePath {
    match script_execution_domain_path(local_executor) {
        ScriptExecutionDomainPath::DirectNamedLane => ScriptExecutionLanePath::DirectNamedLane,
        ScriptExecutionDomainPath::DirectScaffoldLane => {
            ScriptExecutionLanePath::DirectScaffoldLane
        }
        ScriptExecutionDomainPath::CurrentThreadFallback
        | ScriptExecutionDomainPath::Inaccessible => ScriptExecutionLanePath::Inaccessible,
    }
}

#[cfg(test)]
pub(super) fn is_on_script_execution_domain_for(local_executor: &JsLocalExecutor) -> bool {
    !matches!(
        script_execution_domain_path(local_executor),
        ScriptExecutionDomainPath::Inaccessible
    )
}

pub(super) fn is_on_script_execution_lane_for(local_executor: &JsLocalExecutor) -> bool {
    #[cfg(test)]
    {
        crate::local_executor::is_on_script_execution_lane_for(local_executor)
    }

    #[cfg(not(test))]
    {
        is_on_named_owner_execution_lane_for(local_executor)
    }
}

#[cfg(test)]
pub(super) fn is_on_parse_time_scaffold_lane() -> bool {
    crate::local_executor::is_on_scoped_js_local_executor_lane()
}
