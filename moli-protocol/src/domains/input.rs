use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, Cmd, CommandDispatchContext,
    TargetPageResidenceIdentity, TargetPageResidenceObservation, TargetPageResidenceToken,
};
use crate::devtools_runtime::{
    DevToolsCommand, DevToolsCommandContext, DevToolsCommandResult,
    DevToolsDispatchDragEventCommand, DevToolsDispatchKeyEventCommand,
    DevToolsDispatchMouseEventCommand, DevToolsDispatchTouchEventCommand, DevToolsDragData,
    DevToolsDragDataItem, DevToolsDragEventType, DevToolsError, DevToolsErrorKind,
    DevToolsMouseEventType, DevToolsPointerType, DevToolsSynthesizeTapGestureCommand,
    DevToolsTouchEventType, DevToolsTouchPoint,
};
use crate::domains::command_output::CommandOutputPlan;
use moli_core::page::{
    CompletedPageCommand, Page, PageInputExt, PendingPageCommand, RendererCommandTurnCompletion,
    RendererDragData, RendererDragDataItem, RendererDraggedFile, RendererInputDispatchOutcome,
    RendererPendingDownloadActivation, RendererPendingFileChooserActivation,
    RendererPointerEventProperties, RendererTouchPoint, decode_input_dispatch_outcome_completion,
    decode_insert_text_completion,
};
use serde::Deserialize;
#[cfg(test)]
use serde_json::json;

use crate::domains::actions::InputAction;
use crate::domains::activity::{ProtocolOutputPayloads, ProtocolOutputSink};
use chromiumoxide_cdp::cdp::browser_protocol::input::{
    DispatchDragEventParams as CdpDispatchDragEventParams,
    DispatchDragEventType as CdpDispatchDragEventType,
    DispatchMouseEventParams as CdpDispatchMouseEventParams,
    DispatchMouseEventPointerType as CdpDispatchMouseEventPointerType,
    DispatchMouseEventType as CdpDispatchMouseEventType,
    DispatchTouchEventParams as CdpDispatchTouchEventParams,
    DispatchTouchEventType as CdpDispatchTouchEventType,
    EmulateTouchFromMouseEventParams as CdpEmulateTouchFromMouseEventParams,
    EmulateTouchFromMouseEventType as CdpEmulateTouchFromMouseEventType,
    MouseButton as CdpMouseButton, SynthesizeTapGestureParams as CdpSynthesizeTapGestureParams,
};

mod drag;
mod file_chooser;
mod key;
mod mouse;
#[cfg(test)]
mod tests;

pub(crate) const DISPATCH_MOUSE_EVENT_UNSUPPORTED_MESSAGE: &str = "Input.dispatchMouseEvent is not supported: coordinate-based mouse input requires layout hit testing; use DOM.resolveNode and Runtime.callFunctionOn for element activation instead.";
pub(crate) const DISPATCH_TOUCH_EVENT_UNSUPPORTED_MESSAGE: &str = "Input.dispatchTouchEvent is not supported: coordinate-based touch input requires layout hit testing.";
pub(crate) const EMULATE_TOUCH_FROM_MOUSE_EVENT_UNSUPPORTED_MESSAGE: &str = "Input.emulateTouchFromMouseEvent is not supported: coordinate-based touch emulation requires layout hit testing.";
pub(crate) const SYNTHESIZE_TAP_GESTURE_UNSUPPORTED_MESSAGE: &str = "Input.synthesizeTapGesture is not supported: coordinate-based tap gestures require layout hit testing.";
pub(crate) const DISPATCH_DRAG_EVENT_UNSUPPORTED_MESSAGE: &str = "Input.dispatchDragEvent is not supported: coordinate-based drag input requires layout hit testing.";
pub(crate) const SET_INTERCEPT_DRAGS_UNSUPPORTED_MESSAGE: &str =
    "Input.setInterceptDrags is not supported: drag interception events are not implemented.";

#[derive(Deserialize)]
struct SetIgnoreInputEventsParams {
    ignore: bool,
}

pub(crate) struct PendingInputCommandDispatch {
    command_id: Option<u64>,
    session_id: Option<String>,
    owner: TargetPageResidenceIdentity,
    page_residence_token: Option<TargetPageResidenceToken>,
    kind: PendingInputCommandKind,
    pending: PendingInputOperation,
}

pub(crate) struct CompletedInputCommandDispatch {
    command_id: Option<u64>,
    session_id: Option<String>,
    owner: TargetPageResidenceIdentity,
    kind: PendingInputCommandKind,
    completed: CompletedInputOperation,
}

pub(crate) enum InputCommandTaskStep {
    Complete,
}

pub(crate) enum InputCommandDispatchStep {
    Pending(PendingInputCommandDispatch),
    Complete(CommandOutputPlan),
}

struct CompletedInputCommandResult {
    result: Result<DevToolsCommandResult, DevToolsError>,
    protocol_events: Vec<BackgroundProtocolEvent>,
}

#[derive(Default)]
struct InputCommandSideEffects {
    protocol_events: Vec<BackgroundProtocolEvent>,
}

impl InputCommandSideEffects {
    fn events_mut(&mut self) -> &mut Vec<BackgroundProtocolEvent> {
        &mut self.protocol_events
    }

    fn extend_protocol_events(
        &mut self,
        command_context: &mut CommandDispatchContext,
        events: Vec<BackgroundProtocolEvent>,
    ) {
        self.protocol_events
            .extend(command_context.take_protocol_events_before_events(events));
        self.protocol_events
            .extend(command_context.take_post_response_events());
    }

    fn into_events(self) -> Vec<BackgroundProtocolEvent> {
        self.protocol_events
    }
}

struct PendingInputCommandStartError {
    code: i32,
    message: String,
    kind: DevToolsErrorKind,
}

#[derive(Clone)]
enum PendingInputCommandKind {
    DispatchMouseEvent,
    DispatchKeyEvent,
    DispatchTouchEvent,
    DispatchDragEvent,
    SynthesizeTapGesture,
    InsertText,
}

enum PendingInputOperation {
    Page(PendingPageCommand),
    #[cfg(test)]
    RendererAckHeldForTest,
}

enum CompletedInputOperation {
    Page(Box<Result<CompletedPageCommand, String>>),
    PageResidenceSuperseded,
    PageResidenceUnavailable,
}

enum RendererInputWaitOutcome<T> {
    Completed(T),
    PageResidence(TargetPageResidenceObservation),
}

async fn wait_for_renderer_input_or_page_replacement<T>(
    completion: impl std::future::Future<Output = T>,
    page_residence_token: Option<TargetPageResidenceToken>,
) -> RendererInputWaitOutcome<T> {
    let Some(page_residence_token) = page_residence_token else {
        return RendererInputWaitOutcome::Completed(completion.await);
    };
    tokio::pin!(completion);
    tokio::select! {
        biased;
        result = &mut completion => RendererInputWaitOutcome::Completed(result),
        observation = page_residence_token.wait() => {
            RendererInputWaitOutcome::PageResidence(observation)
        }
    }
}

impl PendingInputCommandDispatch {
    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub(crate) async fn wait(self) -> CompletedInputCommandDispatch {
        let completed = match self.pending {
            PendingInputOperation::Page(pending) => {
                match wait_for_renderer_input_or_page_replacement(
                    pending.wait(),
                    self.page_residence_token,
                )
                .await
                {
                    RendererInputWaitOutcome::Completed(result) => CompletedInputOperation::Page(
                        Box::new(result.map_err(|error| error.to_string())),
                    ),
                    RendererInputWaitOutcome::PageResidence(
                        TargetPageResidenceObservation::Superseded,
                    ) => CompletedInputOperation::PageResidenceSuperseded,
                    RendererInputWaitOutcome::PageResidence(
                        TargetPageResidenceObservation::Unavailable,
                    ) => CompletedInputOperation::PageResidenceUnavailable,
                }
            }
            #[cfg(test)]
            PendingInputOperation::RendererAckHeldForTest => {
                match wait_for_renderer_input_or_page_replacement(
                    std::future::pending::<std::convert::Infallible>(),
                    self.page_residence_token,
                )
                .await
                {
                    RendererInputWaitOutcome::Completed(never) => match never {},
                    RendererInputWaitOutcome::PageResidence(
                        TargetPageResidenceObservation::Superseded,
                    ) => CompletedInputOperation::PageResidenceSuperseded,
                    RendererInputWaitOutcome::PageResidence(
                        TargetPageResidenceObservation::Unavailable,
                    ) => CompletedInputOperation::PageResidenceUnavailable,
                }
            }
        };
        CompletedInputCommandDispatch {
            command_id: self.command_id,
            session_id: self.session_id,
            owner: self.owner,
            kind: self.kind,
            completed,
        }
    }

    #[cfg(test)]
    pub(crate) fn hold_renderer_ack_for_test(&mut self) -> bool {
        // The command has already crossed the real JSON/domain admission path
        // and been enqueued in the renderer. Replacing only its reply receiver
        // gives lifecycle smoke tests a deterministic outstanding callback;
        // public CDP intentionally cannot pause that callback while also
        // scheduling a replacement of the same Page owner.
        if !self.kind.uses_renderer_host_ack_cleanup() || self.page_residence_token.is_none() {
            return false;
        }
        self.pending = PendingInputOperation::RendererAckHeldForTest;
        true
    }
}

impl CompletedInputCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
}

impl PendingInputCommandKind {
    fn uses_renderer_host_ack_cleanup(&self) -> bool {
        // Chromium's InputInjector owns replacement cleanup for the mouse and
        // key callback queues. Touch, drag, and IME commands have separate
        // completion protocols and must keep waiting for those terminals.
        matches!(self, Self::DispatchMouseEvent | Self::DispatchKeyEvent)
    }
}

impl PendingInputCommandStartError {
    fn invalid_params() -> Self {
        Self {
            code: -32602,
            message: "InvalidParams".to_owned(),
            kind: DevToolsErrorKind::InvalidArgument,
        }
    }

    fn no_document_loaded() -> Self {
        Self {
            code: -32000,
            message: "NoDocumentLoaded".to_owned(),
            kind: DevToolsErrorKind::Internal,
        }
    }

    fn unsupported(message: &'static str) -> Self {
        Self {
            code: -32000,
            message: message.to_owned(),
            kind: DevToolsErrorKind::Unsupported,
        }
    }

    fn renderer_error(error: impl std::fmt::Display) -> Self {
        Self {
            code: -32000,
            message: error.to_string(),
            kind: DevToolsErrorKind::Internal,
        }
    }
}

