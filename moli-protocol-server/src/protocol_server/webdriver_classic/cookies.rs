use axum::{
    body::Bytes,
    extract::{Path, State},
    response::Response,
};
use moli_protocol::automation::AutomationResult;
use moli_protocol_webdriver_classic::{
    ClassicDevToolsCommandContext, ClassicError, ClassicErrorCode, add_cookie_command,
    classic_cookie_by_name, classic_cookies_from_devtools, classic_error_from_devtools_error,
    delete_all_cookies_command, delete_cookie_command, get_cookies_command,
};
use serde_json::{Value, json};

use super::super::AppState;
use super::{
    classic_current_browsing_context_binding, classic_current_url, classic_error_into_response,
    classic_json_body, classic_success_into_response,
};

pub(in crate::protocol_server) async fn webdriver_classic_get_cookies(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let current_url = match classic_current_url(&binding).await {
        Ok(current_url) => current_url,
        Err(error) => return classic_error_into_response(error),
    };
    let context =
        ClassicDevToolsCommandContext::with_target_id(&binding.session_id, &binding.target_id);
    match binding
        .runtime
        .execute(get_cookies_command(&context, current_url))
        .await
    {
        Ok(AutomationResult::GetCookies(result)) => {
            classic_success_into_response(json!(classic_cookies_from_devtools(result)))
        }
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "get cookies returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

pub(in crate::protocol_server) async fn webdriver_classic_get_named_cookie(
    Path((session_id, name)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let current_url = match classic_current_url(&binding).await {
        Ok(current_url) => current_url,
        Err(error) => return classic_error_into_response(error),
    };
    let context =
        ClassicDevToolsCommandContext::with_target_id(&binding.session_id, &binding.target_id);
    match binding
        .runtime
        .execute(get_cookies_command(&context, current_url))
        .await
    {
        Ok(AutomationResult::GetCookies(result)) => match classic_cookie_by_name(result, &name) {
            Ok(cookie) => classic_success_into_response(cookie),
            Err(error) => classic_error_into_response(error),
        },
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "get cookie returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

pub(in crate::protocol_server) async fn webdriver_classic_add_cookie(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let params = match classic_json_body(&body) {
        Ok(params) => params,
        Err(error) => return classic_error_into_response(error),
    };
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let current_url = match classic_current_url(&binding).await {
        Ok(current_url) => current_url,
        Err(error) => return classic_error_into_response(error),
    };
    let context =
        ClassicDevToolsCommandContext::with_target_id(&binding.session_id, &binding.target_id);
    let command = match add_cookie_command(&context, &params, current_url) {
        Ok(command) => command,
        Err(error) => return classic_error_into_response(error),
    };
    match binding.runtime.execute(command).await {
        Ok(AutomationResult::SetCookies(result)) if result.success => {
            classic_success_into_response(Value::Null)
        }
        Ok(AutomationResult::SetCookies(_)) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::InvalidCookieDomain,
            "cookie could not be set for the current document",
        )),
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "add cookie returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

pub(in crate::protocol_server) async fn webdriver_classic_delete_all_cookies(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let current_url = match classic_current_url(&binding).await {
        Ok(current_url) => current_url,
        Err(error) => return classic_error_into_response(error),
    };
    let context =
        ClassicDevToolsCommandContext::with_target_id(&binding.session_id, &binding.target_id);
    match binding
        .runtime
        .execute(delete_all_cookies_command(&context, current_url))
        .await
    {
        Ok(AutomationResult::Empty | AutomationResult::DeleteCookies(_)) => {
            classic_success_into_response(Value::Null)
        }
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "delete cookies returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}

pub(in crate::protocol_server) async fn webdriver_classic_delete_cookie(
    Path((session_id, name)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Response {
    let binding = match classic_current_browsing_context_binding(&state, &session_id).await {
        Ok(binding) => binding,
        Err(error) => return classic_error_into_response(error),
    };
    let current_url = match classic_current_url(&binding).await {
        Ok(current_url) => current_url,
        Err(error) => return classic_error_into_response(error),
    };
    let context =
        ClassicDevToolsCommandContext::with_target_id(&binding.session_id, &binding.target_id);
    match binding
        .runtime
        .execute(delete_cookie_command(&context, name, current_url))
        .await
    {
        Ok(AutomationResult::Empty | AutomationResult::DeleteCookies(_)) => {
            classic_success_into_response(Value::Null)
        }
        Ok(_) => classic_error_into_response(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "delete cookie returned an unexpected result",
        )),
        Err(error) => classic_error_into_response(classic_error_from_devtools_error(error)),
    }
}
