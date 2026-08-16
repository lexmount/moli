use axum::{
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use moli_protocol_webdriver_classic::{
    ClassicError, ClassicErrorCode, error_response_with_data as classic_error_response_with_data,
    success_response,
};
use serde_json::Value;
use serde_json::json;

pub(super) fn classic_json_body(body: &[u8]) -> Result<serde_json::Value, ClassicError> {
    if body.is_empty() {
        return Ok(json!({}));
    }
    let value: serde_json::Value = serde_json::from_slice(body).map_err(|error| {
        ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("invalid JSON: {error}"),
        )
    })?;
    if !value.is_object() {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "params must be an object",
        ));
    }
    Ok(value)
}

pub(super) fn classic_error_into_response(error: ClassicError) -> Response {
    classic_webdriver_json_response(
        classic_error_status(error.code),
        classic_error_response_with_data(error.code, error.message, error.data),
    )
}

pub(super) fn classic_success_into_response(value: Value) -> Response {
    classic_webdriver_json_response(StatusCode::OK, success_response(value))
}

pub(super) fn classic_webdriver_json_response(status: StatusCode, body: Value) -> Response {
    let mut response = (status, axum::Json(body)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-cache"),
    );
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json; charset=utf-8"),
    );
    response
}

fn classic_error_status(code: ClassicErrorCode) -> StatusCode {
    match code {
        ClassicErrorCode::InvalidArgument
        | ClassicErrorCode::InvalidCookieDomain
        | ClassicErrorCode::InvalidElementState
        | ClassicErrorCode::InvalidSelector
        | ClassicErrorCode::ElementNotInteractable => StatusCode::BAD_REQUEST,
        ClassicErrorCode::InvalidSessionId
        | ClassicErrorCode::NoSuchAlert
        | ClassicErrorCode::NoSuchCookie
        | ClassicErrorCode::NoSuchElement
        | ClassicErrorCode::NoSuchFrame
        | ClassicErrorCode::NoSuchShadowRoot
        | ClassicErrorCode::NoSuchWindow
        | ClassicErrorCode::DetachedShadowRoot
        | ClassicErrorCode::StaleElementReference => StatusCode::NOT_FOUND,
        ClassicErrorCode::JavascriptError | ClassicErrorCode::MoveTargetOutOfBounds => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
        ClassicErrorCode::ScriptTimeout => StatusCode::INTERNAL_SERVER_ERROR,
        ClassicErrorCode::Timeout => StatusCode::REQUEST_TIMEOUT,
        ClassicErrorCode::UnknownCommand => StatusCode::NOT_FOUND,
        ClassicErrorCode::UnsupportedOperation => StatusCode::METHOD_NOT_ALLOWED,
        ClassicErrorCode::SessionNotCreated
        | ClassicErrorCode::UnexpectedAlertOpen
        | ClassicErrorCode::UnknownError => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[cfg(test)]
mod tests {
    use moli_protocol_webdriver_classic::{ClassicError, ClassicErrorCode};
    use serde_json::json;

    use super::{classic_error_into_response, classic_success_into_response};

    #[test]
    fn classic_success_response_includes_webdriver_headers() {
        let response = classic_success_into_response(json!("ok"));

        assert_eq!(response.headers().get("cache-control").unwrap(), "no-cache");
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/json; charset=utf-8"
        );
    }

    #[test]
    fn classic_error_response_includes_webdriver_headers() {
        let response = classic_error_into_response(ClassicError::new(
            ClassicErrorCode::NoSuchElement,
            "missing",
        ));

        assert_eq!(response.headers().get("cache-control").unwrap(), "no-cache");
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/json; charset=utf-8"
        );
    }
}