pub(crate) fn try_start_input_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> InputCommandDispatchStep {
    let Some(action) = cmd.parse_action::<InputAction>() else {
        return InputCommandDispatchStep::Complete(CommandOutputPlan::error(
            -32601,
            "UnknownMethod",
        ));
    };
    match action {
        InputAction::CancelDragging => {
            InputCommandDispatchStep::Complete(drag::cancel_dragging_command_output_plan(conn, cmd))
        }
        InputAction::SetInterceptDrags => InputCommandDispatchStep::Complete(
            drag::set_intercept_drags_command_output_plan(conn, cmd),
        ),
        InputAction::SetIgnoreInputEvents => InputCommandDispatchStep::Complete(
            set_ignore_input_events_command_output_plan(conn, cmd),
        ),
        InputAction::DispatchMouseEvent
        | InputAction::DispatchTouchEvent
        | InputAction::EmulateTouchFromMouseEvent
        | InputAction::SynthesizeTapGesture
        | InputAction::DispatchDragEvent => match validate_coordinate_input_params(cmd, action) {
            Ok(()) if conn.layout_policy() == moli_core::LayoutPolicy::Mock => {
                InputCommandDispatchStep::Complete(CommandOutputPlan::error(
                    -32000,
                    coordinate_input_unsupported_message(action),
                ))
            }
            Ok(()) => start_input_command_dispatch_after_noop_checks(conn, cmd, action),
            Err(error) => InputCommandDispatchStep::Complete(CommandOutputPlan::error(
                error.code,
                error.message,
            )),
        },
        InputAction::DispatchKeyEvent | InputAction::InsertText => {
            start_input_command_dispatch_after_noop_checks(conn, cmd, action)
        }
    }
}

fn set_ignore_input_events_command_output_plan(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let params: SetIgnoreInputEventsParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => return CommandOutputPlan::error(-32602, "InvalidParams"),
    };
    let updated = conn
        .with_target_devtools_session_state_for_session_mut(cmd.session_id, |state| {
            state.page_session_state.input_events_ignored = params.ignore;
        })
        .is_some();
    if !updated {
        return CommandOutputPlan::error(-32000, "NoBrowserContext");
    }
    CommandOutputPlan::success()
}

fn input_events_ignored_for_session_owner(conn: &CdpConnection, session_id: Option<&str>) -> bool {
    conn.page_event_session_ids_for_session_owner(session_id)
        .into_iter()
        .any(|event_session_id| {
            conn.target_page_session_state_for_session(event_session_id.as_deref())
                .is_some_and(|state| state.input_events_ignored)
        })
}

fn required_enum_param_matches(cmd: &Cmd<'_>, name: &str, allowed: &[&str]) -> bool {
    cmd.params
        .and_then(|params| params.get(name))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| allowed.contains(&value))
}

fn optional_enum_param_matches(cmd: &Cmd<'_>, name: &str, allowed: &[&str]) -> bool {
    match cmd.params.and_then(|params| params.get(name)) {
        None => true,
        Some(serde_json::Value::String(value)) => allowed.contains(&value.as_str()),
        Some(_) => false,
    }
}

fn validate_coordinate_input_params(
    cmd: &Cmd<'_>,
    action: InputAction,
) -> Result<(), PendingInputCommandStartError> {
    match action {
        InputAction::DispatchMouseEvent => mouse::validate_dispatch_mouse_event(cmd)
            .map_err(|_| PendingInputCommandStartError::invalid_params())
            .and_then(|()| {
                let params = cmd
                    .get_params::<CdpDispatchMouseEventParams>()
                    .map_err(|_| PendingInputCommandStartError::invalid_params())?
                    .ok_or_else(PendingInputCommandStartError::invalid_params)?;
                let finite = [
                    Some(params.x),
                    Some(params.y),
                    params.delta_x,
                    params.delta_y,
                    params.force,
                    params.tangential_pressure,
                    params.tilt_x,
                    params.tilt_y,
                ]
                .into_iter()
                .flatten()
                .all(f64::is_finite);
                (finite && cdp_modifiers(params.modifiers).is_ok())
                    .then_some(())
                    .ok_or_else(PendingInputCommandStartError::invalid_params)
            }),
        InputAction::DispatchTouchEvent => {
            if !required_enum_param_matches(
                cmd,
                "type",
                &["touchStart", "touchEnd", "touchMove", "touchCancel"],
            ) {
                return Err(PendingInputCommandStartError::invalid_params());
            }
            let params = cmd
                .get_params::<CdpDispatchTouchEventParams>()
                .map_err(|_| PendingInputCommandStartError::invalid_params())?
                .ok_or_else(PendingInputCommandStartError::invalid_params)?;
            if cdp_modifiers(params.modifiers).is_err() {
                return Err(PendingInputCommandStartError::invalid_params());
            }
            let valid_point_count =
                match params.r#type {
                    CdpDispatchTouchEventType::TouchStart
                    | CdpDispatchTouchEventType::TouchMove => !params.touch_points.is_empty(),
                    CdpDispatchTouchEventType::TouchEnd
                    | CdpDispatchTouchEventType::TouchCancel => params.touch_points.is_empty(),
                };
            let point_ids_are_consistent = params.touch_points.is_empty()
                || params.touch_points.iter().all(|point| point.id.is_some())
                || params.touch_points.iter().all(|point| point.id.is_none());
            let pointer_properties_are_valid = params.touch_points.iter().all(|point| {
                mouse::validate_pointer_properties(
                    point.force.unwrap_or(1.0),
                    point.tangential_pressure.unwrap_or(0.0),
                    point.tilt_x.unwrap_or(0.0),
                    point.tilt_y.unwrap_or(0.0),
                    point.twist.unwrap_or(0),
                )
                .is_ok()
            });
            let coordinates_and_ids_are_valid = params.touch_points.iter().all(|point| {
                point.x.is_finite()
                    && point.y.is_finite()
                    && point.id.is_none_or(|id| {
                        id.is_finite()
                            && id.fract() == 0.0
                            && id >= 0.0
                            && id <= f64::from(i32::MAX)
                    })
            });
            let mut ids = std::collections::HashSet::new();
            let ids_are_unique = params
                .touch_points
                .iter()
                .filter_map(|point| point.id.map(|id| id as i32))
                .all(|id| ids.insert(id));
            (valid_point_count
                && point_ids_are_consistent
                && pointer_properties_are_valid
                && coordinates_and_ids_are_valid
                && ids_are_unique)
                .then_some(())
                .ok_or_else(PendingInputCommandStartError::invalid_params)
        }
        InputAction::EmulateTouchFromMouseEvent => {
            if !required_enum_param_matches(
                cmd,
                "type",
                &["mousePressed", "mouseReleased", "mouseMoved", "mouseWheel"],
            ) || !required_enum_param_matches(cmd, "button", &["none", "left", "right"])
            {
                return Err(PendingInputCommandStartError::invalid_params());
            }
            let params = cmd
                .get_params::<CdpEmulateTouchFromMouseEventParams>()
                .map_err(|_| PendingInputCommandStartError::invalid_params())?
                .ok_or_else(PendingInputCommandStartError::invalid_params)?;
            if [Some(params.x), Some(params.y), params.click_count]
                .into_iter()
                .flatten()
                .any(|value| i32::try_from(value).is_err())
            {
                return Err(PendingInputCommandStartError::invalid_params());
            }
            if cdp_modifiers(params.modifiers).is_err()
                || !params.delta_x.is_none_or(f64::is_finite)
                || !params.delta_y.is_none_or(f64::is_finite)
            {
                return Err(PendingInputCommandStartError::invalid_params());
            }
            let valid_wheel_delta = params.r#type != CdpEmulateTouchFromMouseEventType::MouseWheel
                || (params.delta_x.is_some() && params.delta_y.is_some());
            valid_wheel_delta
                .then_some(())
                .ok_or_else(PendingInputCommandStartError::invalid_params)
        }
        InputAction::SynthesizeTapGesture => {
            if !optional_enum_param_matches(
                cmd,
                "gestureSourceType",
                &["default", "touch", "mouse"],
            ) {
                return Err(PendingInputCommandStartError::invalid_params());
            }
            let params = cmd
                .get_params::<CdpSynthesizeTapGestureParams>()
                .map_err(|_| PendingInputCommandStartError::invalid_params())?
                .ok_or_else(PendingInputCommandStartError::invalid_params)?;
            (params.x.is_finite()
                && params.y.is_finite()
                && params.duration.is_none_or(|duration| duration >= 0)
                && params.tap_count.unwrap_or(1) == 1)
                .then_some(())
                .ok_or_else(PendingInputCommandStartError::invalid_params)
        }
        InputAction::DispatchDragEvent => {
            if !required_enum_param_matches(
                cmd,
                "type",
                &["dragEnter", "dragOver", "drop", "dragCancel"],
            ) {
                return Err(PendingInputCommandStartError::invalid_params());
            }
            let params = cmd
                .get_params::<CdpDispatchDragEventParams>()
                .map_err(|_| PendingInputCommandStartError::invalid_params())?
                .ok_or_else(PendingInputCommandStartError::invalid_params)?;
            (params.x.is_finite()
                && params.y.is_finite()
                && cdp_modifiers(params.modifiers).is_ok()
                && i32::try_from(params.data.drag_operations_mask).is_ok())
            .then_some(())
            .ok_or_else(PendingInputCommandStartError::invalid_params)
        }
        InputAction::CancelDragging
        | InputAction::DispatchKeyEvent
        | InputAction::InsertText
        | InputAction::SetIgnoreInputEvents
        | InputAction::SetInterceptDrags => Ok(()),
    }
}

fn coordinate_input_unsupported_message(action: InputAction) -> &'static str {
    match action {
        InputAction::DispatchMouseEvent => DISPATCH_MOUSE_EVENT_UNSUPPORTED_MESSAGE,
        InputAction::DispatchTouchEvent => DISPATCH_TOUCH_EVENT_UNSUPPORTED_MESSAGE,
        InputAction::EmulateTouchFromMouseEvent => {
            EMULATE_TOUCH_FROM_MOUSE_EVENT_UNSUPPORTED_MESSAGE
        }
        InputAction::SynthesizeTapGesture => SYNTHESIZE_TAP_GESTURE_UNSUPPORTED_MESSAGE,
        InputAction::DispatchDragEvent => DISPATCH_DRAG_EVENT_UNSUPPORTED_MESSAGE,
        InputAction::CancelDragging
        | InputAction::DispatchKeyEvent
        | InputAction::InsertText
        | InputAction::SetIgnoreInputEvents
        | InputAction::SetInterceptDrags => {
            unreachable!("non-coordinate input action has no unsupported message")
        }
    }
}

fn start_input_command_dispatch_after_noop_checks(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: InputAction,
) -> InputCommandDispatchStep {
    match start_pending_input_command(conn, cmd, action) {
        Ok(Some(pending)) => InputCommandDispatchStep::Pending(pending),
        Ok(None) => InputCommandDispatchStep::Complete(CommandOutputPlan::success()),
        Err(error) => {
            InputCommandDispatchStep::Complete(CommandOutputPlan::error(error.code, error.message))
        }
    }
}

