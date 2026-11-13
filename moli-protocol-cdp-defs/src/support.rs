use strum::VariantNames;

use crate::actions::*;

pub type DomainMembers = (&'static str, &'static [&'static str]);

/// Methods parsed by the dispatcher's action enums.
pub const IMPLEMENTED_COMMANDS: &[DomainMembers] = &[
    ("Accessibility", AccessibilityAction::VARIANTS),
    ("Audits", AuditsAction::VARIANTS),
    ("Autofill", AutofillAction::VARIANTS),
    ("Browser", BrowserAction::VARIANTS),
    ("Console", ConsoleAction::VARIANTS),
    ("CSS", CssAction::VARIANTS),
    ("DOM", DomAction::VARIANTS),
    ("DOMDebugger", DomDebuggerAction::VARIANTS),
    ("DOMSnapshot", DomSnapshotAction::VARIANTS),
    ("DOMStorage", DomStorageAction::VARIANTS),
    ("Emulation", EmulationAction::VARIANTS),
    ("Fetch", FetchAction::VARIANTS),
    ("HeapProfiler", HeapProfilerAction::VARIANTS),
    ("Input", InputAction::VARIANTS),
    ("Inspector", InspectorAction::VARIANTS),
    ("IO", IoAction::VARIANTS),
    ("Log", LogAction::VARIANTS),
    ("Network", NetworkAction::VARIANTS),
    ("Page", PageAction::VARIANTS),
    ("Performance", PerformanceAction::VARIANTS),
    ("Profiler", ProfilerAction::VARIANTS),
    ("Runtime", RuntimeAction::VARIANTS),
    ("Security", SecurityAction::VARIANTS),
    ("ServiceWorker", ServiceWorkerAction::VARIANTS),
    ("Storage", StorageAction::VARIANTS),
    ("SystemInfo", SystemInfoAction::VARIANTS),
    ("Target", TargetAction::VARIANTS),
    ("Tracing", TracingAction::VARIANTS),
    ("WebAuthn", WebAuthnAction::VARIANTS),
    ("WebMCP", WebMcpAction::VARIANTS),
];

/// Commands handled by the server without an action enum.
pub const SERVER_COMMANDS: &[DomainMembers] = &[("Browser", &["close"])];

pub const FORWARDED_COMMAND_DOMAINS: &[&str] = &["Debugger"];
pub const FORWARDED_EVENT_DOMAINS: &[&str] =
    &["Console", "Debugger", "HeapProfiler", "Profiler", "Runtime"];

#[cfg(test)]
mod implemented_commands_tests {
    use super::IMPLEMENTED_COMMANDS;

    #[test]
    fn variant_names_are_wire_method_names() {
        let all = IMPLEMENTED_COMMANDS;
        let get = |domain: &str| {
            all.iter()
                .find(|(d, _)| *d == domain)
                .map(|(_, c)| *c)
                .unwrap()
        };
        // Explicit `serialize` overrides and camelCase defaults must both come out as the CDP wire names.
        assert!(get("Accessibility").contains(&"getFullAXTree"));
        assert!(get("DOMStorage").contains(&"getDOMStorageItems"));
        assert!(get("Page").contains(&"navigate"));
        assert!(get("Profiler").contains(&"startPreciseCoverage"));
        assert_eq!(all.len(), 30);
    }
}
