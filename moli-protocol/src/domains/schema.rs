use moli_protocol_cdp_defs::{FORWARDED_COMMAND_DOMAINS, IMPLEMENTED_COMMANDS};
use serde_json::json;

use crate::conn::{CdpConnection, CdpSessionRoute, Cmd};
use crate::domains::actions::SchemaAction;
use crate::domains::command_output::CommandOutputPlan;

pub(crate) fn command_output_plan(conn: &CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    if !matches!(
        conn.session_route(cmd.session_id),
        Some(CdpSessionRoute::PageTarget { .. })
    ) || cmd.parse_action::<SchemaAction>() != Some(SchemaAction::GetDomains)
    {
        return CommandOutputPlan::error(-32601, format!("'{}' wasn't found", cmd.method));
    }

    let domains: Vec<_> = IMPLEMENTED_COMMANDS
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| !matches!(*name, "Browser" | "SystemInfo"))
        .chain(FORWARDED_COMMAND_DOMAINS.iter().copied())
        .map(|name| json!({ "name": name, "version": "1.2" }))
        .collect();
    CommandOutputPlan::result(json!({ "domains": domains }))
}