fn start_pending_input_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: InputAction,
) -> Result<Option<PendingInputCommandDispatch>, PendingInputCommandStartError> {
    if action.requires_document_access()
        && let Err(message) = conn.ensure_document_accessible_for_session_owner(cmd.session_id)
    {
        return Err(PendingInputCommandStartError {
            code: -32000,
            message,
            kind: DevToolsErrorKind::Internal,
        });
    }
    match action {
        InputAction::DispatchKeyEvent => {
            let parsed = key::parse_dispatch_key_event(cmd)
                .map_err(|_| PendingInputCommandStartError::invalid_params())?;
            if input_events_ignored_for_session_owner(conn, cmd.session_id) {
                return Ok(None);
            }
            let command = build_cdp_dispatch_key_event_command(conn, cmd, parsed);
            start_devtools_input_command(
                conn,
                cmd.id,
                cmd.session_id,
                DevToolsCommand::DispatchKeyEvent(command),
            )
        }
        InputAction::DispatchMouseEvent
        | InputAction::DispatchTouchEvent
        | InputAction::EmulateTouchFromMouseEvent
        | InputAction::SynthesizeTapGesture
        | InputAction::DispatchDragEvent => {
            if conn.layout_policy() == moli_core::LayoutPolicy::Mock {
                return Err(PendingInputCommandStartError::unsupported(
                    coordinate_input_unsupported_message(action),
                ));
            }
            if input_events_ignored_for_session_owner(conn, cmd.session_id) {
                return Ok(None);
            }
            let command = build_cdp_coordinate_input_command(conn, cmd, action)?;
            start_devtools_input_command(conn, cmd.id, cmd.session_id, command)
        }
        InputAction::InsertText => {
            let text = key::parse_insert_text(cmd)
                .map_err(|_| PendingInputCommandStartError::invalid_params())?;
            start_page_input_command(
                conn,
                cmd.id,
                cmd.session_id,
                PendingInputCommandKind::InsertText,
                |page| page.start_insert_text_into_active_control(&text),
            )
        }
        InputAction::CancelDragging
        | InputAction::SetIgnoreInputEvents
        | InputAction::SetInterceptDrags => Ok(None),
    }
}

fn build_cdp_dispatch_key_event_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    parsed: key::ParsedDispatchKeyEvent,
) -> DevToolsDispatchKeyEventCommand {
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    DevToolsDispatchKeyEventCommand {
        context: cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref()),
        event_type: parsed.event_type,
        key: parsed.key,
        code: parsed.code,
        text: parsed.text,
        modifiers: parsed.modifiers,
        auto_repeat: parsed.auto_repeat,
        should_insert_text: parsed.should_insert_text,
    }
}

fn cdp_input_command_context(conn: &CdpConnection, cmd: &Cmd<'_>) -> DevToolsCommandContext {
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .map(|(browser_context_id, target_id)| (Some(browser_context_id), target_id))
        .unwrap_or((None, None));
    cmd.devtools_command_context(target_id.as_deref(), browser_context_id.as_deref())
}

fn cdp_modifiers(value: Option<i64>) -> Result<u8, PendingInputCommandStartError> {
    value
        .unwrap_or_default()
        .try_into()
        .map_err(|_| PendingInputCommandStartError::invalid_params())
}

fn cdp_mouse_button(button: Option<CdpMouseButton>) -> i32 {
    match button.unwrap_or(CdpMouseButton::None) {
        CdpMouseButton::None => -1,
        CdpMouseButton::Left => 0,
        CdpMouseButton::Middle => 1,
        CdpMouseButton::Right => 2,
        CdpMouseButton::Back => 3,
        CdpMouseButton::Forward => 4,
    }
}

fn build_cdp_coordinate_input_command(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    action: InputAction,
) -> Result<DevToolsCommand, PendingInputCommandStartError> {
    let context = cdp_input_command_context(conn, cmd);
    match action {
        InputAction::DispatchMouseEvent => {
            let params = cmd
                .get_params::<CdpDispatchMouseEventParams>()
                .map_err(|_| PendingInputCommandStartError::invalid_params())?
                .ok_or_else(PendingInputCommandStartError::invalid_params)?;
            if conn.emit_touch_events_for_mouse_for_session_owner(cmd.session_id)
                && params.r#type != CdpDispatchMouseEventType::MouseWheel
            {
                let event_type = match params.r#type {
                    CdpDispatchMouseEventType::MousePressed => DevToolsTouchEventType::Start,
                    CdpDispatchMouseEventType::MouseReleased => DevToolsTouchEventType::End,
                    CdpDispatchMouseEventType::MouseMoved => DevToolsTouchEventType::Move,
                    CdpDispatchMouseEventType::MouseWheel => unreachable!(),
                };
                let touch_points = if event_type == DevToolsTouchEventType::End {
                    Vec::new()
                } else {
                    vec![DevToolsTouchPoint {
                        id: 0,
                        x: params.x,
                        y: params.y,
                    }]
                };
                return Ok(DevToolsCommand::DispatchTouchEvent(
                    DevToolsDispatchTouchEventCommand {
                        context,
                        event_type,
                        touch_points,
                    },
                ));
            }
            Ok(DevToolsCommand::DispatchMouseEvent(
                DevToolsDispatchMouseEventCommand {
                    context,
                    event_type: match params.r#type {
                        CdpDispatchMouseEventType::MousePressed => DevToolsMouseEventType::Pressed,
                        CdpDispatchMouseEventType::MouseReleased => {
                            DevToolsMouseEventType::Released
                        }
                        CdpDispatchMouseEventType::MouseMoved => DevToolsMouseEventType::Moved,
                        CdpDispatchMouseEventType::MouseWheel => DevToolsMouseEventType::Wheel,
                    },
                    pointer_type: match params
                        .pointer_type
                        .unwrap_or(CdpDispatchMouseEventPointerType::Mouse)
                    {
                        CdpDispatchMouseEventPointerType::Mouse => DevToolsPointerType::Mouse,
                        CdpDispatchMouseEventPointerType::Pen => DevToolsPointerType::Pen,
                    },
                    x: params.x,
                    y: params.y,
                    button: cdp_mouse_button(params.button),
                    buttons: params.buttons.map(|value| value as i32),
                    click_count: params.click_count.unwrap_or_default() as i32,
                    delta_x: params.delta_x.unwrap_or_default(),
                    delta_y: params.delta_y.unwrap_or_default(),
                    force: params.force.unwrap_or_default(),
                    tangential_pressure: params.tangential_pressure.unwrap_or_default(),
                    tilt_x: params.tilt_x.unwrap_or_default(),
                    tilt_y: params.tilt_y.unwrap_or_default(),
                    twist: params.twist.unwrap_or_default() as f64,
                    modifiers: cdp_modifiers(params.modifiers)?,
                },
            ))
        }
        InputAction::DispatchTouchEvent => {
            let params = cmd
                .get_params::<CdpDispatchTouchEventParams>()
                .map_err(|_| PendingInputCommandStartError::invalid_params())?
                .ok_or_else(PendingInputCommandStartError::invalid_params)?;
            let touch_points = params
                .touch_points
                .into_iter()
                .enumerate()
                .map(|(index, point)| DevToolsTouchPoint {
                    id: point.id.unwrap_or(index as f64) as i32,
                    x: point.x,
                    y: point.y,
                })
                .collect();
            Ok(DevToolsCommand::DispatchTouchEvent(
                DevToolsDispatchTouchEventCommand {
                    context,
                    event_type: match params.r#type {
                        CdpDispatchTouchEventType::TouchStart => DevToolsTouchEventType::Start,
                        CdpDispatchTouchEventType::TouchMove => DevToolsTouchEventType::Move,
                        CdpDispatchTouchEventType::TouchEnd => DevToolsTouchEventType::End,
                        CdpDispatchTouchEventType::TouchCancel => DevToolsTouchEventType::Cancel,
                    },
                    touch_points,
                },
            ))
        }
        InputAction::EmulateTouchFromMouseEvent => {
            let params = cmd
                .get_params::<CdpEmulateTouchFromMouseEventParams>()
                .map_err(|_| PendingInputCommandStartError::invalid_params())?
                .ok_or_else(PendingInputCommandStartError::invalid_params)?;
            if params.r#type == CdpEmulateTouchFromMouseEventType::MouseWheel {
                return Ok(DevToolsCommand::DispatchMouseEvent(
                    DevToolsDispatchMouseEventCommand {
                        context,
                        event_type: DevToolsMouseEventType::Wheel,
                        pointer_type: DevToolsPointerType::Mouse,
                        x: params.x as f64,
                        y: params.y as f64,
                        button: cdp_mouse_button(Some(params.button)),
                        buttons: None,
                        click_count: params.click_count.unwrap_or_default() as i32,
                        delta_x: params.delta_x.unwrap_or_default(),
                        delta_y: params.delta_y.unwrap_or_default(),
                        force: 0.0,
                        tangential_pressure: 0.0,
                        tilt_x: 0.0,
                        tilt_y: 0.0,
                        twist: 0.0,
                        modifiers: cdp_modifiers(params.modifiers)?,
                    },
                ));
            }
            let event_type = match params.r#type {
                CdpEmulateTouchFromMouseEventType::MousePressed => DevToolsTouchEventType::Start,
                CdpEmulateTouchFromMouseEventType::MouseReleased => DevToolsTouchEventType::End,
                CdpEmulateTouchFromMouseEventType::MouseMoved => DevToolsTouchEventType::Move,
                CdpEmulateTouchFromMouseEventType::MouseWheel => unreachable!(),
            };
            let touch_points = if event_type == DevToolsTouchEventType::End {
                Vec::new()
            } else {
                vec![DevToolsTouchPoint {
                    id: 0,
                    x: params.x as f64,
                    y: params.y as f64,
                }]
            };
            Ok(DevToolsCommand::DispatchTouchEvent(
                DevToolsDispatchTouchEventCommand {
                    context,
                    event_type,
                    touch_points,
                },
            ))
        }
        InputAction::SynthesizeTapGesture => {
            let params = cmd
                .get_params::<CdpSynthesizeTapGestureParams>()
                .map_err(|_| PendingInputCommandStartError::invalid_params())?
                .ok_or_else(PendingInputCommandStartError::invalid_params)?;
            Ok(DevToolsCommand::SynthesizeTapGesture(
                DevToolsSynthesizeTapGestureCommand {
                    context,
                    x: params.x,
                    y: params.y,
                },
            ))
        }
        InputAction::DispatchDragEvent => {
            let params = cmd
                .get_params::<CdpDispatchDragEventParams>()
                .map_err(|_| PendingInputCommandStartError::invalid_params())?
                .ok_or_else(PendingInputCommandStartError::invalid_params)?;
            Ok(DevToolsCommand::DispatchDragEvent(
                DevToolsDispatchDragEventCommand {
                    context,
                    event_type: match params.r#type {
                        CdpDispatchDragEventType::DragEnter => DevToolsDragEventType::Enter,
                        CdpDispatchDragEventType::DragOver => DevToolsDragEventType::Over,
                        CdpDispatchDragEventType::Drop => DevToolsDragEventType::Drop,
                        CdpDispatchDragEventType::DragCancel => DevToolsDragEventType::Cancel,
                    },
                    x: params.x,
                    y: params.y,
                    data: DevToolsDragData {
                        items: params
                            .data
                            .items
                            .into_iter()
                            .map(|item| DevToolsDragDataItem {
                                mime_type: item.mime_type,
                                data: item.data,
                                title: item.title,
                                base_url: item.base_url,
                            })
                            .collect(),
                        files: params.data.files.unwrap_or_default(),
                        drag_operations_mask: params.data.drag_operations_mask as i32,
                    },
                    modifiers: cdp_modifiers(params.modifiers)?,
                },
            ))
        }
        InputAction::CancelDragging
        | InputAction::DispatchKeyEvent
        | InputAction::InsertText
        | InputAction::SetIgnoreInputEvents
        | InputAction::SetInterceptDrags => Err(PendingInputCommandStartError::invalid_params()),
    }
}

