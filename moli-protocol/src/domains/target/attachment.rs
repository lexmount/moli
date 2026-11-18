use anyhow::Result;

use serde::Deserialize;
use serde_json::json;

use crate::automation::DevToolsTargetInfo;
use crate::conn::{BackgroundProtocolEvent, PreparedTargetAttach};

use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttachParams {
    target_id: String,
    flatten: Option<bool>,
}

fn target_command_error_without_session(
    code: i32,
    message: impl std::fmt::Display,
) -> TargetCommandTaskStep {
    TargetCommandTaskStep::Complete(CommandOutputPlan::error_without_session(code, message))
}

fn attach_session_events_then_result_plan(
    session_id: String,
    events: Vec<BackgroundProtocolEvent>,
) -> CommandOutputPlan {
    // Puppeteer creates and registers its CdpCDPSession while handling
    // Target.attachedToTarget. Once the Target.attachToTarget response resolves,
    // it immediately looks that session up by the returned sessionId and fails
    // with "CDPSession creation failed" if the event has not arrived yet.
    //
    // Chromium emits the synchronous attachedToTarget event before completing
    // the command response. Preserve that observable wire order here instead of
    // applying the usual response-before-background-events convention.
    let mut plan = CommandOutputPlan::default();
    for event in events {
        plan.push_background_event(event);
    }
    plan.push_result(json!({ "sessionId": session_id }));
    plan
}

fn attach_session_events_then_result_step(
    session_id: String,
    events: Vec<BackgroundProtocolEvent>,
) -> TargetCommandTaskStep {
    let plan = attach_session_events_then_result_plan(session_id, events);
    TargetCommandTaskStep::Complete(plan)
}

#[derive(Default)]
struct TargetCommandOutput {
    plan: CommandOutputPlan,
    side_effects: events::TargetProtocolSideEffects,
}

impl TargetCommandOutput {
    fn background_events_mut(&mut self) -> &mut Vec<BackgroundProtocolEvent> {
        self.side_effects.background_events_mut()
    }

    fn target_events_mut(&mut self) -> &mut events::TargetProtocolSideEffects {
        &mut self.side_effects
    }

    fn push_success(&mut self) {
        self.flush_side_effects();
        self.plan.push_success();
    }

    fn push_error(&mut self, code: i32, message: impl std::fmt::Display) {
        self.flush_side_effects();
        self.plan
            .push_error_without_session(code, format!("{message:#}"));
    }

    fn into_plan(mut self) -> CommandOutputPlan {
        self.flush_side_effects();
        self.plan
    }

    fn flush_side_effects(&mut self) {
        self.plan.extend(self.side_effects.drain_into_plan());
    }
}

pub(super) fn start_attach_to_target_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> TargetCommandTaskStep {
    let params: AttachParams = match cmd.get_params() {
        Ok(Some(p)) => p,
        _ => {
            return target_command_error_without_session(-32602, "InvalidParams");
        }
    };
    do_attach(
        conn,
        cmd,
        &params.target_id,
        params.flatten.unwrap_or(false),
    )
}

pub(super) fn attach_to_browser_target_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    if cmd.session_id.is_some() && !conn.is_browser_session_id(cmd.session_id) {
        return CommandOutputPlan::error(-32000, "Not allowed");
    }
    attach_browser_target(conn, cmd.session_id, true)
}

fn attach_browser_target(
    conn: &mut CdpConnection,
    owner_session_id: Option<&str>,
    flattened_protocol: bool,
) -> CommandOutputPlan {
    let session_id = conn.gen_session_id();
    let target_info = super::browser_context::devtools_browser_target_info();
    let mut plan = CommandOutputPlan::default();
    for event in conn
        .commit_browser_attached_session_event_plan(
            session_id.clone(),
            owner_session_id,
            super::browser_context::DEVTOOLS_BROWSER_TARGET_ID,
            target_info,
        )
        .with_flattened_protocol(flattened_protocol)
    {
        plan.push_background_event(event);
    }
    plan.push_result(json!({ "sessionId": session_id }));
    plan
}

