use axum::{
    body::Bytes,
    extract::{Path, State},
    response::Response,
};
use moli_protocol::devtools_runtime::{
    DevToolsCommand, DevToolsCommandResult, DevToolsScriptResult,
};
use moli_protocol_webdriver_classic::{
    ClassicDevToolsCommandContext, ClassicError, ClassicErrorCode, ClassicWindowRect,
    ClassicWindowState, classic_error_from_devtools_error, classic_window_rect_for_state,
    close_window_command, execute_sync_command, new_window_command, new_window_type,
    set_window_normal_surface_state_command, set_window_rect_command, set_window_rect_update,
    set_window_state_command, set_window_surface_state_command, switch_window_command,
    window_handles_command, window_handles_from_targets,
};
use serde_json::json;
use tracing::warn;

use super::super::AppState;
use super::{
    classic_error_into_response, classic_json_body, classic_session_binding,
    classic_success_into_response, classic_top_level_browsing_context_binding,
    classic_top_level_browsing_context_binding_without_prompt_handling,
    classic_webdriver_command_exception_error,
};

pub(in crate::protocol_server) async fn webdriver_classic_get_window(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_top_level_browsing_context_binding_without_prompt_handling(
        &state,
        &session_id,
    )
    .await
    {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    classic_success_into_response(json!(binding.target_id))
}

pub(in crate::protocol_server) async fn webdriver_classic_get_window_rect(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_top_level_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    match classic_current_window_rect(&state, &binding).await {
        Ok(rect) => classic_success_into_response(rect.value()),
        Err(error) => classic_error_into_response(error),
    }
}

pub(in crate::protocol_server) async fn webdriver_classic_set_window_rect(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let params = match classic_json_body(&body) {
        Ok(params) => params,
        Err(error) => return classic_error_into_response(error),
    };
    let update = match set_window_rect_update(&params) {
        Ok(update) => update,
        Err(error) => return classic_error_into_response(error),
    };
    let binding = match classic_top_level_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let current = match classic_current_window_rect(&state, &binding).await {
        Ok(rect) => rect,
        Err(error) => return classic_error_into_response(error),
    };
    let next = current.with_update(update);
    let context =
        ClassicDevToolsCommandContext::with_target_id(&binding.session_id, &binding.target_id);
    let mut commands = Vec::new();
    if next.width != current.width || next.height != current.height {
        commands.push(set_window_rect_command(&context, next.width, next.height));
    }
    commands.push(set_window_normal_surface_state_command(&context));
    match classic_apply_window_rect(&state, &binding, next, commands).await {
        Ok(rect) => classic_success_into_response(rect.value()),
        Err(error) => classic_error_into_response(error),
    }
}

pub(in crate::protocol_server) async fn webdriver_classic_maximize_window(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    webdriver_classic_set_window_state(session_id, state, ClassicWindowState::Maximized).await
}

pub(in crate::protocol_server) async fn webdriver_classic_minimize_window(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    webdriver_classic_set_window_state(session_id, state, ClassicWindowState::Minimized).await
}

pub(in crate::protocol_server) async fn webdriver_classic_fullscreen_window(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    webdriver_classic_set_window_state(session_id, state, ClassicWindowState::Fullscreen).await
}

pub(in crate::protocol_server) async fn webdriver_classic_get_window_handles(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_session_binding(&state, &session_id) {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = ClassicDevToolsCommandContext::new(&binding.session_id);
    match binding
        .runtime
        .execute(window_handles_command(&context))
        .await
    {
        Ok(DevToolsCommandResult::GetTargets(result)) => {
            classic_success_into_response(json!(window_handles_from_targets(result)))
        }
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "window handles returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

pub(in crate::protocol_server) async fn webdriver_classic_switch_window(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let params = match classic_json_body(&body) {
        Ok(params) => params,
        Err(error) => return classic_error_into_response(error),
    };
    let binding = match classic_session_binding(&state, &session_id) {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context = ClassicDevToolsCommandContext::new(&binding.session_id);
    let command = match switch_window_command(&context, &params) {
        Ok(command) => command,
        Err(error) => return classic_error_into_response(error),
    };
    let handle = params
        .get("handle")
        .and_then(serde_json::Value::as_str)
        .expect("switch_window_command validates handle");
    match binding.runtime.execute(command).await {
        Ok(DevToolsCommandResult::Empty) => {
            if !state
                .classic_session_registry
                .lock()
                .set_current_target_id(&session_id, handle)
            {
                return classic_error_into_response(ClassicError::new(
                    ClassicErrorCode::InvalidSessionId,
                    "session not found",
                ));
            }
            classic_success_into_response(serde_json::Value::Null)
        }
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "switch window returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

pub(in crate::protocol_server) async fn webdriver_classic_new_window(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    if body.is_empty() {
        return classic_error_into_response(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "params must be an object",
        ));
    }
    let params = match classic_json_body(&body) {
        Ok(params) => params,
        Err(error) => return classic_error_into_response(error),
    };
    let type_name = match new_window_type(&params) {
        Ok(type_name) => type_name,
        Err(error) => return classic_error_into_response(error),
    };
    let binding = match classic_top_level_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context =
        ClassicDevToolsCommandContext::with_target_id(&binding.session_id, &binding.target_id);
    let target_id = match binding.runtime.execute(new_window_command(&context)).await {
        Ok(DevToolsCommandResult::CreateTarget(result)) => result.target_id.into_string(),
        Ok(_) => {
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::UnknownError,
                "new window returned an unexpected result",
            ));
        }
        Err(error) => return classic_error_into_response(classic_error_from_devtools_error(error)),
    };
    classic_success_into_response(json!({
        "handle": target_id,
        "type": type_name,
    }))
}

pub(in crate::protocol_server) async fn webdriver_classic_close_window(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_top_level_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let context =
        ClassicDevToolsCommandContext::with_target_id(&binding.session_id, &binding.target_id);
    let command = match close_window_command(&context) {
        Ok(command) => command,
        Err(error) => return classic_error_into_response(error),
    };
    match binding.runtime.execute(command).await {
        Ok(DevToolsCommandResult::CloseTarget(result)) if result.success => {}
        Ok(DevToolsCommandResult::CloseTarget(_)) => {
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::UnknownError,
                "close window failed",
            ));
        }
        Ok(_) => {
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::UnknownError,
                "close window returned an unexpected result",
            ));
        }
        Err(error) => return classic_error_into_response(classic_error_from_devtools_error(error)),
    }
    state
        .classic_session_registry
        .lock()
        .remove_window_position(&binding.session_id, &binding.target_id);

    let handles = match binding
        .runtime
        .execute(window_handles_command(&context))
        .await
    {
        Ok(DevToolsCommandResult::GetTargets(result)) => window_handles_from_targets(result),
        Ok(_) => {
            return classic_error_into_response(ClassicError::new(
                ClassicErrorCode::UnknownError,
                "window handles returned an unexpected result",
            ));
        }
        Err(error) => return classic_error_into_response(classic_error_from_devtools_error(error)),
    };

    if let Some(next_target_id) = handles.first() {
        state
            .classic_session_registry
            .lock()
            .set_current_target_id(&session_id, next_target_id);
    } else {
        let runtime = state
            .classic_session_registry
            .lock()
            .release_session(&session_id);
        if let Some(runtime) = runtime {
            let cookie_commit = runtime.shutdown().await;
            if let Err(error) = state.cookie_profile.commit_and_save(cookie_commit) {
                warn!(?error, "failed to persist Classic cookie profile");
            }
        }
    }

    classic_success_into_response(json!(handles))
}

