use serde_json::json;
use strum::IntoEnumIterator;

use crate::conn::Cmd;
use crate::domains::command_output::CommandOutputPlan;

/// Domains installed by the connection dispatcher. Keep discovery and dispatch exhaustive.
#[derive(Clone, Copy, strum::EnumString, strum::EnumIter, strum::AsRefStr)]
pub(crate) enum CdpDomain {
    Browser,
    Runtime,
    HeapProfiler,
    Profiler,
    Debugger,
    Accessibility,
    Input,
    #[strum(serialize = "CSS")]
    Css,
    #[strum(serialize = "DOM")]
    Dom,
    #[strum(serialize = "DOMStorage")]
    DomStorage,
    Console,
    Network,
    Target,
    Tracing,
    Fetch,
    Page,
    Inspector,
    Log,
    Storage,
    #[strum(serialize = "DOMSnapshot")]
    DomSnapshot,
    Overlay,
    Security,
    ServiceWorker,
    #[strum(serialize = "IO")]
    Io,
    Autofill,
    Audits,
    SystemInfo,
    WebAuthn,
    WebMCP,
    Performance,
    #[strum(serialize = "DOMDebugger")]
    DomDebugger,
    Emulation,
    Schema,
}

pub(crate) fn command_output_plan(cmd: &Cmd<'_>) -> CommandOutputPlan {
    if cmd.method != "Schema.getDomains" {
        return CommandOutputPlan::error(-32601, "UnknownMethod");
    }
    let domains: Vec<_> = CdpDomain::iter()
        .map(|domain| json!({"name": domain.as_ref(), "version": crate::version::PROTOCOL_VERSION}))
        .collect();
    CommandOutputPlan::result(json!({"domains": domains}))
}