fn do_attach(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    target_id: &str,
    flattened_protocol: bool,
) -> TargetCommandTaskStep {
    if target_id == super::browser_context::DEVTOOLS_BROWSER_TARGET_ID {
        // Chromium applies its browser access-mode check to the specialized
        // AttachToBrowserTarget action, not to generic attachment of a known host.
        return TargetCommandTaskStep::Complete(attach_browser_target(
            conn,
            cmd.session_id,
            flattened_protocol,
        ));
    }
    conn.ensure_default_target_live(target_id);
    let attach_from_browser_session = conn.is_browser_session_id(cmd.session_id);
    if conn
        .primary_page_target_id_for_tab_target_id(target_id)
        .is_some()
    {
        return do_attach_tab_target(
            conn,
            cmd.session_id,
            target_id,
            attach_from_browser_session,
            flattened_protocol,
        );
    }
    let restore_browser_context_id =
        attach_from_browser_session.then(|| previously_active_browser_context_id(conn));
    if let Err(message) = select_browser_context_for_target(conn, target_id) {
        if let Some(restore_browser_context_id) = restore_browser_context_id.as_ref() {
            restore_previously_active_browser_context(conn, restore_browser_context_id.as_deref());
        }
        return target_command_error_without_session(-31998, message);
    }
    let bc = match conn.browser_context.as_ref() {
        Some(bc) => bc,
        None => {
            if let Some(restore_browser_context_id) = restore_browser_context_id.as_ref() {
                restore_previously_active_browser_context(
                    conn,
                    restore_browser_context_id.as_deref(),
                );
            }
            return target_command_error_without_session(-31998, "BrowserContextNotLoaded");
        }
    };
    if bc.has_shared_worker_target(target_id) {
        return do_attach_shared_worker_target(conn, cmd.session_id, target_id, flattened_protocol);
    }
    if bc.has_dedicated_worker_target(target_id) {
        return do_attach_dedicated_worker_target(
            conn,
            cmd.session_id,
            target_id,
            flattened_protocol,
        );
    }
    if bc.has_service_worker_target(target_id) {
        return do_attach_service_worker_target(
            conn,
            cmd.session_id,
            target_id,
            flattened_protocol,
        );
    }
    if bc.active_target_id().is_none() && bc.has_no_background_targets() {
        if let Some(restore_browser_context_id) = restore_browser_context_id.as_ref() {
            restore_previously_active_browser_context(conn, restore_browser_context_id.as_deref());
        }
        return target_command_error_without_session(-31998, "TargetNotLoaded");
    }
    let active_target_identity = bc.active_target_identity();
    let target_has_primary_session = if matches!(
        active_target_identity,
        Some((ref active_target_id, _)) if active_target_id == target_id
    ) {
        active_target_identity
            .as_ref()
            .is_some_and(|(_, session_id)| session_id.is_some())
    } else if bc.background_target(target_id).is_some() {
        bc.background_target(target_id)
            .is_some_and(|target| target.has_session())
    } else {
        if let Some(restore_browser_context_id) = restore_browser_context_id.as_ref() {
            restore_previously_active_browser_context(conn, restore_browser_context_id.as_deref());
        }
        return target_command_error_without_session(-31998, "UnknownTargetId");
    };
    let browser_context_id = bc.id.clone();

    let session_id = conn.gen_session_id();
    let bc = conn.browser_context.as_mut().unwrap();
    let assigned = if attach_from_browser_session || target_has_primary_session {
        bc.assign_attached_session_to_target(target_id, session_id.clone())
    } else {
        bc.assign_session_to_target(target_id, session_id.clone())
    };
    if !assigned {
        if let Some(restore_browser_context_id) = restore_browser_context_id.as_ref() {
            restore_previously_active_browser_context(conn, restore_browser_context_id.as_deref());
        }
        return target_command_error_without_session(-31998, "UnknownTargetId");
    }
    let session_key = if attach_from_browser_session || target_has_primary_session {
        moli_page_types::DevToolsSessionKey::Attached(session_id.clone())
    } else {
        moli_page_types::DevToolsSessionKey::Primary
    };
    let prepared_session = crate::conn::TargetAttachSessionCommit::direct(
        session_id.clone(),
        cmd.session_id.map(str::to_owned),
        crate::conn::CdpSessionRoute::PageTarget {
            browser_context_id,
            target_id: target_id.to_owned(),
            session_key,
        },
        false,
    )
    .with_flattened_protocol(flattened_protocol);
    let owner = crate::conn::CommandOwnerScope::for_route(prepared_session.route().clone());

    let initial_document = match conn.start_initial_document_page_ensure_for_owner(&owner) {
        Ok(pending) => pending.map(Box::new),
        Err(message) => {
            if let Some(restore_browser_context_id) = restore_browser_context_id.as_ref() {
                restore_previously_active_browser_context(
                    conn,
                    restore_browser_context_id.as_deref(),
                );
            }
            return target_command_error_without_session(-32000, message);
        }
    };
    let bc = conn.browser_context.as_ref().unwrap();
    let target_info = bc
        .devtools_target_info(target_id)
        .expect("attached target must be addressable");
    if let Some(restore_browser_context_id) = restore_browser_context_id.as_ref() {
        restore_previously_active_browser_context(conn, restore_browser_context_id.as_deref());
    }
    TargetCommandTaskStep::Pending(PendingTargetCommandDispatch {
        command_id: cmd.id,
        session_id: cmd.session_id.map(str::to_owned),
        kind: Box::new(PendingTargetCommandKind::AttachToTarget {
            prepared_session,
            target_info,
            initial_document,
        }),
    })
}

