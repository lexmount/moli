use crate::conn::{CdpConnection, Cmd};
use crate::domains::actions::HeapProfilerAction;
use crate::domains::command_output::CommandOutputPlan;
use crate::domains::runtime::{
    RuntimeCommandTaskStep, start_heap_profiler_inspector_command_dispatch,
    start_moli_diagnostics_command_dispatch,
};

pub(crate) fn try_start_heap_profiler_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<RuntimeCommandTaskStep> {
    match cmd.parse_action::<HeapProfilerAction>() {
        Some(HeapProfilerAction::MoliAllocatorStats) => Some(RuntimeCommandTaskStep::Complete(
            CommandOutputPlan::result(conn.moli_allocator_stats_for_diagnostics()),
        )),
        Some(HeapProfilerAction::MoliDiagnostics) => {
            Some(start_moli_diagnostics_command_dispatch(conn, cmd))
        }
        Some(HeapProfilerAction::MoliResetIdleEngine) => Some(RuntimeCommandTaskStep::Complete(
            CommandOutputPlan::result(conn.moli_reset_idle_navigation_engine_for_diagnostics()),
        )),
        Some(action) => Some(start_heap_profiler_inspector_command_dispatch(
            conn, cmd, action,
        )),
        None => Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32601,
            "UnknownMethod",
        ))),
    }
}
