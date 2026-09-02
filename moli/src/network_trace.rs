use moli_cookie_jar::{StoredCookieQueryReport, StoredCookieSetReport};
use moli_core::page::{
    NavigationRedirect, Page, SubresourceNetworkOutcome, SubresourceNetworkRecord,
    SubresourceResponseWaitCriteria, WebSocketFrameDirection, WebSocketLifecycleEvent,
    WebSocketLifecycleKind, WebSocketNetworkEvent,
};
use moli_fetch::FetchConfig;
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NetworkTraceConfigSummary {
    pub(crate) explicit_http_proxy: bool,
    pub(crate) libcurl_env_proxy_fallback: bool,
    pub(crate) http_no_proxy: bool,
    pub(crate) proxy_bearer_token: bool,
    pub(crate) tls_verify_host: bool,
    pub(crate) obey_robots: bool,
    pub(crate) http_cache: bool,
    pub(crate) connect_timeout_ms: Option<u64>,
    pub(crate) request_timeout_ms: u64,
    pub(crate) max_concurrent: Option<u32>,
    pub(crate) max_host_open: Option<u32>,
    pub(crate) max_host_connections: Option<u8>,
    pub(crate) effective_max_host_connections: Option<u8>,
    pub(crate) max_total_connections: Option<u16>,
    pub(crate) http2_max_concurrent_streams: Option<u16>,
    pub(crate) max_response_size: Option<usize>,
    pub(crate) block_private_networks: bool,
    pub(crate) block_cidr_count: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct NetworkTraceOptions {
    pub(crate) include_matched_response_body: bool,
}

impl From<&FetchConfig> for NetworkTraceConfigSummary {
    fn from(config: &FetchConfig) -> Self {
        Self {
            explicit_http_proxy: config.http_proxy().is_some(),
            libcurl_env_proxy_fallback: config.http_proxy().is_none(),
            http_no_proxy: config.http_no_proxy().is_some(),
            proxy_bearer_token: config.proxy_bearer_token().is_some(),
            tls_verify_host: config.tls_verify_host(),
            obey_robots: config.obey_robots(),
            http_cache: config.http_cache_dir().is_some(),
            connect_timeout_ms: config.http_connect_timeout_ms(),
            request_timeout_ms: config.request_timeout_ms(),
            max_concurrent: config.http_max_concurrent().map(|value| value.get()),
            max_host_open: config.http_max_host_open().map(|value| value.get()),
            max_host_connections: config.http_max_host_connections(),
            effective_max_host_connections: config.effective_http_max_host_connections(),
            max_total_connections: config.http_max_total_connections(),
            http2_max_concurrent_streams: config.http2_max_concurrent_streams(),
            max_response_size: config.http_max_response_size(),
            block_private_networks: config.block_private_networks(),
            block_cidr_count: config.block_cidrs().len(),
        }
    }
}

pub(crate) fn render_network_trace(
    page: &Page,
    main_document_html: &str,
    response_wait: Option<&SubresourceResponseWaitCriteria>,
    config: Option<&NetworkTraceConfigSummary>,
    options: NetworkTraceOptions,
) -> Value {
    let mut payload = Map::new();
    if let Some(config) = config {
        payload.insert("config".to_owned(), render_config_summary(config));
    }
    payload.insert(
        "main_document".to_owned(),
        render_main_document_trace_from_parts(
            page.requested_url(),
            page.final_url(),
            page.status(),
            page.headers(),
            main_document_html,
            page.navigation_redirected(),
            page.navigation_redirect_count(),
        ),
    );
    payload.insert(
        "subresources".to_owned(),
        Value::Array(
            page.subresource_network_records()
                .iter()
                .map(render_subresource_network_record)
                .collect::<Vec<_>>(),
        ),
    );
    payload.insert(
        "websocket_frames".to_owned(),
        Value::Array(
            page.websocket_network_events()
                .iter()
                .map(render_websocket_network_event)
                .collect::<Vec<_>>(),
        ),
    );
    payload.insert(
        "websocket_lifecycle".to_owned(),
        Value::Array(
            page.websocket_lifecycle_events()
                .iter()
                .map(render_websocket_lifecycle_event)
                .collect::<Vec<_>>(),
        ),
    );
    payload.insert(
        "websocket_summary".to_owned(),
        render_websocket_summary(
            page.websocket_lifecycle_events(),
            page.websocket_network_events(),
        ),
    );
    if let Some(criteria) = response_wait
        && let Some(record) = page
            .subresource_network_records()
            .iter()
            .find(|record| criteria.diagnostic_matches(record))
    {
        payload.insert(
            "matched_response".to_owned(),
            render_subresource_network_record_with_body_option(
                record,
                options.include_matched_response_body,
            ),
        );
    }
    Value::Object(payload)
}

fn render_websocket_network_event(event: &WebSocketNetworkEvent) -> Value {
    json!({
        "url": event.url().as_str(),
        "document_url": event.document_url().as_str(),
        "direction": event.direction().as_str(),
        "opcode": event.opcode().as_str(),
        "payload_length": event.payload_length(),
    })
}

fn render_websocket_lifecycle_event(event: &WebSocketLifecycleEvent) -> Value {
    let mut payload = Map::new();
    payload.insert("url".to_owned(), json!(event.url().as_str()));
    payload.insert(
        "document_url".to_owned(),
        json!(event.document_url().as_str()),
    );
    payload.insert("type".to_owned(), json!(event.kind().as_str()));
    if let Some(error_text) = event.error_text() {
        payload.insert("error_text".to_owned(), json!(error_text));
    }
    if let Some(code) = event.close_code() {
        payload.insert("code".to_owned(), json!(code));
    }
    if let Some(reason) = event.close_reason() {
        payload.insert("reason".to_owned(), json!(reason));
    }
    if let Some(was_clean) = event.was_clean() {
        payload.insert("was_clean".to_owned(), json!(was_clean));
    }
    Value::Object(payload)
}

fn render_websocket_summary(
    lifecycle_events: &[WebSocketLifecycleEvent],
    frame_events: &[WebSocketNetworkEvent],
) -> Value {
    let mut socket_ids = std::collections::BTreeSet::new();
    let mut opened_socket_ids = std::collections::BTreeSet::new();
    let mut opens = 0_usize;
    let mut errors = 0_usize;
    let mut handshake_errors = 0_usize;
    let mut runtime_errors = 0_usize;
    let mut closing = 0_usize;
    let mut closes = 0_usize;
    let mut clean_closes = 0_usize;
    let mut unclean_closes = 0_usize;

    for event in lifecycle_events {
        socket_ids.insert(event.socket_id());
        match event.kind() {
            WebSocketLifecycleKind::Open => {
                opens += 1;
                opened_socket_ids.insert(event.socket_id());
            }
            WebSocketLifecycleKind::Error => {
                errors += 1;
                if opened_socket_ids.contains(&event.socket_id()) {
                    runtime_errors += 1;
                } else {
                    handshake_errors += 1;
                }
            }
            WebSocketLifecycleKind::Closing => {
                closing += 1;
            }
            WebSocketLifecycleKind::Close => {
                closes += 1;
                match event.was_clean() {
                    Some(true) => clean_closes += 1,
                    Some(false) => unclean_closes += 1,
                    None => {}
                }
            }
        }
    }

    let mut sent_frames = 0_usize;
    let mut received_frames = 0_usize;
    let mut sent_payload_bytes = 0_usize;
    let mut received_payload_bytes = 0_usize;
    for event in frame_events {
        socket_ids.insert(event.socket_id());
        match event.direction() {
            WebSocketFrameDirection::Sent => {
                sent_frames += 1;
                sent_payload_bytes += event.payload_length();
            }
            WebSocketFrameDirection::Received => {
                received_frames += 1;
                received_payload_bytes += event.payload_length();
            }
        }
    }

    json!({
        "sockets": socket_ids.len(),
        "opens": opens,
        "errors": errors,
        "handshake_errors": handshake_errors,
        "runtime_errors": runtime_errors,
        "closing": closing,
        "closes": closes,
        "clean_closes": clean_closes,
        "unclean_closes": unclean_closes,
        "sent_frames": sent_frames,
        "received_frames": received_frames,
        "sent_payload_bytes": sent_payload_bytes,
        "received_payload_bytes": received_payload_bytes,
    })
}

#[cfg(test)]
pub(crate) fn render_http_error_network_trace(
    final_url: &url::Url,
    status: u16,
    headers: &[(String, String)],
    body: &str,
    config: Option<&NetworkTraceConfigSummary>,
) -> Value {
    let mut payload = Map::new();
    if let Some(config) = config {
        payload.insert("config".to_owned(), render_config_summary(config));
    }
    payload.insert(
        "main_document".to_owned(),
        render_main_document_trace_from_parts(
            final_url, final_url, status, headers, body, false, 0,
        ),
    );
    payload.insert("subresources".to_owned(), Value::Array(Vec::new()));
    payload.insert("websocket_frames".to_owned(), Value::Array(Vec::new()));
    payload.insert("websocket_lifecycle".to_owned(), Value::Array(Vec::new()));
    payload.insert(
        "websocket_summary".to_owned(),
        render_websocket_summary(&[], &[]),
    );
    Value::Object(payload)
}

fn render_config_summary(config: &NetworkTraceConfigSummary) -> Value {
    json!({
        "explicit_http_proxy": config.explicit_http_proxy,
        "libcurl_env_proxy_fallback": config.libcurl_env_proxy_fallback,
        "http_no_proxy": config.http_no_proxy,
        "proxy_bearer_token": config.proxy_bearer_token,
        "tls_verify_host": config.tls_verify_host,
        "obey_robots": config.obey_robots,
        "http_cache": config.http_cache,
        "connect_timeout_ms": config.connect_timeout_ms,
        "request_timeout_ms": config.request_timeout_ms,
        "max_concurrent": config.max_concurrent,
        "max_host_open": config.max_host_open,
        "max_host_connections": config.max_host_connections,
        "effective_max_host_connections": config.effective_max_host_connections,
        "max_total_connections": config.max_total_connections,
        "http2_max_concurrent_streams": config.http2_max_concurrent_streams,
        "max_response_size": config.max_response_size,
        "block_private_networks": config.block_private_networks,
        "block_cidr_count": config.block_cidr_count,
    })
}

fn render_main_document_trace_from_parts(
    requested_url: &url::Url,
    final_url: &url::Url,
    status: u16,
    headers: &[(String, String)],
    body: &str,
    redirected: bool,
    redirect_count: usize,
) -> Value {
    let mut payload = Map::new();
    payload.insert("url".to_owned(), json!(final_url.as_str()));
    payload.insert("requested_url".to_owned(), json!(requested_url.as_str()));
    payload.insert("status".to_owned(), json!(status));
    payload.insert(
        "content_type".to_owned(),
        json!(header_value(headers, "content-type")),
    );
    payload.insert("redirected".to_owned(), json!(redirected));
    payload.insert("redirect_count".to_owned(), json!(redirect_count));
    if let Some(summary) = json_response_summary(headers, body) {
        payload.insert("json_summary".to_owned(), summary);
    }
    if let Some(diagnostics) = render_success_response_diagnostics(
        requested_url,
        final_url,
        status,
        headers,
        body,
        redirect_count,
        &[],
        GateHintBodyMode::SkipBody,
    ) {
        payload.insert("diagnostics".to_owned(), diagnostics);
    }
    Value::Object(payload)
}

fn render_subresource_network_record(record: &SubresourceNetworkRecord) -> Value {
    render_subresource_network_record_with_body_option(record, false)
}

fn render_subresource_network_record_with_body_option(
    record: &SubresourceNetworkRecord,
    include_body_text: bool,
) -> Value {
    let mut payload = Map::new();
    payload.insert(
        "resource_type".to_owned(),
        json!(record.resource_type().as_cdp_type()),
    );
    payload.insert("method".to_owned(), json!(record.method()));
    payload.insert("url".to_owned(), json!(record.url().as_str()));
    payload.insert(
        "document_url".to_owned(),
        json!(record.document_url().as_str()),
    );
    if let Some(frame_id) = record.frame_id() {
        payload.insert("frame_id".to_owned(), json!(frame_id));
    }

    match record.outcome() {
        SubresourceNetworkOutcome::Success {
            final_url,
            status,
            redirect_chain,
            response_headers,
            response_body,
            ..
        } => {
            payload.insert("ok".to_owned(), json!(true));
            payload.insert("final_url".to_owned(), json!(final_url.as_str()));
            payload.insert("status".to_owned(), json!(status));
            payload.insert(
                "content_type".to_owned(),
                json!(header_value(response_headers, "content-type")),
            );
            payload.insert("body_length".to_owned(), json!(response_body.len()));
            // Trace diagnostics are textual hints. Keep exact protocol bytes in
            // the subresource carrier and derive the lossy view only here.
            let response_body_bytes = response_body.diagnostic_bytes();
            let response_body_text = String::from_utf8_lossy(response_body_bytes.as_ref());
            if include_body_text {
                payload.insert("body_text".to_owned(), json!(response_body_text.as_ref()));
            }
            if let Some(summary) =
                json_response_summary(response_headers, response_body_text.as_ref())
            {
                payload.insert("json_summary".to_owned(), summary);
            }
            if let Some(diagnostics) = render_success_response_diagnostics(
                record.url(),
                final_url,
                *status,
                response_headers,
                response_body_text.as_ref(),
                redirect_chain.len(),
                redirect_chain,
                GateHintBodyMode::IncludeBody,
            ) {
                payload.insert("diagnostics".to_owned(), diagnostics);
            }
        }
        SubresourceNetworkOutcome::Failure { error_text } => {
            payload.insert("ok".to_owned(), json!(false));
            payload.insert("error_text".to_owned(), json!(error_text));
            if let Some(diagnostics) = render_failure_response_diagnostics(error_text) {
                payload.insert("diagnostics".to_owned(), diagnostics);
            }
        }
    }
    if let Some(cookies) = render_subresource_cookie_summary(record) {
        payload.insert("cookies".to_owned(), cookies);
    }

    Value::Object(payload)
}

fn render_success_response_diagnostics(
    request_url: &url::Url,
    final_url: &url::Url,
    status: u16,
    response_headers: &[(String, String)],
    response_body: &str,
    redirect_count: usize,
    redirect_chain: &[NavigationRedirect],
    gate_hint_body_mode: GateHintBodyMode,
) -> Option<Value> {
    // These are trace hints for debugging network state, not navigation or fetch policy.
    let mut classifications = Vec::new();
    let mut details = Map::new();

    if status >= 400 {
        classifications.push("http_error");
    }

    let server_auth_schemes = auth_schemes(response_headers, "www-authenticate");
    if status == 401 || !server_auth_schemes.is_empty() {
        classifications.push("server_auth_challenge");
        if !server_auth_schemes.is_empty() {
            details.insert("server_auth_schemes".to_owned(), json!(server_auth_schemes));
        }
    }

    let proxy_auth_schemes = auth_schemes(response_headers, "proxy-authenticate");
    if status == 407 || !proxy_auth_schemes.is_empty() {
        classifications.push("proxy_auth_challenge");
        if !proxy_auth_schemes.is_empty() {
            details.insert("proxy_auth_schemes".to_owned(), json!(proxy_auth_schemes));
        }
    }

    if redirect_count > 0 {
        classifications.push("redirected");
        details.insert("redirect_count".to_owned(), json!(redirect_count));
    }

    let gate_reasons = gate_hint_reasons(
        request_url,
        final_url,
        response_headers,
        response_body,
        redirect_chain,
        gate_hint_body_mode,
    );
    if !gate_reasons.is_empty() {
        classifications.push("login_or_risk_gate");
        details.insert("gate_reasons".to_owned(), json!(gate_reasons));
    }

    if json_response_error_like(response_headers, response_body) {
        classifications.push("json_error_like");
    }

    render_diagnostics_payload(classifications, details)
}

fn render_failure_response_diagnostics(error_text: &str) -> Option<Value> {
    let mut classifications = vec!["network_error"];
    let details = Map::new();
    let lower = error_text.to_ascii_lowercase();
    if lower.contains("proxy") {
        classifications.push("proxy_error");
    }
    if lower.contains("timeout") || lower.contains("timed out") {
        classifications.push("timeout");
    }
    render_diagnostics_payload(classifications, details)
}

fn render_diagnostics_payload(
    mut classifications: Vec<&'static str>,
    details: Map<String, Value>,
) -> Option<Value> {
    classifications.sort_unstable();
    classifications.dedup();
    if classifications.is_empty() {
        return None;
    }

    let mut payload = Map::new();
    payload.insert("classifications".to_owned(), json!(classifications));
    for (key, value) in details {
        payload.insert(key, value);
    }
    Some(Value::Object(payload))
}

fn auth_schemes(headers: &[(String, String)], name: &str) -> Vec<String> {
    let mut schemes = headers
        .iter()
        .filter(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .filter_map(|(_, value)| value.split_whitespace().next())
        .filter(|scheme| !scheme.is_empty())
        .map(|scheme| scheme.trim_matches(',').to_ascii_lowercase())
        .collect::<Vec<_>>();
    schemes.sort();
    schemes.dedup();
    schemes
}

fn gate_hint_reasons(
    request_url: &url::Url,
    final_url: &url::Url,
    response_headers: &[(String, String)],
    response_body: &str,
    redirect_chain: &[NavigationRedirect],
    body_mode: GateHintBodyMode,
) -> Vec<&'static str> {
    let mut reasons = Vec::new();
    if final_url != request_url && url_has_login_or_risk_hint(final_url) {
        reasons.push("final_url");
    }
    if redirect_chain.iter().any(|redirect| {
        url_has_login_or_risk_hint(&redirect.from_url)
            || url_has_login_or_risk_hint(&redirect.to_url)
    }) {
        reasons.push("redirect_url");
    }
    if header_value(response_headers, "x-baxia-info").is_some()
        || header_value(response_headers, "x-baxia-status").is_some()
    {
        reasons.push("baxia_header");
    }
    if matches!(body_mode, GateHintBodyMode::IncludeBody)
        && response_body_has_login_or_risk_hint(response_body)
    {
        reasons.push("body_marker");
    }
    reasons.sort_unstable();
    reasons.dedup();
    reasons
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GateHintBodyMode {
    IncludeBody,
    SkipBody,
}

fn url_has_login_or_risk_hint(url: &url::Url) -> bool {
    let haystack = format!(
        "{}{}{}",
        url.host_str().unwrap_or_default(),
        url.path(),
        url.query().unwrap_or_default()
    )
    .to_ascii_lowercase();
    contains_login_or_risk_hint(&haystack)
}

fn response_body_has_login_or_risk_hint(body: &str) -> bool {
    let lower = body
        .chars()
        .take(4096)
        .collect::<String>()
        .to_ascii_lowercase();
    contains_login_or_risk_hint(&lower)
}

fn contains_login_or_risk_hint(haystack: &str) -> bool {
    [
        "login", "signin", "passport", "captcha", "verify", "security", "baxia", "risk",
    ]
    .iter()
    .any(|needle| haystack.contains(needle))
}

fn render_subresource_cookie_summary(record: &SubresourceNetworkRecord) -> Option<Value> {
    let request = record
        .request_cookie_report()
        .map(render_cookie_request_summary);
    let response = (!record.cookie_set_reports().is_empty())
        .then(|| render_cookie_response_summary(record.cookie_set_reports()));
    if request.is_none() && response.is_none() {
        return None;
    }

    let mut payload = Map::new();
    if let Some(request) = request {
        payload.insert("request".to_owned(), request);
    }
    if let Some(response) = response {
        payload.insert("response".to_owned(), response);
    }
    Some(Value::Object(payload))
}

fn render_cookie_request_summary(report: &StoredCookieQueryReport) -> Value {
    let mut payload = Map::new();
    payload.insert(
        "access_enabled".to_owned(),
        json!(report.facade_status.cookie_access_enabled),
    );
    payload.insert(
        "store_available".to_owned(),
        json!(report.facade_status.store_available),
    );
    payload.insert("included".to_owned(), json!(report.included_cookies.len()));
    payload.insert("excluded".to_owned(), json!(report.excluded_cookies.len()));

    let blocked_reasons = debug_reason_names(&report.facade_status.blocked_reasons);
    if !blocked_reasons.is_empty() {
        payload.insert("blocked_reasons".to_owned(), json!(blocked_reasons));
    }
    let facade_exclusion_reasons = debug_reason_names(&report.facade_exclusion_reasons);
    if !facade_exclusion_reasons.is_empty() {
        payload.insert(
            "facade_exclusion_reasons".to_owned(),
            json!(facade_exclusion_reasons),
        );
    }
    let excluded_reasons = debug_reason_names(
        report
            .excluded_cookies
            .iter()
            .flat_map(|access| access.exclusion_reasons.iter()),
    );
    if !excluded_reasons.is_empty() {
        payload.insert("excluded_reasons".to_owned(), json!(excluded_reasons));
    }
    let warning_reasons = debug_reason_names(
        report
            .included_cookies
            .iter()
            .chain(report.excluded_cookies.iter())
            .flat_map(|access| access.warning_reasons.iter()),
    );
    if !warning_reasons.is_empty() {
        payload.insert("warning_reasons".to_owned(), json!(warning_reasons));
    }

    Value::Object(payload)
}

fn render_cookie_response_summary(reports: &[StoredCookieSetReport]) -> Value {
    let mut payload = Map::new();
    payload.insert(
        "accepted".to_owned(),
        json!(reports.iter().filter(|report| report.is_accepted()).count()),
    );
    payload.insert(
        "rejected".to_owned(),
        json!(
            reports
                .iter()
                .filter(|report| !report.is_accepted())
                .count()
        ),
    );

    let rejection_reasons = debug_reason_names(
        reports
            .iter()
            .flat_map(|report| report.rejection_reasons.iter()),
    );
    if !rejection_reasons.is_empty() {
        payload.insert("rejection_reasons".to_owned(), json!(rejection_reasons));
    }
    let warning_reasons = debug_reason_names(
        reports
            .iter()
            .flat_map(|report| report.warning_reasons.iter()),
    );
    if !warning_reasons.is_empty() {
        payload.insert("warning_reasons".to_owned(), json!(warning_reasons));
    }

    Value::Object(payload)
}

fn debug_reason_names<'a, T, I>(reasons: I) -> Vec<String>
where
    T: std::fmt::Debug + 'a,
    I: IntoIterator<Item = &'a T>,
{
    let mut names = reasons
        .into_iter()
        .map(|reason| format!("{reason:?}"))
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn json_response_summary(headers: &[(String, String)], body: &str) -> Option<Value> {
    let content_type = header_value(headers, "content-type")?;
    if !content_type
        .split(';')
        .next()
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
    {
        return None;
    }

    let parsed: Value = serde_json::from_str(body).ok()?;
    let object = parsed.as_object()?;
    let mut summary = Map::new();
    for key in ["api", "ret"] {
        if let Some(value) = object.get(key) {
            summary.insert(key.to_owned(), value.clone());
        }
    }
    for key in ["success", "code", "error", "message", "msg"] {
        if let Some(value) = object.get(key).and_then(json_summary_scalar) {
            summary.insert(key.to_owned(), value);
        }
    }
    if let Some(url) = object
        .get("data")
        .and_then(Value::as_object)
        .and_then(|data| data.get("url"))
    {
        summary.insert("data.url".to_owned(), url.clone());
    }
    (!summary.is_empty()).then_some(Value::Object(summary))
}

fn json_summary_scalar(value: &Value) -> Option<Value> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Some(value.clone()),
        Value::String(text) => Some(json!(clamp_diagnostic_string(text))),
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn clamp_diagnostic_string(text: &str) -> String {
    const MAX_LEN: usize = 240;
    let mut chars = text.chars();
    let mut output = String::new();
    for ch in chars.by_ref().take(MAX_LEN) {
        output.push(ch);
    }
    if chars.next().is_some() {
        output.push_str("...");
    }
    output
}

fn json_response_error_like(headers: &[(String, String)], body: &str) -> bool {
    let Some(content_type) = header_value(headers, "content-type") else {
        return false;
    };
    if !content_type
        .split(';')
        .next()
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
    {
        return false;
    }

    let Ok(parsed) = serde_json::from_str::<Value>(body) else {
        return false;
    };
    let Some(object) = parsed.as_object() else {
        return false;
    };

    if object.get("success").and_then(Value::as_bool) == Some(false) {
        return true;
    }
    if object.get("error").is_some_and(json_error_value_present) {
        return true;
    }
    if object.get("code").is_some_and(json_error_code_like) {
        return true;
    }
    if let Some(ret) = object.get("ret") {
        return json_ret_error_like(ret);
    }
    false
}

fn json_error_value_present(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_i64().unwrap_or(1) != 0,
        Value::String(text) => {
            let normalized = text.trim();
            !normalized.is_empty()
                && !["0", "ok", "success", "false"]
                    .contains(&normalized.to_ascii_lowercase().as_str())
        }
        Value::Array(items) => !items.is_empty(),
        Value::Object(object) => !object.is_empty(),
    }
}

fn json_error_code_like(value: &Value) -> bool {
    match value {
        Value::Number(number) => number.as_i64().is_some_and(|code| code != 0),
        Value::String(text) => {
            let normalized = text.trim().to_ascii_lowercase();
            !normalized.is_empty() && !["0", "ok", "success"].contains(&normalized.as_str())
        }
        _ => false,
    }
}

fn json_ret_error_like(value: &Value) -> bool {
    match value {
        Value::String(text) => {
            let normalized = text.to_ascii_lowercase();
            normalized.contains("fail")
                || normalized.contains("error")
                || normalized.contains("deny")
        }
        Value::Array(items) => items.iter().any(json_ret_error_like),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_core::page::{
        SubresourceNetworkRecord, SubresourceResourceType, WebSocketFrameDirection,
        WebSocketFrameOpcode, WebSocketLifecycleEvent, WebSocketNetworkEvent,
    };

    #[test]
    fn clamp_diagnostic_string_truncates_without_splitting_chars() {
        assert_eq!(clamp_diagnostic_string("short"), "short");

        let exact = "界".repeat(240);
        assert_eq!(clamp_diagnostic_string(&exact), exact);

        let long = format!("{}尾", "界".repeat(240));
        assert_eq!(
            clamp_diagnostic_string(&long),
            format!("{}...", "界".repeat(240))
        );
    }

    #[test]
    fn subresource_trace_diagnostics_classify_auth_challenges() -> anyhow::Result<()> {
        let record = SubresourceNetworkRecord::success(
            None,
            "http://example.test/page".parse()?,
            "http://example.test/api".parse()?,
            "GET".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::Fetch,
            None,
            Vec::new(),
            "http://example.test/api".parse()?,
            401,
            vec![(
                "WWW-Authenticate".to_owned(),
                "Basic realm=\"private\"".to_owned(),
            )],
            "auth required".to_owned(),
            Vec::new(),
        );

        let rendered = render_subresource_network_record(&record);

        assert_eq!(rendered["diagnostics"]["classifications"][0], "http_error");
        assert!(
            rendered["diagnostics"]["classifications"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "server_auth_challenge")
        );
        assert_eq!(rendered["diagnostics"]["server_auth_schemes"][0], "basic");
        Ok(())
    }

    #[test]
    fn subresource_trace_diagnostics_classify_proxy_failures() {
        let record = SubresourceNetworkRecord::failure(
            None,
            "http://example.test/page".parse().unwrap(),
            "http://example.test/api".parse().unwrap(),
            "GET".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::Xhr,
            "proxy connection timed out".to_owned(),
        );

        let rendered = render_subresource_network_record(&record);

        let classifications = rendered["diagnostics"]["classifications"]
            .as_array()
            .unwrap();
        assert!(classifications.iter().any(|value| value == "network_error"));
        assert!(classifications.iter().any(|value| value == "proxy_error"));
        assert!(classifications.iter().any(|value| value == "timeout"));
    }

    #[test]
    fn subresource_trace_renders_websocket_handshake_records() -> anyhow::Result<()> {
        let record = SubresourceNetworkRecord::success(
            None,
            "https://example.test/page".parse()?,
            "wss://example.test/socket".parse()?,
            "GET".to_owned(),
            vec![("origin".to_owned(), "https://example.test".to_owned())],
            None,
            SubresourceResourceType::WebSocket,
            None,
            Vec::new(),
            "wss://example.test/socket".parse()?,
            101,
            vec![("sec-websocket-accept".to_owned(), "accept-token".to_owned())],
            String::new(),
            Vec::new(),
        );

        let rendered = render_subresource_network_record(&record);

        assert_eq!(rendered["resource_type"], "WebSocket");
        assert_eq!(rendered["method"], "GET");
        assert_eq!(rendered["ok"], true);
        assert_eq!(rendered["status"], 101);
        assert_eq!(rendered["body_length"], 0);
        Ok(())
    }

    #[test]
    fn websocket_trace_renders_frame_metadata_without_payload() -> anyhow::Result<()> {
        let event = WebSocketNetworkEvent::new(
            7,
            "https://example.test/page".parse()?,
            "wss://example.test/socket".parse()?,
            WebSocketFrameDirection::Received,
            WebSocketFrameOpcode::Binary,
            42,
        );

        let rendered = render_websocket_network_event(&event);

        assert_eq!(rendered["url"], "wss://example.test/socket");
        assert_eq!(rendered["document_url"], "https://example.test/page");
        assert_eq!(rendered["direction"], "received");
        assert_eq!(rendered["opcode"], "binary");
        assert_eq!(rendered["payload_length"], 42);
        assert!(rendered.get("payload").is_none());
        Ok(())
    }

    #[test]
    fn websocket_trace_renders_lifecycle_events_and_summary() -> anyhow::Result<()> {
        let document_url = "https://example.test/page".parse()?;
        let socket_url = "wss://example.test/socket".parse()?;
        let lifecycle_events = vec![
            WebSocketLifecycleEvent::open(7, document_url, socket_url),
            WebSocketLifecycleEvent::closing(
                7,
                "https://example.test/page".parse()?,
                "wss://example.test/socket".parse()?,
            ),
            WebSocketLifecycleEvent::close(
                7,
                "https://example.test/page".parse()?,
                "wss://example.test/socket".parse()?,
                1000,
                "done".to_owned(),
                true,
            ),
            WebSocketLifecycleEvent::error(
                8,
                "https://example.test/page".parse()?,
                "wss://example.test/fail".parse()?,
                "handshake failed".to_owned(),
            ),
            WebSocketLifecycleEvent::close(
                8,
                "https://example.test/page".parse()?,
                "wss://example.test/fail".parse()?,
                1006,
                String::new(),
                false,
            ),
        ];
        let frame_events = vec![
            WebSocketNetworkEvent::new(
                7,
                "https://example.test/page".parse()?,
                "wss://example.test/socket".parse()?,
                WebSocketFrameDirection::Sent,
                WebSocketFrameOpcode::Text,
                4,
            ),
            WebSocketNetworkEvent::new(
                7,
                "https://example.test/page".parse()?,
                "wss://example.test/socket".parse()?,
                WebSocketFrameDirection::Received,
                WebSocketFrameOpcode::Binary,
                6,
            ),
        ];

        let rendered_event = render_websocket_lifecycle_event(&lifecycle_events[2]);
        assert_eq!(rendered_event["type"], "close");
        assert_eq!(rendered_event["code"], 1000);
        assert_eq!(rendered_event["reason"], "done");
        assert_eq!(rendered_event["was_clean"], true);

        let summary = render_websocket_summary(&lifecycle_events, &frame_events);
        assert_eq!(summary["sockets"], 2);
        assert_eq!(summary["opens"], 1);
        assert_eq!(summary["errors"], 1);
        assert_eq!(summary["handshake_errors"], 1);
        assert_eq!(summary["runtime_errors"], 0);
        assert_eq!(summary["closing"], 1);
        assert_eq!(summary["closes"], 2);
        assert_eq!(summary["clean_closes"], 1);
        assert_eq!(summary["unclean_closes"], 1);
        assert_eq!(summary["sent_frames"], 1);
        assert_eq!(summary["received_frames"], 1);
        assert_eq!(summary["sent_payload_bytes"], 4);
        assert_eq!(summary["received_payload_bytes"], 6);
        Ok(())
    }

    #[test]
    fn subresource_trace_diagnostics_classify_login_gate_and_json_error() -> anyhow::Result<()> {
        let redirect_chain = vec![NavigationRedirect {
            from_url: "http://example.test/api/detail".parse()?,
            to_url: "http://login.example.test/security/baxia".parse()?,
            status: 302,
            headers: Vec::new(),
            network_extra_info_available: true,
            request_extra_info: None,
            response_extra_info: None,
            redirect_has_extra_info: true,
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        }];
        let record = SubresourceNetworkRecord::success(
            None,
            "http://example.test/page".parse()?,
            "http://example.test/api/detail".parse()?,
            "GET".to_owned(),
            Vec::new(),
            None,
            SubresourceResourceType::Fetch,
            None,
            redirect_chain,
            "http://login.example.test/security/baxia".parse()?,
            200,
            vec![
                ("content-type".to_owned(), "application/json".to_owned()),
                ("x-baxia-info".to_owned(), "challenge".to_owned()),
            ],
            r#"{"success":false,"code":"AUTH_REQUIRED","message":"login required","ret":["FAIL_SYS_USER_VALIDATE"]}"#.to_owned(),
            Vec::new(),
        );

        let rendered = render_subresource_network_record(&record);
        let classifications = rendered["diagnostics"]["classifications"]
            .as_array()
            .unwrap();

        assert!(
            classifications
                .iter()
                .any(|value| value == "json_error_like")
        );
        assert!(
            classifications
                .iter()
                .any(|value| value == "login_or_risk_gate")
        );
        assert!(classifications.iter().any(|value| value == "redirected"));
        assert_eq!(rendered["diagnostics"]["redirect_count"], 1);
        assert_eq!(rendered["json_summary"]["code"], "AUTH_REQUIRED");
        assert_eq!(rendered["json_summary"]["success"], false);
        Ok(())
    }

    #[test]
    fn main_document_trace_reports_redirect_gate_without_body_marker() -> anyhow::Result<()> {
        let requested_url = "http://example.test/article/1".parse()?;
        let final_url = "http://login.example.test/passport".parse()?;
        let rendered = render_main_document_trace_from_parts(
            &requested_url,
            &final_url,
            200,
            &[("content-type".to_owned(), "text/html".to_owned())],
            "<html><body>login keyword in body should not drive main document diagnostics</body></html>",
            true,
            1,
        );

        assert_eq!(rendered["requested_url"], "http://example.test/article/1");
        assert_eq!(rendered["redirected"], true);
        assert_eq!(rendered["redirect_count"], 1);
        let classifications = rendered["diagnostics"]["classifications"]
            .as_array()
            .unwrap();
        assert!(classifications.iter().any(|value| value == "redirected"));
        assert!(
            classifications
                .iter()
                .any(|value| value == "login_or_risk_gate")
        );
        let gate_reasons = rendered["diagnostics"]["gate_reasons"].as_array().unwrap();
        assert!(gate_reasons.iter().any(|value| value == "final_url"));
        assert!(!gate_reasons.iter().any(|value| value == "body_marker"));
        Ok(())
    }

    #[test]
    fn trace_config_summary_reports_proxy_state_without_sensitive_values() {
        let summary = NetworkTraceConfigSummary {
            explicit_http_proxy: true,
            libcurl_env_proxy_fallback: false,
            http_no_proxy: true,
            proxy_bearer_token: true,
            tls_verify_host: false,
            obey_robots: true,
            http_cache: true,
            connect_timeout_ms: Some(1500),
            request_timeout_ms: 3000,
            max_concurrent: Some(8),
            max_host_open: Some(2),
            max_host_connections: Some(4),
            effective_max_host_connections: Some(4),
            max_total_connections: Some(64),
            http2_max_concurrent_streams: Some(100),
            max_response_size: Some(4096),
            block_private_networks: true,
            block_cidr_count: 3,
        };

        let rendered = render_config_summary(&summary);

        assert_eq!(rendered["explicit_http_proxy"], true);
        assert_eq!(rendered["libcurl_env_proxy_fallback"], false);
        assert_eq!(rendered["http_no_proxy"], true);
        assert_eq!(rendered["proxy_bearer_token"], true);
        assert_eq!(rendered["tls_verify_host"], false);
        assert_eq!(rendered["connect_timeout_ms"], 1500);
        assert_eq!(rendered["max_host_connections"], 4);
        assert_eq!(rendered["effective_max_host_connections"], 4);
        assert_eq!(rendered["max_total_connections"], 64);
        assert_eq!(rendered["http2_max_concurrent_streams"], 100);
        let serialized = rendered.to_string();
        assert!(!serialized.contains("http://proxy.example"));
        assert!(!serialized.contains("secret"));
    }

    #[test]
    fn http_error_network_trace_includes_config_and_auth_diagnostics() -> anyhow::Result<()> {
        let config = NetworkTraceConfigSummary {
            explicit_http_proxy: false,
            libcurl_env_proxy_fallback: true,
            http_no_proxy: false,
            proxy_bearer_token: false,
            tls_verify_host: true,
            obey_robots: false,
            http_cache: false,
            connect_timeout_ms: None,
            request_timeout_ms: 30_000,
            max_concurrent: None,
            max_host_open: None,
            max_host_connections: None,
            effective_max_host_connections: Some(6),
            max_total_connections: None,
            http2_max_concurrent_streams: None,
            max_response_size: None,
            block_private_networks: false,
            block_cidr_count: 0,
        };
        let trace = render_http_error_network_trace(
            &"http://example.test/private".parse()?,
            401,
            &[(
                "www-authenticate".to_owned(),
                "Bearer realm=\"api\"".to_owned(),
            )],
            "unauthorized",
            Some(&config),
        );

        assert_eq!(trace["config"]["libcurl_env_proxy_fallback"], true);
        assert_eq!(trace["main_document"]["status"], 401);
        assert_eq!(
            trace["main_document"]["diagnostics"]["server_auth_schemes"][0],
            "bearer"
        );
        Ok(())
    }
}