fn do_attach_tab_target(
    conn: &mut CdpConnection,
    command_session_id: Option<&str>,
    tab_target_id: &str,
    attach_from_browser_session: bool,
    flattened_protocol: bool,
) -> TargetCommandTaskStep {
    let is_attached_session = attach_from_browser_session
        || conn
            .primary_session_id_for_tab_target_id(tab_target_id)
            .is_some();
    let session_id = conn.gen_session_id();
    let event_plan = match conn.attach_tab_target_session_event_plan(
        session_id.clone(),
        command_session_id,
        tab_target_id,
        is_attached_session,
    ) {
        Ok(event_plan) => event_plan,
        Err(message) => return target_command_error_without_session(-31998, message),
    };
    attach_session_events_then_result_step(
        session_id,
        event_plan
            .with_flattened_protocol(flattened_protocol)
            .into_background_events(),
    )
}

pub(crate) fn commit_target_attachment(
    conn: &mut CdpConnection,
    prepared: TargetAttachSessionCommit,
    target_info: DevToolsTargetInfo,
) -> Result<CommandOutputPlan, CommandOutputPlan> {
    use super::session_disposal::SessionDisposalError;
    let owner = CommandOwnerScope::for_route(prepared.route().clone());
    if conn.target_is_closing_for_owner(&owner)
        || prepared
            .owner_session_id()
            .is_some_and(|session| conn.session_is_closing(session))
    {
        return Err(CommandOutputPlan::error_without_session(
            -32000,
            SessionDisposalError::Closing,
        ));
    }
    let Some(target_id) = target_info
        .target_id
        .as_ref()
        .map(|id| id.as_str().to_owned())
    else {
        return Err(CommandOutputPlan::error_without_session(
            -32000,
            SessionDisposalError::UnknownTarget,
        ));
    };
    let binding_exists = prepared
        .route()
        .browser_context_id()
        .and_then(|context| conn.browser_context_by_id(context))
        .and_then(|context| context.page_target(&target_id))
        .is_some_and(|target| {
            target
                .devtools_sessions
                .key_for_wire_session_id(prepared.session_id())
                .is_some()
        });
    if !binding_exists
        || prepared
            .owner_session_id()
            .is_some_and(|session| conn.session_route(Some(session)).is_none())
    {
        return Err(CommandOutputPlan::error_without_session(
            -32000,
            SessionDisposalError::UnknownTarget,
        ));
    }
    if let Some(error) = super::transient_no_page_devtools_target_info_error(conn, &target_info) {
        return Err(CommandOutputPlan::error_without_session(-32000, error));
    }
    let session_id = prepared.session_id().to_owned();
    let events = conn.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
        target_id,
        target_info,
        [prepared],
    ));
    Ok(attach_session_events_then_result_plan(
        session_id,
        events.into_background_events(),
    ))
}