pub(crate) async fn execute_devtools_input_command_async_with_protocol_events(
    conn: &mut CdpConnection,
    command: DevToolsCommand,
) -> (
    Result<DevToolsCommandResult, DevToolsError>,
    Vec<crate::conn::BackgroundProtocolEvent>,
) {
    let (target_id, command_session_id) = match devtools_input_command_route(&command) {
        Ok(route) => route,
        Err(error) => return (Err(error), Vec::new()),
    };
    if let Some(target_id) = target_id.as_deref() {
        let route = conn
            .target_session_route_for_target_id(target_id)
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"));
        let route = match route {
            Ok(route) => route,
            Err(error) => return (Err(error), Vec::new()),
        };
        let mut route_scope = conn.scoped_none_session_owner_route_override(route);
        return execute_devtools_input_command_on_current_route_with_protocol_events(
            route_scope.conn_mut(),
            None,
            command,
        )
        .await;
    }

    execute_devtools_input_command_on_current_route_with_protocol_events(
        conn,
        command_session_id.as_deref(),
        command,
    )
    .await
}

fn devtools_input_command_route(
    command: &DevToolsCommand,
) -> Result<(Option<String>, Option<String>), DevToolsError> {
    let context = match command {
        DevToolsCommand::DispatchMouseEvent(command) => &command.context,
        DevToolsCommand::DispatchKeyEvent(command) => &command.context,
        DevToolsCommand::DispatchTouchEvent(command) => &command.context,
        DevToolsCommand::DispatchDragEvent(command) => &command.context,
        DevToolsCommand::SynthesizeTapGesture(command) => &command.context,
        _ => {
            return Err(DevToolsError::new(
                DevToolsErrorKind::Unsupported,
                "UnsupportedDevToolsCommand",
            ));
        }
    };
    Ok((
        context
            .target_id
            .as_ref()
            .map(|target_id| target_id.to_string()),
        context
            .session_id
            .as_ref()
            .map(|session_id| session_id.to_string()),
    ))
}

async fn execute_devtools_input_command_on_current_route_with_protocol_events(
    conn: &mut CdpConnection,
    command_session_id: Option<&str>,
    command: DevToolsCommand,
) -> (
    Result<DevToolsCommandResult, DevToolsError>,
    Vec<crate::conn::BackgroundProtocolEvent>,
) {
    let pending = match start_devtools_input_command(conn, None, command_session_id, command) {
        Ok(Some(pending)) => pending,
        Ok(None) => return (Ok(DevToolsCommandResult::Empty), Vec::new()),
        Err(error) => {
            return (
                Err(devtools_error_from_input_start_error(error)),
                Vec::new(),
            );
        }
    };

    let completed = complete_pending_input_command(
        conn,
        pending.wait().await,
        &mut CommandDispatchContext::default(),
    )
    .await;
    (completed.result, completed.protocol_events)
}

fn devtools_error_from_input_start_error(error: PendingInputCommandStartError) -> DevToolsError {
    DevToolsError::new(error.kind, error.message)
}

fn start_devtools_dispatch_key_event_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: DevToolsDispatchKeyEventCommand,
) -> Result<Option<PendingInputCommandDispatch>, PendingInputCommandStartError> {
    start_page_input_command(
        conn,
        command_id,
        command_session_id,
        PendingInputCommandKind::DispatchKeyEvent,
        |page| {
            page.start_dispatch_key_event_with_outcome(
                key::devtools_key_event_dom_event_name(command.event_type),
                &command.key,
                &command.code,
                &command.text,
                command.modifiers,
                command.auto_repeat,
                command.should_insert_text,
            )
        },
    )
}

fn start_page_input_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    kind: PendingInputCommandKind,
    start: impl FnOnce(&Page) -> anyhow::Result<PendingPageCommand>,
) -> Result<Option<PendingInputCommandDispatch>, PendingInputCommandStartError> {
    let owner = conn
        .target_page_residence_identity_for_session(command_session_id)
        .ok_or_else(PendingInputCommandStartError::no_document_loaded)?;
    let page_residence_token = if kind.uses_renderer_host_ack_cleanup() {
        Some(
            conn.capture_target_page_residence_token_for_session(command_session_id)
                .ok_or_else(PendingInputCommandStartError::no_document_loaded)?,
        )
    } else {
        None
    };
    let page = loaded_page_mut(conn, command_session_id)
        .ok_or_else(PendingInputCommandStartError::no_document_loaded)?;
    let pending = start(page).map_err(PendingInputCommandStartError::renderer_error)?;
    Ok(Some(PendingInputCommandDispatch {
        command_id,
        session_id: command_session_id.map(str::to_owned),
        owner,
        page_residence_token,
        kind,
        pending: PendingInputOperation::Page(pending),
    }))
}

fn mouse_event_name(event_type: DevToolsMouseEventType) -> &'static str {
    match event_type {
        DevToolsMouseEventType::Pressed => "mousedown",
        DevToolsMouseEventType::Released => "mouseup",
        DevToolsMouseEventType::Moved => "mousemove",
        DevToolsMouseEventType::Wheel => "wheel",
    }
}

fn touch_event_name(event_type: DevToolsTouchEventType) -> &'static str {
    match event_type {
        DevToolsTouchEventType::Start => "touchstart",
        DevToolsTouchEventType::Move => "touchmove",
        DevToolsTouchEventType::End => "touchend",
        DevToolsTouchEventType::Cancel => "touchcancel",
    }
}

fn drag_event_name(event_type: DevToolsDragEventType) -> &'static str {
    match event_type {
        DevToolsDragEventType::Enter => "dragenter",
        DevToolsDragEventType::Over => "dragover",
        DevToolsDragEventType::Drop => "drop",
        DevToolsDragEventType::Cancel => "dragleave",
    }
}

fn start_devtools_dispatch_mouse_event_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: DevToolsDispatchMouseEventCommand,
) -> Result<Option<PendingInputCommandDispatch>, PendingInputCommandStartError> {
    let event_name = mouse_event_name(command.event_type);
    let pointer = RendererPointerEventProperties {
        pointer_id: 1,
        pointer_type: match command.pointer_type {
            DevToolsPointerType::Mouse => "mouse",
            DevToolsPointerType::Pen => "pen",
        }
        .to_owned(),
        pressure: command.force,
        tangential_pressure: command.tangential_pressure,
        tilt_x: command.tilt_x,
        tilt_y: command.tilt_y,
        twist: command.twist,
    };
    start_page_input_command(
        conn,
        command_id,
        command_session_id,
        PendingInputCommandKind::DispatchMouseEvent,
        |page| {
            page.start_dispatch_mouse_event_at_point_with_pointer_outcome(
                command.x,
                command.y,
                event_name,
                command.button,
                command.buttons,
                command.click_count,
                command.delta_x,
                command.delta_y,
                pointer,
                command.modifiers,
            )
        },
    )
}

fn start_devtools_dispatch_touch_event_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: DevToolsDispatchTouchEventCommand,
) -> Result<Option<PendingInputCommandDispatch>, PendingInputCommandStartError> {
    let event_name = touch_event_name(command.event_type);
    let points = command
        .touch_points
        .into_iter()
        .map(|point| RendererTouchPoint {
            id: point.id,
            x: point.x,
            y: point.y,
        })
        .collect::<Vec<_>>();
    start_page_input_command(
        conn,
        command_id,
        command_session_id,
        PendingInputCommandKind::DispatchTouchEvent,
        |page| page.start_dispatch_touch_event_at_points_with_outcome(points, event_name, false),
    )
}