async fn classic_current_window_rect(
    state: &AppState,
    binding: &super::state::ClassicSessionBinding,
) -> Result<moli_protocol_webdriver_classic::ClassicWindowRect, ClassicError> {
    let position = state
        .classic_session_registry
        .lock()
        .window_position(&binding.session_id, &binding.target_id);
    let context =
        ClassicDevToolsCommandContext::with_target_id(&binding.session_id, &binding.target_id);
    // Headless window dimensions are live browser state, independent of whether
    // the page has published layout or resized since its last visual output.
    let command = execute_sync_command(
        &context,
        &json!({
            "script": "return [window.innerWidth, window.innerHeight];",
            "args": []
        }),
    )?;
    match binding.runtime.execute(command).await {
        Ok(DevToolsCommandResult::Script(result)) => match *result {
            DevToolsScriptResult::Value(value) => {
                let (width, height): (u32, u32) =
                    serde_json::from_value(value.value).map_err(|error| {
                        ClassicError::new(ClassicErrorCode::UnknownError, error.to_string())
                    })?;
                Ok(ClassicWindowRect {
                    x: position.x,
                    y: position.y,
                    width,
                    height,
                })
            }
            DevToolsScriptResult::Exception(exception) => {
                Err(classic_webdriver_command_exception_error(exception))
            }
        },
        Ok(_) => Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "window rect returned an unexpected result",
        )),
        Err(error) => Err(classic_error_from_devtools_error(error)),
    }
}

async fn webdriver_classic_set_window_state(
    session_id: String,
    state: AppState,
    window_state: ClassicWindowState,
) -> Response {
    let binding = match classic_top_level_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let current = match classic_current_window_rect(&state, &binding).await {
        Ok(rect) => rect,
        Err(error) => return classic_error_into_response(error),
    };
    let next = classic_window_rect_for_state(current, window_state);
    let context =
        ClassicDevToolsCommandContext::with_target_id(&binding.session_id, &binding.target_id);
    let mut commands = Vec::new();
    if let Some(command) = set_window_state_command(&context, window_state) {
        commands.push(command);
    }
    commands.push(set_window_surface_state_command(&context, window_state));
    match classic_apply_window_rect(&state, &binding, next, commands).await {
        Ok(rect) => classic_success_into_response(rect.value()),
        Err(error) => classic_error_into_response(error),
    }
}

async fn classic_apply_window_rect(
    state: &AppState,
    binding: &super::state::ClassicSessionBinding,
    next: ClassicWindowRect,
    commands: Vec<DevToolsCommand>,
) -> Result<ClassicWindowRect, ClassicError> {
    for command in commands {
        match binding.runtime.execute(command).await {
            Ok(DevToolsCommandResult::Empty) => {}
            Ok(_) => {
                return Err(ClassicError::new(
                    ClassicErrorCode::UnknownError,
                    "set window rect returned an unexpected result",
                ));
            }
            Err(error) => return Err(classic_error_from_devtools_error(error)),
        }
    }
    if !state.classic_session_registry.lock().set_window_position(
        &binding.session_id,
        &binding.target_id,
        next.position(),
    ) {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidSessionId,
            "session not found",
        ));
    }
    Ok(next)
}
