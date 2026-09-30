//! Target-local configuration has a Renderer terminal and Browser replay
//! policy. Only a genuinely unpublished, retired attachment can fall back to
//! the Browser acknowledgement of that policy; a published reply always wins.
use super::*;
use crate::domains::native::{self, NativeCommandStep};
use moli_core::{
    RendererNativeOperation as Operation, RendererNativeOperationStep as Step,
    RendererNativeProtocolResponse as Response, RendererPageCommand as Command,
    RendererPageReply as Reply,
};

pub(super) fn try_start(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    operation: Operation,
    policy: PendingEmulationPageOperation,
) -> Option<EmulationCommandTaskStep> {
    let attachment = native::frontend_attachment(conn, cmd)?;
    Some(start_admitted(conn, cmd, operation, policy, attachment))
}

pub(super) fn start_admitted(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    operation: Operation,
    policy: PendingEmulationPageOperation,
    attachment: moli_page_types::RendererAgentAttachmentId,
) -> EmulationCommandTaskStep {
    let owner = policy
        .has_authoritative_replay_state()
        .then(|| CommandOwnerScope::capture(conn, cmd.session_id));
    match native::start_operation(conn, cmd, operation) {
        NativeCommandStep::Complete(plan) => EmulationCommandTaskStep::Complete(plan),
        NativeCommandStep::Pending(pending) => {
            // Transient Page state (idle override) uses the ordinary failure
            // path; only replayable Browser policy needs an attachment fallback.
            let pending = if let Some(owner) = owner {
                pending.on_unpublished_failure(move |conn, error| {
                    let target = PendingEmulationPageTarget::SessionOwner { owner_scope: owner };
                    if pending_emulation_page_configuration_will_be_replayed(
                        conn,
                        &target,
                        &policy,
                        Some(attachment),
                    ) {
                        CommandOutputPlan::success()
                    } else {
                        CommandOutputPlan::error(-32000, error)
                    }
                })
            } else {
                pending
            };
            EmulationCommandTaskStep::Native(Box::new(pending))
        }
    }
}

pub(super) fn try_start_surface(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<EmulationCommandTaskStep> {
    let attachment = native::frontend_attachment(conn, cmd)?;
    let inputs =
        conn.navigation_load_inputs_for_owner(&CommandOwnerScope::capture(conn, cmd.session_id));
    let operation = Operation::then(
        Command::SetNavigatorOverrides(inputs.navigator_overrides),
        move |reply| match reply {
            Ok(Reply::Unit) => {
                Step::Continue(unit(Command::SetDocumentActivity(inputs.document_activity)))
            }
            Err(error) => Step::Complete(Response::error(-32000, error.to_string())),
            _ => unreachable!("navigator configuration acknowledgement"),
        },
    );
    Some(start_admitted(
        conn,
        cmd,
        operation,
        PendingEmulationPageOperation::SetDocumentActivity,
        attachment,
    ))
}

pub(super) fn unit(command: Command) -> Operation {
    Operation::new(command, |reply| match reply {
        Ok(Reply::Unit) => Response::success(json!({})),
        Err(error) => Response::error(-32000, error.to_string()),
        _ => unreachable!("configuration acknowledgement"),
    })
}