fn renderer_drag_data(
    data: DevToolsDragData,
) -> Result<RendererDragData, PendingInputCommandStartError> {
    let files = data
        .files
        .iter()
        .map(|path| {
            let path = std::path::Path::new(path);
            let metadata = std::fs::metadata(path).map_err(|error| {
                PendingInputCommandStartError::renderer_error(format_args!(
                    "could not read Input.dispatchDragEvent file: {error}"
                ))
            })?;
            if !metadata.is_file() {
                return Err(PendingInputCommandStartError::renderer_error(
                    "Input.dispatchDragEvent file is not a regular file",
                ));
            }
            let bytes = std::fs::read(path).map_err(|error| {
                PendingInputCommandStartError::renderer_error(format_args!(
                    "could not read Input.dispatchDragEvent file: {error}"
                ))
            })?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_owned();
            let last_modified = metadata
                .modified()
                .ok()
                .and_then(|modified| {
                    modified
                        .duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .map(|duration| duration.as_millis() as f64)
                })
                .unwrap_or_default();
            Ok(RendererDraggedFile {
                bytes,
                mime_type: String::new(),
                name,
                last_modified,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RendererDragData {
        items: data
            .items
            .into_iter()
            .map(|item| RendererDragDataItem {
                mime_type: item.mime_type,
                data: item.data,
                title: item.title,
                base_url: item.base_url,
            })
            .collect(),
        files,
        directories: Vec::new(),
        drag_operations_mask: data.drag_operations_mask,
    })
}

fn start_devtools_dispatch_drag_event_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: DevToolsDispatchDragEventCommand,
) -> Result<Option<PendingInputCommandDispatch>, PendingInputCommandStartError> {
    let event_name = drag_event_name(command.event_type);
    let data = renderer_drag_data(command.data)?;
    start_page_input_command(
        conn,
        command_id,
        command_session_id,
        PendingInputCommandKind::DispatchDragEvent,
        |page| {
            page.start_dispatch_drag_event_at_point_with_outcome(
                command.x,
                command.y,
                event_name,
                data,
                command.modifiers,
            )
        },
    )
}

fn start_devtools_synthesize_tap_gesture_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: DevToolsSynthesizeTapGestureCommand,
) -> Result<Option<PendingInputCommandDispatch>, PendingInputCommandStartError> {
    start_page_input_command(
        conn,
        command_id,
        command_session_id,
        PendingInputCommandKind::SynthesizeTapGesture,
        |page| {
            page.start_dispatch_touch_event_at_point_with_outcome(
                command.x, command.y, "touchend", true,
            )
        },
    )
}

fn start_devtools_input_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: DevToolsCommand,
) -> Result<Option<PendingInputCommandDispatch>, PendingInputCommandStartError> {
    let coordinate_unsupported = match &command {
        DevToolsCommand::DispatchMouseEvent(_) => Some(DISPATCH_MOUSE_EVENT_UNSUPPORTED_MESSAGE),
        DevToolsCommand::DispatchTouchEvent(_) => Some(DISPATCH_TOUCH_EVENT_UNSUPPORTED_MESSAGE),
        DevToolsCommand::DispatchDragEvent(_) => Some(DISPATCH_DRAG_EVENT_UNSUPPORTED_MESSAGE),
        DevToolsCommand::SynthesizeTapGesture(_) => {
            Some(SYNTHESIZE_TAP_GESTURE_UNSUPPORTED_MESSAGE)
        }
        _ => None,
    };
    if let Some(message) = coordinate_unsupported {
        if conn.layout_policy() == moli_core::LayoutPolicy::Mock {
            return Err(PendingInputCommandStartError::unsupported(message));
        }
        if input_events_ignored_for_session_owner(conn, command_session_id) {
            return Ok(None);
        }
    }
    match command {
        DevToolsCommand::DispatchMouseEvent(command) => {
            start_devtools_dispatch_mouse_event_command(
                conn,
                command_id,
                command_session_id,
                command,
            )
        }
        DevToolsCommand::DispatchTouchEvent(command) => {
            start_devtools_dispatch_touch_event_command(
                conn,
                command_id,
                command_session_id,
                command,
            )
        }
        DevToolsCommand::DispatchDragEvent(command) => start_devtools_dispatch_drag_event_command(
            conn,
            command_id,
            command_session_id,
            command,
        ),
        DevToolsCommand::SynthesizeTapGesture(command) => {
            start_devtools_synthesize_tap_gesture_command(
                conn,
                command_id,
                command_session_id,
                command,
            )
        }
        DevToolsCommand::DispatchKeyEvent(command) => {
            start_devtools_dispatch_key_event_command(conn, command_id, command_session_id, command)
        }
        _ => Err(PendingInputCommandStartError {
            code: -32000,
            message: "UnsupportedDevToolsCommand".to_owned(),
            kind: DevToolsErrorKind::Unsupported,
        }),
    }
}

async fn complete_pending_input_command(
    conn: &mut CdpConnection,
    completed: CompletedInputCommandDispatch,
    command_context: &mut CommandDispatchContext,
) -> CompletedInputCommandResult {
    let session_id = completed.session_id.as_deref();
    let owner = completed.owner;
    let completed_operation = match completed.completed {
        CompletedInputOperation::Page(completed) => completed,
        CompletedInputOperation::PageResidenceSuperseded => {
            // Match InputInjector::Cleanup(): replacement of the widget/Page
            // retires an outstanding mouse/key ACK as protocol success even
            // when the old renderer never publishes its normal completion.
            return CompletedInputCommandResult {
                result: Ok(DevToolsCommandResult::Empty),
                protocol_events: Vec::new(),
            };
        }
        CompletedInputOperation::PageResidenceUnavailable => {
            return CompletedInputCommandResult {
                result: Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    "input command Page residence became unavailable",
                )),
                protocol_events: Vec::new(),
            };
        }
    };
    let mut side_effects = InputCommandSideEffects::default();

    let result = match completed.kind {
        kind @ (PendingInputCommandKind::DispatchMouseEvent
        | PendingInputCommandKind::DispatchTouchEvent
        | PendingInputCommandKind::DispatchDragEvent
        | PendingInputCommandKind::SynthesizeTapGesture) => {
            let result = match completed_page_command_result(completed_operation) {
                Ok(result) => result,
                Err(error) => {
                    let protocol_events = side_effects.into_events();
                    return CompletedInputCommandResult {
                        result: Err(error),
                        protocol_events,
                    };
                }
            };
            let completion = settle_completed_input_page_command(
                conn,
                session_id,
                &owner,
                result,
                command_context,
            );
            let operation = match kind {
                PendingInputCommandKind::DispatchMouseEvent => "mouse event page command",
                PendingInputCommandKind::DispatchTouchEvent
                | PendingInputCommandKind::SynthesizeTapGesture => "touch event page command",
                PendingInputCommandKind::DispatchDragEvent => "drag event page command",
                PendingInputCommandKind::DispatchKeyEvent | PendingInputCommandKind::InsertText => {
                    unreachable!()
                }
            };
            let outcome = decode_input_dispatch_outcome_completion(completion, operation);
            match outcome {
                Ok(outcome) => {
                    if let Err(error) = handle_input_dispatch_outcome_async(
                        conn,
                        &mut side_effects,
                        session_id,
                        &owner,
                        outcome,
                        command_context,
                    )
                    .await
                    {
                        let protocol_events = side_effects.into_events();
                        return CompletedInputCommandResult {
                            result: Err(DevToolsError::new(DevToolsErrorKind::Internal, error)),
                            protocol_events,
                        };
                    }
                    Ok(DevToolsCommandResult::Empty)
                }
                Err(error) => Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    error.to_string(),
                )),
            }
        }
        PendingInputCommandKind::DispatchKeyEvent => {
            let result = match completed_page_command_result(completed_operation) {
                Ok(result) => result,
                Err(error) => {
                    let protocol_events = side_effects.into_events();
                    return CompletedInputCommandResult {
                        result: Err(error),
                        protocol_events,
                    };
                }
            };
            let completion = settle_completed_input_page_command(
                conn,
                session_id,
                &owner,
                result,
                command_context,
            );
            let outcome =
                decode_input_dispatch_outcome_completion(completion, "key event page command");
            match outcome {
                Ok(outcome) => {
                    if let Err(error) = handle_input_dispatch_outcome_async(
                        conn,
                        &mut side_effects,
                        session_id,
                        &owner,
                        outcome,
                        command_context,
                    )
                    .await
                    {
                        let protocol_events = side_effects.into_events();
                        return CompletedInputCommandResult {
                            result: Err(DevToolsError::new(DevToolsErrorKind::Internal, error)),
                            protocol_events,
                        };
                    }
                    Ok(DevToolsCommandResult::Empty)
                }
                Err(error) => Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    error.to_string(),
                )),
            }
        }
        PendingInputCommandKind::InsertText => {
            let result = match completed_page_command_result(completed_operation) {
                Ok(result) => result,
                Err(error) => {
                    let protocol_events = side_effects.into_events();
                    return CompletedInputCommandResult {
                        result: Err(error),
                        protocol_events,
                    };
                }
            };
            let completion = settle_completed_input_page_command(
                conn,
                session_id,
                &owner,
                result,
                command_context,
            );
            let result = decode_insert_text_completion(completion);
            match result {
                Ok(_) => Ok(DevToolsCommandResult::Empty),
                Err(error) => Err(DevToolsError::new(
                    DevToolsErrorKind::Internal,
                    error.to_string(),
                )),
            }
        }
    };

    let protocol_events = side_effects.into_events();
    CompletedInputCommandResult {
        result,
        protocol_events,
    }
}

fn settle_completed_input_page_command(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    owner: &TargetPageResidenceIdentity,
    completion: CompletedPageCommand,
    command_context: &mut CommandDispatchContext,
) -> RendererCommandTurnCompletion {
    let output = conn.settle_page_command_turn_for_owner(session_id, owner, completion);
    command_context.consume_renderer_command_turn_output(output)
}

pub(crate) async fn complete_pending_input_command_output_plan(
    conn: &mut CdpConnection,
    completed: CompletedInputCommandDispatch,
    command_context: &mut CommandDispatchContext,
) -> (InputCommandTaskStep, CommandOutputPlan) {
    let completed = complete_pending_input_command(conn, completed, command_context).await;
    let mut plan = match completed.result {
        Ok(result) => CommandOutputPlan::from_devtools_result(result),
        Err(error) => CommandOutputPlan::from_devtools_error(error),
    };
    for event in completed.protocol_events {
        plan.push_background_event(event);
    }
    (InputCommandTaskStep::Complete, plan)
}

fn completed_page_command_result(
    completed: Box<Result<CompletedPageCommand, String>>,
) -> Result<CompletedPageCommand, DevToolsError> {
    match *completed {
        Ok(completion) => Ok(completion),
        Err(error) => Err(DevToolsError::new(DevToolsErrorKind::Internal, error)),
    }
}

pub(super) fn loaded_page_mut<'a>(
    conn: &'a mut CdpConnection,
    session_id: Option<&str>,
) -> Option<&'a mut Page> {
    conn.loaded_page_mut_for_protocol_access(session_id).ok()
}

pub(super) fn browser_context_mut_for_session_owner<'a>(
    conn: &'a mut CdpConnection,
    session_id: Option<&str>,
) -> Option<&'a mut crate::conn::BrowserContext> {
    let browser_context_id = conn
        .session_route(session_id)
        .and_then(|route| route.browser_context_id().map(str::to_owned));
    if let Some(browser_context_id) = browser_context_id {
        return conn.browser_context_by_id_mut(&browser_context_id);
    }
    conn.browser_context.as_mut()
}

#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct InputPreparedOutputs {
    download_activations: Vec<RendererPendingDownloadActivation>,
    file_chooser_activations: Vec<file_chooser::PreparedFileChooserActivation>,
}

#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct InputPreparedOutputSlot {
    outputs: InputPreparedOutputs,
}

impl InputPreparedOutputs {
    pub(crate) fn from_renderer_download_activation(
        activation: RendererPendingDownloadActivation,
    ) -> Self {
        Self {
            download_activations: vec![activation],
            file_chooser_activations: Vec::new(),
        }
    }

    pub(crate) fn from_renderer_file_chooser_activation(
        conn: &CdpConnection,
        session_id: Option<&str>,
        activation: RendererPendingFileChooserActivation,
    ) -> Self {
        let Some(page_owner) = conn.target_page_residence_identity_for_session(session_id) else {
            return Self::default();
        };
        let Some(activation) = file_chooser::PreparedFileChooserActivation::capture(
            conn, session_id, page_owner, activation,
        ) else {
            return Self::default();
        };
        Self {
            download_activations: Vec::new(),
            file_chooser_activations: vec![activation],
        }
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.download_activations.extend(other.download_activations);
        self.file_chooser_activations
            .extend(other.file_chooser_activations);
    }