fn do_attach_shared_worker_target(
    conn: &mut CdpConnection,
    command_session_id: Option<&str>,
    target_id: &str,
    flattened_protocol: bool,
) -> TargetCommandTaskStep {
    let session_id = conn.gen_session_id();
    let event_plan = match conn.attach_shared_worker_target_session_event_plan(
        session_id.clone(),
        command_session_id,
        target_id,
    ) {
        Ok(event_plan) => event_plan,
        Err(message) => return target_command_error_without_session(-31998, message),
    };
    attach_session_events_then_result_step(
        session_id,
        event_plan
            .with_flattened_protocol(flattened_protocol)
            .into_background_events(),
    )
}

fn do_attach_service_worker_target(
    conn: &mut CdpConnection,
    command_session_id: Option<&str>,
    target_id: &str,
    flattened_protocol: bool,
) -> TargetCommandTaskStep {
    let session_id = conn.gen_session_id();
    let event_plan = match conn.attach_service_worker_target_session_event_plan(
        session_id.clone(),
        command_session_id,
        target_id,
    ) {
        Ok(event_plan) => event_plan,
        Err(message) => return target_command_error_without_session(-31998, message),
    };
    attach_session_events_then_result_step(
        session_id,
        event_plan
            .with_flattened_protocol(flattened_protocol)
            .into_background_events(),
    )
}

fn do_attach_dedicated_worker_target(
    conn: &mut CdpConnection,
    command_session_id: Option<&str>,
    target_id: &str,
    flattened_protocol: bool,
) -> TargetCommandTaskStep {
    let session_id = conn.gen_session_id();
    let event_plan = match conn.attach_dedicated_worker_target_session_event_plan(
        session_id.clone(),
        command_session_id,
        target_id,
    ) {
        Ok(event_plan) => event_plan,
        Err(message) => return target_command_error_without_session(-31998, message),
    };
    attach_session_events_then_result_step(
        session_id,
        event_plan
            .with_flattened_protocol(flattened_protocol)
            .into_background_events(),
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendMessageParams {
    message: String,
    session_id: Option<String>,
}

mod message;
pub(crate) use message::retain_target_message_output;
pub(super) use message::start_send_message_to_target_command;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DetachParams {
    target_id: Option<String>,
    session_id: Option<String>,
}

mod detachment;
pub(super) use detachment::TargetDetachment;
pub(crate) use detachment::TargetDetachmentOutput;
pub(crate) use detachment::{TargetDetachmentFailureCleanup, TargetDetachmentRecovery};

pub(super) fn start_detach_from_target_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> TargetCommandTaskStep {
    let params = match cmd.get_params::<DetachParams>() {
        Ok(Some(params)) => params,
        Ok(None) => DetachParams {
            target_id: None,
            session_id: None,
        },
        Err(error) => return super::target_command_error(-32602, error),
    };
    let previous_context = previously_active_browser_context_id(conn);
    let prepared = TargetDetachment::prepare(conn, cmd.session_id, params);
    restore_previously_active_browser_context(conn, previous_context.as_deref());
    match prepared {
        Ok(detachment) => TargetCommandTaskStep::Pending(PendingTargetCommandDispatch {
            command_id: cmd.id,
            session_id: cmd.session_id.map(str::to_owned),
            kind: Box::new(PendingTargetCommandKind::DetachFromTarget {
                detachment,
                access: conn.protocol_state_access(),
            }),
        }),
        Err(error) => super::target_command_error(-31998, format!("{error:#}")),
    }
}
