use super::*;

pub(super) fn has_cookie(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get(COOKIE)
        .and_then(|value| value.to_str().ok())
        .map(|cookies| cookies.contains(expected))
        .unwrap_or(false)
}

pub(super) fn redirect_with_cookies(location: &str, cookies: &[&str]) -> Response {
    let mut response = Redirect::temporary(location).into_response();
    for cookie in cookies {
        response.headers_mut().append(
            SET_COOKIE,
            HeaderValue::from_str(cookie).expect("valid set-cookie value"),
        );
    }
    response
}

pub(super) fn javascript_response(source: impl IntoResponse) -> Response {
    let mut response = source.into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/javascript"),
    );
    response
}

pub(super) use self::javascript_response as javascript_string_response;

pub(super) fn css_response(source: &'static str) -> Response {
    let mut response = source.into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/css; charset=utf-8"),
    );
    response
}