    pub(in crate::domains) fn append_to_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        let has_download = !self.download_activations.is_empty();
        let has_file_chooser = !self.file_chooser_activations.is_empty();
        if has_file_chooser {
            sink.push_produced_slot(crate::domains::page::SLOT_FILE_CHOOSER);
        }
        if has_download {
            sink.push_produced_slot(crate::domains::page::SLOT_DOWNLOAD);
        }
        if has_download || has_file_chooser {
            sink.push_prepared_payload(InputPreparedOutputSlot::from_outputs(self).into());
        }
    }

    #[cfg(test)]
    pub(crate) fn from_download_activations_for_test(
        activations: Vec<RendererPendingDownloadActivation>,
    ) -> Self {
        Self {
            download_activations: activations,
            file_chooser_activations: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_file_chooser_activations_for_test(
        owner: TargetPageResidenceIdentity,
        root_frame_id: &str,
        activations: Vec<RendererPendingFileChooserActivation>,
    ) -> Self {
        Self {
            download_activations: Vec::new(),
            file_chooser_activations: activations
                .into_iter()
                .map(|activation| {
                    file_chooser::PreparedFileChooserActivation::from_renderer_for_test(
                        owner.clone(),
                        root_frame_id,
                        activation,
                    )
                })
                .collect(),
        }
    }
}

impl InputPreparedOutputSlot {
    pub(crate) fn from_outputs(outputs: InputPreparedOutputs) -> Self {
        Self { outputs }
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.outputs.extend(other.outputs);
    }

    pub(crate) fn take_download_activations(
        &mut self,
    ) -> Option<Vec<RendererPendingDownloadActivation>> {
        (!self.outputs.download_activations.is_empty())
            .then(|| std::mem::take(&mut self.outputs.download_activations))
    }

    fn take_file_chooser_activations(
        &mut self,
    ) -> Option<Vec<file_chooser::PreparedFileChooserActivation>> {
        (!self.outputs.file_chooser_activations.is_empty())
            .then(|| std::mem::take(&mut self.outputs.file_chooser_activations))
    }
}

async fn handle_input_dispatch_outcome_async(
    conn: &mut CdpConnection,
    out: &mut InputCommandSideEffects,
    session_id: Option<&str>,
    owner: &TargetPageResidenceIdentity,
    outcome: RendererInputDispatchOutcome,
    command_context: &mut CommandDispatchContext,
) -> Result<(), String> {
    if let Some(download) = outcome.pending_download {
        let mut events = Vec::new();
        conn.handle_pending_download_activation_background_events_async(
            &mut events,
            session_id,
            download,
            command_context,
        )
        .await?;
        out.extend_protocol_events(command_context, events);
    }
    if let Some(file_chooser) = outcome.pending_file_chooser
        && let Some(file_chooser) = file_chooser::PreparedFileChooserActivation::capture(
            conn,
            session_id,
            owner.clone(),
            file_chooser,
        )
    {
        file_chooser::emit_prepared_activation_async(
            conn,
            out.events_mut(),
            session_id,
            file_chooser,
        )
        .await;
    }
    // A navigation triggered by input is frozen by the renderer command turn
    // into a concrete owner-action record. Protocol ingress schedules that
    // action independently; this command path must not rescan mutable Page
    // state or execute a second navigation.
    Ok(())
}

pub(in crate::domains) async fn emit_download_activity_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
    command_context: &mut CommandDispatchContext,
) {
    if let Some(activations) = prepared_outputs
        .and_then(ProtocolOutputPayloads::input_mut)
        .and_then(InputPreparedOutputSlot::take_download_activations)
    {
        emit_download_activations(conn, out, session_id, activations, command_context).await;
    }
}

async fn emit_download_activations(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    activations: Vec<RendererPendingDownloadActivation>,
    command_context: &mut CommandDispatchContext,
) {
    for activation in activations {
        if let Err(error) = conn
            .handle_pending_download_activation_background_events_async(
                out,
                session_id,
                activation,
                command_context,
            )
            .await
        {
            tracing::warn!(
                error = error.as_str(),
                "pending download activation failed during protocol output projection"
            );
        }
    }
}

pub(in crate::domains) async fn emit_file_chooser_activity_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    if let Some(activations) = prepared_outputs
        .and_then(ProtocolOutputPayloads::input_mut)
        .and_then(InputPreparedOutputSlot::take_file_chooser_activations)
    {
        file_chooser::emit_prepared_activations_async(conn, out, session_id, activations).await;
    }
}

#[cfg(test)]
mod protocol_neutral_tests {
    use crate::devtools_runtime::{
        DevToolsCommand, DevToolsCommandContext, DevToolsDispatchDragEventCommand,
        DevToolsDispatchMouseEventCommand, DevToolsDispatchTouchEventCommand, DevToolsDragData,
        DevToolsDragEventType, DevToolsErrorKind, DevToolsKeyEventType, DevToolsMouseEventType,
        DevToolsPointerType, DevToolsProtocol, DevToolsSynthesizeTapGestureCommand,
        DevToolsTouchEventType, DevToolsTouchPoint,
    };
    use serde_json::Value;

    use crate::conn::{CdpConnection, Cmd};
    use crate::testing::real_layout_test_connection;

    fn assert_coordinate_routes_to_document_owner(
        result: Result<
            Option<super::PendingInputCommandDispatch>,
            super::PendingInputCommandStartError,
        >,
    ) {
        let Err(error) = result else {
            panic!("coordinate input without a document should fail at the document owner");
        };
        assert_eq!(error.code, -32000);
        assert_eq!(error.kind, DevToolsErrorKind::Internal);
        assert_eq!(error.message, "NoDocumentLoaded");
    }

    fn cdp_context(session_id: &str) -> DevToolsCommandContext {
        DevToolsCommandContext {
            protocol: DevToolsProtocol::Cdp,
            session_id: Some(session_id.to_owned().into()),
            target_id: None,
            browser_context_id: None,
        }
    }

    fn coordinate_mouse_command(session_id: &str) -> DevToolsDispatchMouseEventCommand {
        DevToolsDispatchMouseEventCommand {
            context: cdp_context(session_id),
            event_type: DevToolsMouseEventType::Pressed,
            pointer_type: DevToolsPointerType::Mouse,
            x: 1.0,
            y: 2.0,
            button: 0,
            buttons: Some(1),
            click_count: 1,
            delta_x: 0.0,
            delta_y: 0.0,
            force: 0.0,
            tangential_pressure: 0.0,
            tilt_x: 0.0,
            tilt_y: 0.0,
            twist: 0.0,
            modifiers: 0,
        }
    }

    fn coordinate_touch_command(session_id: &str) -> DevToolsDispatchTouchEventCommand {
        DevToolsDispatchTouchEventCommand {
            context: cdp_context(session_id),
            event_type: DevToolsTouchEventType::Start,
            touch_points: vec![DevToolsTouchPoint {
                id: 0,
                x: 1.0,
                y: 2.0,
            }],
        }
    }

    fn coordinate_drag_command(
        session_id: &str,
        drag_operations_mask: i32,
    ) -> DevToolsDispatchDragEventCommand {
        DevToolsDispatchDragEventCommand {
            context: cdp_context(session_id),
            event_type: DevToolsDragEventType::Drop,
            x: 1.0,
            y: 2.0,
            data: DevToolsDragData {
                items: Vec::new(),
                files: Vec::new(),
                drag_operations_mask,
            },
            modifiers: 0,
        }
    }

    #[test]
    fn devtools_input_entry_routes_coordinate_mouse_command_to_document_owner() {
        let mut conn = real_layout_test_connection();
        let result = super::start_devtools_input_command(
            &mut conn,
            Some(42),
            Some("SID-mouse"),
            DevToolsCommand::DispatchMouseEvent(coordinate_mouse_command("SID-mouse")),
        );

        assert_coordinate_routes_to_document_owner(result);
    }

    #[test]
    fn cdp_dispatch_key_event_builds_protocol_neutral_key_command() {
        let conn = CdpConnection::new();
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(43),
            "Input.dispatchKeyEvent",
            &params,
            Some("SID-key"),
            r#"{"id":43,"method":"Input.dispatchKeyEvent"}"#,
        );
        let parsed = super::key::ParsedDispatchKeyEvent {
            event_type: DevToolsKeyEventType::KeyDown,
            key: "A".to_owned(),
            code: "KeyA".to_owned(),
            text: "a".to_owned(),
            modifiers: 2,
            auto_repeat: true,
            should_insert_text: true,
        };

        let command = super::build_cdp_dispatch_key_event_command(&conn, &cmd, parsed);

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-key")
        );
        assert_eq!(command.context.target_id, None);
        assert_eq!(command.context.browser_context_id, None);
        assert_eq!(command.event_type, DevToolsKeyEventType::KeyDown);
        assert_eq!(command.key, "A");
        assert_eq!(command.code, "KeyA");
        assert_eq!(command.text, "a");
        assert_eq!(command.modifiers, 2);
        assert!(command.auto_repeat);
        assert!(command.should_insert_text);
    }

    #[test]
    fn devtools_input_entry_routes_key_command_to_input_owner() {
        let mut conn = CdpConnection::new();
        let params = Value::Null;
        let cmd = Cmd::for_test(
            Some(44),
            "Input.dispatchKeyEvent",
            &params,
            Some("SID-key"),
            r#"{"id":44,"method":"Input.dispatchKeyEvent"}"#,
        );
        let parsed = super::key::ParsedDispatchKeyEvent {
            event_type: DevToolsKeyEventType::KeyUp,
            key: "A".to_owned(),
            code: "KeyA".to_owned(),
            text: String::new(),
            modifiers: 0,
            auto_repeat: false,
            should_insert_text: false,
        };
        let command = super::build_cdp_dispatch_key_event_command(&conn, &cmd, parsed);

        let result = super::start_devtools_input_command(
            &mut conn,
            cmd.id,
            cmd.session_id,
            DevToolsCommand::DispatchKeyEvent(command),
        );

        let Err(error) = result else {
            panic!("missing document should surface through the unified input entry");
        };
        assert_eq!(error.code, -32000);
        assert_eq!(error.message, "NoDocumentLoaded");
    }

    #[test]
    fn devtools_input_entry_routes_coordinate_touch_command_to_document_owner() {
        let mut conn = real_layout_test_connection();
        let result = super::start_devtools_input_command(
            &mut conn,
            Some(46),
            Some("SID-touch"),
            DevToolsCommand::DispatchTouchEvent(coordinate_touch_command("SID-touch")),
        );

        assert_coordinate_routes_to_document_owner(result);
    }

    #[test]
    fn devtools_input_entry_routes_coordinate_drag_command_to_document_owner() {
        let mut conn = real_layout_test_connection();
        let result = super::start_devtools_input_command(
            &mut conn,
            Some(48),
            Some("SID-drag"),
            DevToolsCommand::DispatchDragEvent(coordinate_drag_command("SID-drag", 1)),
        );

        assert_coordinate_routes_to_document_owner(result);
    }

    #[test]
    fn devtools_input_entry_routes_zero_mask_coordinate_drag_drop_to_document_owner() {
        let mut conn = real_layout_test_connection();
        let result = super::start_devtools_input_command(
            &mut conn,
            Some(49),
            Some("SID-drag"),
            DevToolsCommand::DispatchDragEvent(coordinate_drag_command("SID-drag", 0)),
        );

        assert_coordinate_routes_to_document_owner(result);
    }

    #[test]
    fn devtools_input_entry_routes_coordinate_tap_command_to_document_owner() {
        let mut conn = real_layout_test_connection();
        let result = super::start_devtools_input_command(
            &mut conn,
            Some(51),
            Some("SID-tap"),
            DevToolsCommand::SynthesizeTapGesture(DevToolsSynthesizeTapGestureCommand {
                context: cdp_context("SID-tap"),
                x: 1.0,
                y: 2.0,
            }),
        );

        assert_coordinate_routes_to_document_owner(result);
    }
}

