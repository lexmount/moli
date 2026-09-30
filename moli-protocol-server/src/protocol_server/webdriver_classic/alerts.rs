use axum::{
    body::Bytes,
    extract::{Path, State},
    response::Response,
};
use moli_protocol::automation::AutomationResult;
use moli_protocol_webdriver_classic::{
    ClassicError, ClassicErrorCode, alert_handle_command, alert_send_text_command,
    alert_text_command, classic_error_from_devtools_error,
};
use serde_json::{Value, json};

use super::super::AppState;
use super::{
    classic_browsing_context, classic_error_into_response, classic_json_body,
    classic_success_into_response,
};

pub(in crate::protocol_server) async fn webdriver_classic_get_alert_text(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let binding = match state
        .classic_session_registry
        .lock()
        .session_binding(&session_id)
    {
        Some(binding) => binding,
        None => {
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::InvalidSessionId,
                "session not found",
            ));
        }
    };
    match get_alert_dialog(&binding).await {
        Ok(dialog) => classic_success_into_response(json!(dialog.message)),
        Err(error) => classic_error_into_response(error),
    }
}

pub(in crate::protocol_server) async fn webdriver_classic_accept_alert(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    handle_alert(session_id, state, true).await
}

pub(in crate::protocol_server) async fn webdriver_classic_dismiss_alert(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    handle_alert(session_id, state, false).await
}

pub(in crate::protocol_server) async fn webdriver_classic_send_alert_text(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let params = match classic_json_body(&body) {
        Ok(params) => params,
        Err(error) => return classic_error_into_response(error),
    };
    let binding = match state
        .classic_session_registry
        .lock()
        .session_binding(&session_id)
    {
        Some(binding) => binding,
        None => {
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::InvalidSessionId,
                "session not found",
            ));
        }
    };
    let dialog = match get_alert_dialog(&binding).await {
        Ok(dialog) => dialog,
        Err(error) => return classic_error_into_response(error),
    };
    if dialog.dialog_type != "prompt" {
        return classic_error_into_response(ClassicError::new(
            ClassicErrorCode::ElementNotInteractable,
            "current user prompt is not a prompt",
        ));
    }
    let context = classic_browsing_context(&binding);
    let command = match alert_send_text_command(&context, &params) {
        Ok(command) => command,
        Err(error) => return classic_error_into_response(error),
    };
    match binding.runtime.execute(command).await {
        Ok(AutomationResult::Empty) => classic_success_into_response(Value::Null),
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "send alert text returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

async fn handle_alert(session_id: String, state: AppState, accept: bool) -> Response {
    let binding = match state
        .classic_session_registry
        .lock()
        .session_binding(&session_id)
    {
        Some(binding) => binding,
        None => {
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::InvalidSessionId,
                "session not found",
            ));
        }
    };
    let context = classic_browsing_context(&binding);
    match binding
        .runtime
        .execute(alert_handle_command(&context, accept))
        .await
    {
        Ok(AutomationResult::Empty) => classic_success_into_response(Value::Null),
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "handle alert returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

struct ClassicAlertDialog {
    dialog_type: String,
    message: String,
}

async fn get_alert_dialog(
    binding: &super::state::ClassicSessionBinding,
) -> Result<ClassicAlertDialog, ClassicError> {
    let context = classic_browsing_context(binding);
    match binding.runtime.execute(alert_text_command(&context)).await {
        Ok(AutomationResult::JavaScriptDialog(result)) => Ok(ClassicAlertDialog {
            dialog_type: result.dialog_type,
            message: result.message,
        }),
        Ok(_) => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "get alert text returned an unexpected result",
        )),
        Err(error) => Err(classic_error_from_devtools_error(error)),
    }
}