#[cfg(test)]
mod producer_tests {
    use moli_core::page::{
        RENDERER_BACKEND_NODE_ID_START, RendererDocumentLifecycleIdentity,
        RendererDocumentLifecycleSnapshot, RendererDocumentToken, RendererFrameToken,
        RendererLifecycleEpoch, RendererLifecycleEventStamp, RendererPageCreationArtifacts,
        RendererPendingDownloadActivation, RendererPendingFileChooserActivation,
    };
    use serde_json::json;

    use crate::conn::{
        BackgroundProtocolEvent, BrowserContext, CdpConnection, CommandDispatchContext,
        TargetPageResidenceIdentity, build_event,
    };
    use crate::devtools_runtime::{
        AutomationEvent, BrowserDownloadProgressEvent, BrowserDownloadWillBeginEvent,
        DevToolsFrameId, webdriver_bidi_node_shared_id_for_backend_node_id,
    };
    use crate::domains::activity::ProtocolOutputPayloads;

    fn renderer_document_identity_for_test(
        lifecycle_document_id: u64,
        epoch: u64,
    ) -> RendererDocumentLifecycleIdentity {
        let page_id = moli_core::PageId::new_for_testing(31);
        RendererDocumentLifecycleIdentity {
            frame: RendererFrameToken { page_id },
            document: RendererDocumentToken::new_for_testing(page_id, lifecycle_document_id),
            epoch: RendererLifecycleEpoch(epoch),
        }
    }

    fn bind_renderer_document_for_test(
        conn: &mut CdpConnection,
        session_id: &str,
        frame_id: &str,
        identity: RendererDocumentLifecycleIdentity,
    ) {
        let runtime_slot = conn
            .runtime_session_owner_slot_mut(Some(session_id))
            .expect("test target should expose a runtime owner slot");
        if runtime_slot.page_attachment_id().is_none() {
            runtime_slot.set_page_attachment_id_for_test(identity.document.page_id.as_u64());
        }
        let lifecycle_snapshot = RendererDocumentLifecycleSnapshot {
            frame: identity.frame,
            document: identity.document,
            epoch: identity.epoch,
            started: RendererLifecycleEventStamp {
                sequence: 1,
                timestamp_micros: 1,
            },
            dom_content_loaded: None,
            load: None,
            terminated: None,
        };
        let (binding, initial_events) = conn.bind_renderer_document_lifecycle_for_session_owner(
            Some(session_id),
            RendererPageCreationArtifacts {
                active_document: identity.document,
                active_epoch: identity.epoch,
                lifecycle_snapshot,
                initial_lifecycle_events: Vec::new(),
            },
            None,
            frame_id.to_owned(),
            "LOADER-file-chooser".to_owned(),
        );
        assert!(binding.is_some(), "test renderer Document should bind");
        assert!(initial_events.is_empty());
    }

    fn page_residence_identity_for_test(
        conn: &CdpConnection,
        session_id: &str,
    ) -> TargetPageResidenceIdentity {
        conn.target_page_residence_identity_for_session(Some(session_id))
            .expect("test target should expose a Page residence identity")
    }

    fn renderer_file_chooser_for_test(
        source_document: RendererDocumentLifecycleIdentity,
        source_frame_id: Option<&str>,
        backend_node_id: u32,
        allow_multiple: bool,
    ) -> RendererPendingFileChooserActivation {
        RendererPendingFileChooserActivation::new(
            source_document,
            source_frame_id.map(str::to_owned),
            RENDERER_BACKEND_NODE_ID_START + backend_node_id,
            allow_multiple,
        )
    }

    #[test]
    fn input_prepared_slot_keeps_download_and_file_chooser_payloads_separate() {
        let source_document = renderer_document_identity_for_test(1, 1);
        let owner = TargetPageResidenceIdentity::new_for_test(
            "BID-slot".to_owned(),
            Some("TID-slot".to_owned()),
            1,
        );
        let mut slot = super::InputPreparedOutputSlot::from_outputs(super::InputPreparedOutputs {
            download_activations: vec![RendererPendingDownloadActivation {
                url: "https://example.test/download".to_owned(),
                suggested_filename: Some("download.txt".to_owned()),
                response: None,
            }],
            file_chooser_activations: vec![
                super::file_chooser::PreparedFileChooserActivation::from_renderer_for_test(
                    owner,
                    "ROOT-slot",
                    renderer_file_chooser_for_test(source_document, Some("FRAME-1"), 42, false),
                ),
            ],
        });

        assert_eq!(
            slot.take_download_activations().map(|items| items.len()),
            Some(1)
        );
        assert_eq!(
            slot.take_file_chooser_activations()
                .map(|items| items.len()),
            Some(1)
        );
        assert!(slot.take_download_activations().is_none());
        assert!(slot.take_file_chooser_activations().is_none());
    }

    #[test]
    fn input_command_side_effect_events_preserve_typed_sidecars() {
        let mut side_effects = super::InputCommandSideEffects::default();
        let mut command_context = CommandDispatchContext::default();
        side_effects.extend_protocol_events(
            &mut command_context,
            vec![BackgroundProtocolEvent::immediate_automation_event(
                build_event(
                    "Browser.downloadWillBegin",
                    json!({
                        "frameId": "FRAME-download",
                        "guid": "GUID-download",
                        "url": "https://example.test/download",
                        "suggestedFilename": "download.txt"
                    }),
                    None,
                ),
                AutomationEvent::BrowserDownloadWillBegin(BrowserDownloadWillBeginEvent {
                    frame_id: DevToolsFrameId::from("FRAME-download"),
                    guid: "GUID-download".to_owned(),
                    url: "https://example.test/download".to_owned(),
                    suggested_filename: "download.txt".to_owned(),
                }),
            )],
        );

        let mut events = side_effects.into_events();

        assert_eq!(events.len(), 1);
        let (message, automation_event) = events.remove(0).into_parts();
        assert_eq!(message["method"], json!("Browser.downloadWillBegin"));
        assert_eq!(message["params"]["frameId"], json!("FRAME-download"));
        assert_eq!(message["params"]["guid"], json!("GUID-download"));
        let Some(AutomationEvent::BrowserDownloadWillBegin(event)) = automation_event else {
            panic!("expected typed Browser.downloadWillBegin automation sidecar");
        };
        assert_eq!(event.frame_id.as_str(), "FRAME-download");
        assert_eq!(event.guid, "GUID-download");
        assert_eq!(event.url, "https://example.test/download");
        assert_eq!(event.suggested_filename, "download.txt");
    }

    #[test]
    fn input_command_side_effect_events_include_post_response_events() {
        let mut side_effects = super::InputCommandSideEffects::default();
        let mut command_context = CommandDispatchContext::default();
        command_context.extend_post_response_events(vec![
            BackgroundProtocolEvent::immediate_automation_event(
                build_event(
                    "Browser.downloadProgress",
                    json!({
                        "guid": "GUID-download",
                        "state": "completed",
                        "receivedBytes": 123,
                        "totalBytes": 123,
                        "filePath": "/tmp/download.txt"
                    }),
                    None,
                ),
                AutomationEvent::BrowserDownloadProgress(BrowserDownloadProgressEvent {
                    guid: "GUID-download".to_owned(),
                    state: "completed".to_owned(),
                    received_bytes: 123,
                    total_bytes: 123,
                    file_path: Some("/tmp/download.txt".to_owned()),
                }),
            ),
        ]);
        side_effects.extend_protocol_events(&mut command_context, Vec::new());

        let mut events = side_effects.into_events();

        assert_eq!(events.len(), 1);
        let (message, automation_event) = events.remove(0).into_parts();
        assert_eq!(message["method"], json!("Browser.downloadProgress"));
        assert_eq!(message["params"]["guid"], json!("GUID-download"));
        assert_eq!(message["params"]["state"], json!("completed"));
        let Some(AutomationEvent::BrowserDownloadProgress(event)) = automation_event else {
            panic!("expected typed Browser.downloadProgress automation sidecar");
        };
        assert_eq!(event.guid, "GUID-download");
        assert_eq!(event.state, "completed");
        assert_eq!(event.received_bytes, 123);
        assert_eq!(event.total_bytes, 123);
        assert_eq!(event.file_path.as_deref(), Some("/tmp/download.txt"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn file_chooser_opened_preserves_typed_automation_sidecar() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-typed".into());
        bc.attach_active_session("SID-typed");
        bc.devtools_session_state
            .page_session_state
            .page_file_chooser_opened_event_enabled = true;
        conn.browser_context = Some(bc);
        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(&mut conn, "SID-typed", "ROOT-typed", source_document);
        let owner = page_residence_identity_for_test(&conn, "SID-typed");
        let mut out: Vec<BackgroundProtocolEvent> = Vec::new();

        let prepared = super::file_chooser::PreparedFileChooserActivation::capture(
            &conn,
            Some("SID-typed"),
            owner,
            renderer_file_chooser_for_test(source_document, Some("FRAME-typed"), 77, false),
        )
        .expect("current file chooser should prepare");
        super::file_chooser::emit_prepared_activation_async(
            &mut conn,
            &mut out,
            Some("SID-typed"),
            prepared,
        )
        .await;

        assert_eq!(out.len(), 1);
        let (message, automation_event) = out.remove(0).into_parts();
        assert_eq!(message["method"], json!("Page.fileChooserOpened"));
        assert_eq!(message["params"]["frameId"], json!("FRAME-typed"));
        assert_eq!(message["params"]["mode"], json!("selectSingle"));
        assert_eq!(
            message["params"]["backendNodeId"],
            json!(RENDERER_BACKEND_NODE_ID_START + 77)
        );
        assert_eq!(message["sessionId"], json!("SID-typed"));
        let Some(AutomationEvent::PageFileChooserOpened(event)) = automation_event else {
            panic!("expected typed Page.fileChooserOpened automation sidecar");
        };
        assert_eq!(event.frame_id.as_str(), "FRAME-typed");
        assert_eq!(event.mode, "selectSingle");
        assert_eq!(event.backend_node_id, RENDERER_BACKEND_NODE_ID_START + 77);
        assert_eq!(
            event.element_shared_id,
            Some(webdriver_bidi_node_shared_id_for_backend_node_id(
                RENDERER_BACKEND_NODE_ID_START + 77
            )),
            "the typed event must retain its frozen renderer node identity even when no live renderer binding can be installed"
        );
    }

    #[test]
    fn file_chooser_capture_resolves_root_frame_once() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-root-capture".into());
        bc.set_active_target_id("TID-root-capture");
        bc.attach_active_session("SID-root-capture");
        conn.browser_context = Some(bc);
        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(
            &mut conn,
            "SID-root-capture",
            "TID-root-capture",
            source_document,
        );
        let owner = page_residence_identity_for_test(&conn, "SID-root-capture");

        let prepared = super::file_chooser::PreparedFileChooserActivation::capture(
            &conn,
            Some("SID-root-capture"),
            owner,
            renderer_file_chooser_for_test(source_document, None, 41, false),
        )
        .expect("root file chooser should bind to the current Document");

        assert_eq!(prepared.source_document(), source_document);
        assert_eq!(prepared.source_frame_id(), "TID-root-capture");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn document_open_replacement_preserves_causal_file_chooser_activation() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-document-collision".into());
        bc.set_active_target_id("TID-document-collision");
        bc.attach_active_session("SID-document-collision");
        bc.devtools_session_state
            .page_session_state
            .page_file_chooser_opened_event_enabled = true;
        conn.browser_context = Some(bc);
        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(
            &mut conn,
            "SID-document-collision",
            "TID-document-collision",
            source_document,
        );
        let owner = page_residence_identity_for_test(&conn, "SID-document-collision");
        let prepared = super::file_chooser::PreparedFileChooserActivation::capture(
            &conn,
            Some("SID-document-collision"),
            owner,
            renderer_file_chooser_for_test(source_document, None, 42, false),
        )
        .expect("source file chooser should prepare");

        let replacement_document = renderer_document_identity_for_test(2, 2);
        bind_renderer_document_for_test(
            &mut conn,
            "SID-document-collision",
            "TID-document-collision",
            replacement_document,
        );
        let mut out = Vec::new();

        super::file_chooser::emit_prepared_activation_async(
            &mut conn,
            &mut out,
            Some("SID-document-collision"),
            prepared,
        )
        .await;

        assert_eq!(
            out.len(),
            1,
            "Chromium preserves an activation whose listener synchronously replaced the Document"
        );
        let (message, automation_event) = out.remove(0).into_parts();
        assert_eq!(
            message["params"]["backendNodeId"],
            json!(RENDERER_BACKEND_NODE_ID_START + 42)
        );
        assert_eq!(message["params"]["mode"], json!("selectSingle"));
        assert_eq!(
            message["params"]["frameId"],
            json!("TID-document-collision")
        );
        let Some(AutomationEvent::PageFileChooserOpened(event)) = automation_event else {
            panic!("expected typed Page.fileChooserOpened automation sidecar");
        };
        assert_eq!(
            event.element_shared_id,
            Some(webdriver_bidi_node_shared_id_for_backend_node_id(
                RENDERER_BACKEND_NODE_ID_START + 42
            )),
            "document.open must not erase the causal element identity carried by the event"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn replaced_page_residence_discards_only_stale_backend_node_collision() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-page-replacement".into());
        bc.set_active_target_id("TID-page-replacement");
        bc.attach_active_session("SID-page-replacement");
        bc.devtools_session_state
            .page_session_state
            .page_file_chooser_opened_event_enabled = true;
        conn.browser_context = Some(bc);
        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(
            &mut conn,
            "SID-page-replacement",
            "TID-page-replacement",
            source_document,
        );
        let owner = page_residence_identity_for_test(&conn, "SID-page-replacement");
        let stale = super::file_chooser::PreparedFileChooserActivation::capture(
            &conn,
            Some("SID-page-replacement"),
            owner.clone(),
            renderer_file_chooser_for_test(source_document, None, 43, false),
        )
        .expect("source file chooser should prepare");
        conn.runtime_session_owner_slot_mut(Some("SID-page-replacement"))
            .expect("test runtime slot should exist")
            .replace_page_attachment_id_for_test();
        let replacement_document = renderer_document_identity_for_test(2, 2);
        bind_renderer_document_for_test(
            &mut conn,
            "SID-page-replacement",
            "TID-page-replacement",
            replacement_document,
        );
        let replacement_owner = page_residence_identity_for_test(&conn, "SID-page-replacement");
        let current = super::file_chooser::PreparedFileChooserActivation::capture(
            &conn,
            Some("SID-page-replacement"),
            replacement_owner,
            renderer_file_chooser_for_test(replacement_document, None, 43, true),
        )
        .expect("replacement Page activation should prepare");
        let mut out = Vec::new();

        super::file_chooser::emit_prepared_activations_async(
            &mut conn,
            &mut out,
            Some("SID-page-replacement"),
            vec![stale, current],
        )
        .await;

        assert_eq!(
            out.len(),
            1,
            "a retired Page must not project a colliding backend node id into its replacement"
        );
        let message = out.remove(0).into_protocol_message();
        assert_eq!(
            message["params"]["backendNodeId"],
            json!(RENDERER_BACKEND_NODE_ID_START + 43)
        );
        assert_eq!(message["params"]["mode"], json!("selectMultiple"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn download_activity_without_protocol_observers_keeps_typed_automation_events() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-download".into());
        bc.set_active_target_id("FRAME-download");
        bc.attach_active_session("SID-download");
        conn.browser_context = Some(bc);
        conn.download_behavior
            .set_global("deny".to_owned(), None, true);
        let mut out: Vec<BackgroundProtocolEvent> = Vec::new();
        let mut command_context = CommandDispatchContext::default();
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::InputPreparedOutputSlot::from_outputs(
                super::InputPreparedOutputs::from_download_activations_for_test(vec![
                    RendererPendingDownloadActivation {
                        url: "https://example.test/report.txt".to_owned(),
                        suggested_filename: Some("report.txt".to_owned()),
                        response: None,
                    },
                ]),
            ));

        super::emit_download_activity_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-download"),
            Some(&mut prepared),
            &mut command_context,
        )
        .await;

        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|event| !event.has_protocol_wire_message()));
        let (_, will_begin_event) = out.remove(0).into_parts();
        let Some(AutomationEvent::BrowserDownloadWillBegin(will_begin)) = will_begin_event else {
            panic!("expected typed Browser.downloadWillBegin automation event");
        };
        assert_eq!(will_begin.frame_id.as_str(), "FRAME-download");
        assert_eq!(will_begin.url, "https://example.test/report.txt");
        assert_eq!(will_begin.suggested_filename, "report.txt");

        let (_, progress_event) = out.remove(0).into_parts();
        let Some(AutomationEvent::BrowserDownloadProgress(progress)) = progress_event else {
            panic!("expected typed Browser.downloadProgress automation event");
        };
        assert_eq!(progress.guid, will_begin.guid);
        assert_eq!(progress.state, "canceled");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn file_chooser_drain_consumes_prepared_activations_without_page_readback() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.attach_active_session("SID-1");
        bc.devtools_session_state
            .page_session_state
            .page_file_chooser_opened_event_enabled = true;
        conn.browser_context = Some(bc);
        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(&mut conn, "SID-1", "ROOT-1", source_document);
        let owner = page_residence_identity_for_test(&conn, "SID-1");
        let mut out: Vec<BackgroundProtocolEvent> = Vec::new();
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::InputPreparedOutputSlot::from_outputs(
                super::InputPreparedOutputs::from_file_chooser_activations_for_test(
                    owner,
                    "ROOT-1",
                    vec![renderer_file_chooser_for_test(
                        source_document,
                        Some("FRAME-1"),
                        42,
                        true,
                    )],
                ),
            ));

        super::emit_file_chooser_activity_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-1"),
            Some(&mut prepared),
        )
        .await;

        assert_eq!(out.len(), 1);
        let message = out.remove(0).into_protocol_message();
        assert_eq!(message["method"], json!("Page.fileChooserOpened"));
        assert_eq!(message["params"]["frameId"], json!("FRAME-1"));
        assert_eq!(message["params"]["mode"], json!("selectMultiple"));
        assert_eq!(
            message["params"]["backendNodeId"],
            json!(RENDERER_BACKEND_NODE_ID_START + 42)
        );
        assert_eq!(message["sessionId"], json!("SID-1"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn file_chooser_activity_background_events_keep_typed_sidecar() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new("BID-context".into());
        bc.attach_active_session("SID-context");
        bc.devtools_session_state
            .page_session_state
            .page_file_chooser_opened_event_enabled = true;
        conn.browser_context = Some(bc);
        let source_document = renderer_document_identity_for_test(1, 1);
        bind_renderer_document_for_test(&mut conn, "SID-context", "ROOT-context", source_document);
        let owner = page_residence_identity_for_test(&conn, "SID-context");
        let mut out: Vec<BackgroundProtocolEvent> = Vec::new();
        let mut prepared =
            ProtocolOutputPayloads::from_slot(super::InputPreparedOutputSlot::from_outputs(
                super::InputPreparedOutputs::from_file_chooser_activations_for_test(
                    owner,
                    "ROOT-context",
                    vec![renderer_file_chooser_for_test(
                        source_document,
                        Some("FRAME-context"),
                        88,
                        false,
                    )],
                ),
            ));

        super::emit_file_chooser_activity_background_events_async(
            &mut conn,
            &mut out,
            Some("SID-context"),
            Some(&mut prepared),
        )
        .await;

        assert_eq!(out.len(), 1);
        let (message, automation_event) = out.remove(0).into_parts();
        assert_eq!(message["method"], json!("Page.fileChooserOpened"));
        assert_eq!(message["params"]["frameId"], json!("FRAME-context"));
        assert_eq!(message["params"]["mode"], json!("selectSingle"));
        assert_eq!(
            message["params"]["backendNodeId"],
            json!(RENDERER_BACKEND_NODE_ID_START + 88)
        );
        assert_eq!(message["sessionId"], json!("SID-context"));
        let Some(AutomationEvent::PageFileChooserOpened(event)) = automation_event else {
            panic!("expected typed Page.fileChooserOpened sidecar");
        };
        assert_eq!(event.frame_id.as_str(), "FRAME-context");
        assert_eq!(event.mode, "selectSingle");
        assert_eq!(event.backend_node_id, RENDERER_BACKEND_NODE_ID_START + 88);
    }
}
