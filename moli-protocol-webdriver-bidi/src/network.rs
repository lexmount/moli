use std::collections::{BTreeMap, BTreeSet, VecDeque};

use moli_protocol::devtools_runtime::{
    AutomationEvent, DevToolsNetworkInterceptId, NetworkRedirectResponseEvent, NetworkRequestEvent,
    webdriver_bidi_navigation_id_from_loader_id,
};
use moli_protocol::domains::network::{
    cdp_cookie_query_report, cdp_request_headers_object, fetch_auth_required_params,
    fetch_request_paused_params, http_status_text,
};
use serde_json::{Value, json};
use url::Url;

use crate::events::{bidi_timestamp_millis, non_empty_json_string};
use crate::storage::bidi_cookie_from_cdp_cookie;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct BidiNetworkEventState {
    requests: BTreeMap<String, BidiNetworkRequestState>,
    completed_request_ids: BTreeSet<String>,
    completed_request_order: VecDeque<String>,
}

const COMPLETED_REQUEST_ID_LIMIT: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
struct BidiNetworkRequestState {
    context: Option<String>,
    navigation: Option<String>,
    redirect_count: u64,
    request: Value,
    response: Option<Value>,
}

impl BidiNetworkEventState {
    pub(crate) fn events_from_protocol_message(&mut self, message: &Value) -> Vec<Value> {
        let params = &message["params"];
        match message.get("method").and_then(Value::as_str) {
            Some("Network.requestWillBeSent") => self.before_request_sent_from_cdp_params(
                params,
                &blocked_intercepts_from_cdp_params(params),
            ),
            Some("Network.responseReceived") => {
                option_event_vec(self.response_started_from_cdp_params(
                    params,
                    &blocked_intercepts_from_cdp_params(params),
                ))
            }
            Some("Network.loadingFinished") => {
                option_event_vec(self.response_completed_from_cdp_params(params))
            }
            Some("Network.loadingFailed") => {
                option_event_vec(self.fetch_error_from_cdp_params(params))
            }
            Some("Fetch.requestPaused") => {
                option_event_vec(self.response_started_from_fetch_paused_params(
                    params,
                    &blocked_intercepts_from_cdp_params(params),
                ))
            }
            Some("Fetch.authRequired") => option_event_vec(self.auth_required_from_cdp_params(
                params,
                &blocked_intercepts_from_cdp_params(params),
            )),
            _ => Vec::new(),
        }
    }

    pub(crate) fn events_from_automation_event(&mut self, event: &AutomationEvent) -> Vec<Value> {
        match event {
            AutomationEvent::NetworkBeforeRequestSent(event) => self
                .before_request_sent_from_cdp_params(
                    &network_request_event_cdp_params(event, "Network.requestWillBeSent"),
                    &event.blocked_intercepts,
                ),
            AutomationEvent::NetworkResponseStarted(event) => {
                option_event_vec(self.response_started_from_cdp_params(
                    &network_request_event_cdp_params(event, "Network.responseReceived"),
                    &event.blocked_intercepts,
                ))
            }
            AutomationEvent::NetworkResponseCompleted(event) => {
                option_event_vec(self.response_completed_from_cdp_params(
                    &network_request_event_cdp_params(event, "Network.loadingFinished"),
                ))
            }
            AutomationEvent::NetworkFetchError(event) => {
                option_event_vec(self.fetch_error_from_cdp_params(
                    &network_request_event_cdp_params(event, "Network.loadingFailed"),
                ))
            }
            AutomationEvent::NetworkAuthRequired(event) => {
                option_event_vec(self.auth_required_from_cdp_params(
                    &network_request_event_cdp_params(event, "Fetch.authRequired"),
                    &event.blocked_intercepts,
                ))
            }
            AutomationEvent::RequestPaused(event) => {
                option_event_vec(self.response_started_from_fetch_paused_params(
                    &network_request_event_cdp_params(event, "Fetch.requestPaused"),
                    &event.blocked_intercepts,
                ))
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn forget_context(&mut self, context: &str) {
        self.requests
            .retain(|_, request| request.context.as_deref() != Some(context));
    }

    fn before_request_sent_from_cdp_params(
        &mut self,
        params: &Value,
        blocked_intercepts: &[DevToolsNetworkInterceptId],
    ) -> Vec<Value> {
        let Some(request_id) = non_empty_json_string(&params["requestId"]) else {
            return Vec::new();
        };
        self.forget_completed_request_id(&request_id);
        let redirect_response = params.get("redirectResponse").filter(|response| {
            response
                .get("url")
                .and_then(Value::as_str)
                .is_some_and(|url| !url.is_empty())
        });
        let redirected_state = redirect_response
            .is_some()
            .then(|| self.requests.remove(&request_id))
            .flatten();
        let previous_state = redirect_response
            .is_none()
            .then(|| self.requests.remove(&request_id))
            .flatten();
        let previous_redirect_count = redirected_state
            .as_ref()
            .map(|request| request.redirect_count)
            .unwrap_or(0);
        let redirect_count = if redirect_response.is_some() {
            redirected_state
                .as_ref()
                .map(|request| request.redirect_count.saturating_add(1))
                .unwrap_or(1)
        } else {
            0
        };
        let mut events = redirect_response
            .map(|_| {
                let state = redirected_state.unwrap_or_else(|| {
                    synthesized_bidi_network_redirect_request_state(
                        &request_id,
                        params,
                        previous_redirect_count,
                    )
                });
                Self::redirect_response_events_from_state(params, state)
            })
            .unwrap_or_default();
        let state = BidiNetworkRequestState {
            context: non_empty_json_string(&params["frameId"]).or_else(|| {
                previous_state
                    .as_ref()
                    .and_then(|state| state.context.clone())
            }),
            navigation: bidi_network_navigation_id_from_cdp_params(params).or_else(|| {
                previous_state
                    .as_ref()
                    .and_then(|state| state.navigation.clone())
            }),
            redirect_count,
            request: bidi_network_request_data_from_cdp_request_params(params, &request_id),
            response: previous_state.and_then(|state| state.response),
        };
        self.requests.insert(request_id, state.clone());

        let mut event = bidi_network_event(
            "network.beforeRequestSent",
            &state,
            bidi_network_timestamp_millis_from_cdp_params(params),
            blocked_intercepts,
        );
        if let Some(initiator) = bidi_network_initiator_from_cdp_params(params)
            && let Some(event_params) = event["params"].as_object_mut()
        {
            event_params.insert("initiator".to_owned(), initiator);
        }
        events.push(event);
        events
    }

    fn redirect_response_events_from_state(
        params: &Value,
        mut state: BidiNetworkRequestState,
    ) -> Vec<Value> {
        let response = bidi_network_response_data_from_cdp_redirect_response_params(params);
        state.response = Some(response.clone());
        let timestamp = bidi_network_timestamp_millis_from_cdp_params(params);
        vec![
            bidi_network_event_with_response(
                "network.responseStarted",
                &state,
                timestamp,
                response.clone(),
                &[],
            ),
            bidi_network_event_with_response(
                "network.responseCompleted",
                &state,
                timestamp,
                bidi_network_response_with_encoded_length(response, Some(0)),
                &[],
            ),
        ]
    }

    fn response_started_from_cdp_params(
        &mut self,
        params: &Value,
        blocked_intercepts: &[DevToolsNetworkInterceptId],
    ) -> Option<Value> {
        let request_id = non_empty_json_string(&params["requestId"])?;
        if self.completed_request_ids.contains(&request_id) {
            return None;
        }
        if self
            .requests
            .get(&request_id)
            .is_some_and(|state| state.response.is_some())
        {
            return None;
        }
        let response = bidi_network_response_data_from_cdp_response_params(params);
        let mut state = self
            .requests
            .remove(&request_id)
            .unwrap_or_else(|| synthesized_bidi_network_request_state(&request_id, params));
        if state.context.is_none() {
            state.context = non_empty_json_string(&params["frameId"]);
        }
        if state.navigation.is_none() {
            state.navigation = bidi_network_navigation_id_from_cdp_params(params);
        }
        apply_fetch_request_id_override(params, &mut state.request);
        state.response = Some(response.clone());
        self.requests.insert(request_id, state.clone());

        Some(bidi_network_event_with_response(
            "network.responseStarted",
            &state,
            bidi_network_timestamp_millis_from_cdp_params(params),
            response,
            blocked_intercepts,
        ))
    }

    fn response_started_from_fetch_paused_params(
        &mut self,
        params: &Value,
        blocked_intercepts: &[DevToolsNetworkInterceptId],
    ) -> Option<Value> {
        if blocked_intercepts.is_empty() || params.get("responseStatusCode").is_none() {
            return None;
        }
        let network_request_id = non_empty_json_string(&params["networkId"])?;
        if self.completed_request_ids.contains(&network_request_id) {
            return None;
        }
        let fetch_request_id = non_empty_json_string(&params["requestId"])?;
        let response = bidi_network_response_data_from_fetch_paused_params(params);
        let mut state = self
            .requests
            .remove(&network_request_id)
            .unwrap_or_else(|| BidiNetworkRequestState {
                context: non_empty_json_string(&params["frameId"]),
                navigation: None,
                redirect_count: 0,
                request: bidi_network_request_data_from_cdp_request_params(
                    params,
                    &fetch_request_id,
                ),
                response: None,
            });
        if state.context.is_none() {
            state.context = non_empty_json_string(&params["frameId"]);
        }
        if state.navigation.is_none() {
            state.navigation = bidi_network_navigation_id_from_cdp_params(params);
        }
        apply_fetch_request_id(&mut state.request, &fetch_request_id);
        state.response = Some(response.clone());
        self.requests.insert(network_request_id, state.clone());

        Some(bidi_network_event_with_response(
            "network.responseStarted",
            &state,
            bidi_network_timestamp_millis_from_cdp_params(params),
            response,
            blocked_intercepts,
        ))
    }

    fn response_completed_from_cdp_params(&mut self, params: &Value) -> Option<Value> {
        let request_id = non_empty_json_string(&params["requestId"])?;
        let state = self.requests.remove(&request_id)?;
        let response = state.response.clone()?;
        self.remember_completed_request_id(request_id);
        let response = bidi_network_response_with_encoded_length(
            response,
            json_number_as_u64(&params["encodedDataLength"]),
        );
        Some(bidi_network_event_with_response(
            "network.responseCompleted",
            &state,
            bidi_network_timestamp_millis_from_cdp_params(params),
            response,
            &[],
        ))
    }

    fn fetch_error_from_cdp_params(&mut self, params: &Value) -> Option<Value> {
        let request_id = non_empty_json_string(&params["requestId"])?;
        let state = self
            .requests
            .remove(&request_id)
            .unwrap_or_else(|| synthesized_bidi_network_request_state(&request_id, params));
        self.remember_completed_request_id(request_id);
        let mut event = bidi_network_event(
            "network.fetchError",
            &state,
            bidi_network_timestamp_millis_from_cdp_params(params),
            &[],
        );
        if let Some(event_params) = event["params"].as_object_mut() {
            event_params.insert(
                "errorText".to_owned(),
                json!(
                    params
                        .get("errorText")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                ),
            );
        }
        Some(event)
    }

    fn auth_required_from_cdp_params(
        &mut self,
        params: &Value,
        blocked_intercepts: &[DevToolsNetworkInterceptId],
    ) -> Option<Value> {
        let request_id = non_empty_json_string(&params["requestId"])?;
        let response = bidi_network_response_data_from_auth_required_params(params);
        let mut state =
            self.requests
                .remove(&request_id)
                .unwrap_or_else(|| BidiNetworkRequestState {
                    context: non_empty_json_string(&params["frameId"]),
                    navigation: bidi_network_navigation_id_from_cdp_params(params),
                    redirect_count: 0,
                    request: bidi_network_request_data_from_cdp_request_params(params, &request_id),
                    response: None,
                });
        if state.context.is_none() {
            state.context = non_empty_json_string(&params["frameId"]);
        }
        if state.navigation.is_none() {
            state.navigation = bidi_network_navigation_id_from_cdp_params(params);
        }
        state.response = Some(response.clone());
        self.requests.insert(request_id, state.clone());

        Some(bidi_network_event_with_response(
            "network.authRequired",
            &state,
            bidi_network_timestamp_millis_from_cdp_params(params),
            response,
            blocked_intercepts,
        ))
    }

    fn remember_completed_request_id(&mut self, request_id: String) {
        if !self.completed_request_ids.insert(request_id.clone()) {
            return;
        }
        self.completed_request_order.push_back(request_id);
        while self.completed_request_order.len() > COMPLETED_REQUEST_ID_LIMIT {
            if let Some(expired) = self.completed_request_order.pop_front() {
                self.completed_request_ids.remove(&expired);
            }
        }
    }

    fn forget_completed_request_id(&mut self, request_id: &str) {
        if !self.completed_request_ids.remove(request_id) {
            return;
        }
        self.completed_request_order
            .retain(|completed| completed != request_id);
    }
}

fn option_event_vec(event: Option<Value>) -> Vec<Value> {
    event.into_iter().collect()
}

fn network_request_event_cdp_params(event: &NetworkRequestEvent, method: &str) -> Value {
    match method {
        "Network.requestWillBeSent" => {
            let mut params = json!({
                "requestId": event.request_id.as_str(),
                "loaderId": event.loader_id.as_ref().map(|id| id.as_str()).unwrap_or_default(),
                "documentURL": event.document_url.as_deref().unwrap_or(event.url.as_str()),
                "request": {
                    "url": event.url,
                    "method": event.method.as_deref().unwrap_or_default(),
                    "headers": cdp_request_headers_object(
                        &event.request_headers,
                        event.request_cookie_report.as_ref(),
                    ),
                    "hasPostData": event.request_body.is_some(),
                },
                "timestamp": event.timestamp.unwrap_or_default(),
                "wallTime": event.wall_time.unwrap_or_else(|| event.timestamp.unwrap_or_default()),
                "initiator": {
                    "type": event.request_initiator_type.as_deref().unwrap_or("other")
                },
                "redirectHasExtraInfo": event.redirect_has_extra_info,
                "type": event
                    .resource_type
                    .map(|resource_type| resource_type.as_cdp_type())
                    .unwrap_or_default(),
                "frameId": event.frame_id.as_ref().map(|id| id.as_str()).unwrap_or_default(),
                "hasUserGesture": false,
            });
            if let Some(request_body) = event.request_body.as_ref() {
                params["request"]["postData"] = json!(request_body);
            }
            if let Some(redirect_response) = event.redirect_response.as_ref() {
                params["redirectResponse"] = network_redirect_response_cdp_value(redirect_response);
            }
            if let Some(cookie_access_report) = event.request_cookie_report.as_ref() {
                params["cookieAccessReport"] = cdp_cookie_query_report(cookie_access_report);
            }
            if let Some(bidi_request_initiator_type) = event.bidi_request_initiator_type.as_ref() {
                params["__moliRequestInitiatorType"] = json!(bidi_request_initiator_type);
            }
            if let Some(fetch_request_id) = event.fetch_request_id.as_ref() {
                params["__moliFetchRequestId"] = json!(fetch_request_id.as_str());
            }
            params
        }
        "Network.responseReceived" => {
            let response_mime_type = event
                .response_mime_type
                .clone()
                .or_else(|| response_header_value(&event.response_headers, "content-type"))
                .unwrap_or_default();
            let response_protocol = event
                .response_protocol
                .clone()
                .unwrap_or_else(|| response_protocol_for_url(&event.url));
            json!({
                "requestId": event.request_id.as_str(),
                "loaderId": event.loader_id.as_ref().map(|id| id.as_str()).unwrap_or_default(),
                "timestamp": event.timestamp.unwrap_or_default(),
                "type": event
                    .resource_type
                    .map(|resource_type| resource_type.as_cdp_type())
                    .unwrap_or_default(),
                "frameId": event.frame_id.as_ref().map(|id| id.as_str()).unwrap_or_default(),
                "response": {
                    "url": event.url,
                    "status": event.status.unwrap_or_default(),
                    "statusText": event
                        .status_text
                        .as_deref()
                        .unwrap_or_else(|| http_status_text(event.status.unwrap_or_default())),
                    "headers": json_object_from_header_pairs(&event.response_headers),
                    "mimeType": response_mime_type,
                    "encodedDataLength": event.encoded_data_length.unwrap_or_default(),
                    "protocol": response_protocol,
                    "fromDiskCache": event.from_cache,
                    "fromPrefetchCache": false,
                },
            })
        }
        "Network.loadingFinished" => json!({
            "requestId": event.request_id.as_str(),
            "timestamp": event.timestamp.unwrap_or_default(),
            "encodedDataLength": event.encoded_data_length.unwrap_or_default(),
        }),
        "Network.loadingFailed" => json!({
            "requestId": event.request_id.as_str(),
            "timestamp": event.timestamp.unwrap_or_default(),
            "type": event
                .resource_type
                .map(|resource_type| resource_type.as_cdp_type())
                .unwrap_or_default(),
            "errorText": event.error_text.as_deref().unwrap_or_default(),
            "canceled": event.loading_failed_canceled,
        }),
        "Fetch.authRequired" => fetch_auth_required_params(event),
        "Fetch.requestPaused" => fetch_request_paused_params(event),
        _ => Value::Null,
    }
}

fn json_object_from_header_pairs(headers: &[(String, Vec<u8>)]) -> serde_json::Map<String, Value> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.clone(),
                json!(moli_header_field::decode_header_value(value)),
            )
        })
        .collect()
}

fn response_header_value(headers: &[(String, Vec<u8>)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| moli_header_field::decode_header_value(value).into_owned())
}

fn response_protocol_for_url(url: &str) -> String {
    Url::parse(url)
        .ok()
        .map(|url| match url.scheme() {
            "http" | "https" => "http/1.1".to_owned(),
            scheme => scheme.to_owned(),
        })
        .unwrap_or_default()
}

fn network_redirect_response_cdp_value(response: &NetworkRedirectResponseEvent) -> Value {
    json!({
        "url": response.url,
        "status": response.status,
        "statusText": response
            .status_text
            .as_deref()
            .unwrap_or_else(|| http_status_text(response.status)),
        "headers": json_object_from_header_pairs(&response.response_headers),
        "mimeType": response_header_value(&response.response_headers, "content-type")
            .unwrap_or_default(),
        "connectionReused": false,
        "connectionId": 0,
        "encodedDataLength": response.encoded_data_length,
        "fromDiskCache": response.from_cache,
        "securityState": "secure",
        "protocol": response_protocol_for_url(&response.url),
    })
}

fn bidi_network_event(
    method: &str,
    state: &BidiNetworkRequestState,
    timestamp: u64,
    blocked_intercepts: &[DevToolsNetworkInterceptId],
) -> Value {
    let mut event = json!({
        "type": "event",
        "method": method,
        "params": {
            "context": state
                .context
                .as_deref()
                .map(Value::from)
                .unwrap_or(Value::Null),
            "isBlocked": !blocked_intercepts.is_empty(),
            "navigation": state
                .navigation
                .as_deref()
                .map(Value::from)
                .unwrap_or(Value::Null),
            "redirectCount": state.redirect_count,
            "request": state.request,
            "timestamp": timestamp,
        },
    });
    if !blocked_intercepts.is_empty()
        && let Some(params) = event["params"].as_object_mut()
    {
        params.insert(
            "intercepts".to_owned(),
            Value::Array(
                blocked_intercepts
                    .iter()
                    .map(|intercept| Value::from(intercept.as_str()))
                    .collect(),
            ),
        );
    }
    event
}

fn bidi_network_event_with_response(
    method: &str,
    state: &BidiNetworkRequestState,
    timestamp: u64,
    response: Value,
    blocked_intercepts: &[DevToolsNetworkInterceptId],
) -> Value {
    let mut event = bidi_network_event(method, state, timestamp, blocked_intercepts);
    if let Some(params) = event["params"].as_object_mut() {
        params.insert("response".to_owned(), response);
    }
    event
}

fn bidi_network_request_data_from_cdp_request_params(params: &Value, request_id: &str) -> Value {
    let request = &params["request"];
    let bidi_request_id = params
        .get("__moliFetchRequestId")
        .and_then(Value::as_str)
        .unwrap_or(request_id);
    let headers = bidi_network_headers_from_cdp_headers(&request["headers"]);
    let resource_type = params
        .get("type")
        .or_else(|| params.get("resourceType"))
        .and_then(Value::as_str);
    let initiator_type = params["initiator"].get("type").and_then(Value::as_str);
    let request_initiator_type_override = params
        .get("__moliRequestInitiatorType")
        .and_then(Value::as_str);
    let post_data = request.get("postData").and_then(Value::as_str);
    json!({
        "request": bidi_request_id,
        "url": request
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "method": request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "headers": headers,
        "cookies": bidi_network_request_cookies_from_cdp_params(params),
        "headersSize": bidi_network_headers_size(&headers),
        "bodySize": bidi_network_request_body_size(request, &headers, post_data),
        "destination": bidi_network_destination(resource_type, initiator_type),
        "initiatorType": request_initiator_type_override
            .or_else(|| bidi_network_request_initiator_type(resource_type, initiator_type))
            .map(Value::from)
            .unwrap_or(Value::Null),
        "timings": bidi_network_timings(bidi_network_timestamp_millis_from_cdp_params(params)),
    })
}

fn bidi_network_request_cookies_from_cdp_params(params: &Value) -> Vec<Value> {
    params
        .pointer("/cookieAccessReport/includedCookies")
        .and_then(Value::as_array)
        .map(|cookies| {
            cookies
                .iter()
                .filter_map(|access| access.get("cookie"))
                .cloned()
                .map(bidi_cookie_from_cdp_cookie)
                .collect()
        })
        .unwrap_or_default()
}

fn bidi_network_navigation_id_from_cdp_params(params: &Value) -> Option<String> {
    let resource_type = params
        .get("type")
        .or_else(|| params.get("resourceType"))
        .and_then(Value::as_str)?;
    if resource_type != "Document" {
        return None;
    }
    let loader_id = non_empty_json_string(&params["loaderId"])?;
    Some(webdriver_bidi_navigation_id_from_loader_id(&loader_id).into_string())
}

fn apply_fetch_request_id_override(params: &Value, request: &mut Value) {
    let Some(fetch_request_id) = params.get("__moliFetchRequestId").and_then(Value::as_str) else {
        return;
    };
    apply_fetch_request_id(request, fetch_request_id);
}

fn apply_fetch_request_id(request: &mut Value, fetch_request_id: &str) {
    if let Some(request) = request.as_object_mut() {
        request.insert("request".to_owned(), json!(fetch_request_id));
    }
}

fn bidi_network_response_data_from_cdp_response_params(params: &Value) -> Value {
    let response = &params["response"];
    let headers = bidi_network_headers_from_cdp_headers(&response["headers"]);
    let status = json_number_as_u64(&response["status"]).unwrap_or_default();
    let bytes_received = json_number_as_u64(&response["encodedDataLength"]).unwrap_or_default();
    let from_cache = response
        .get("fromDiskCache")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || response
            .get("fromPrefetchCache")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let mut response_data = json!({
        "url": response
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "protocol": response
            .get("protocol")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "status": status,
        "statusText": response
            .get("statusText")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "fromCache": from_cache,
        "headers": headers.clone(),
        "mimeType": response
            .get("mimeType")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "bytesReceived": bytes_received,
        "headersSize": bidi_network_headers_size(&headers),
        "bodySize": 0_u64,
        "content": {
            "size": 0_u64,
        },
    });
    add_bidi_network_response_auth_challenges(&mut response_data, status, &headers);
    response_data
}

fn bidi_network_response_data_from_cdp_redirect_response_params(params: &Value) -> Value {
    bidi_network_response_data_from_cdp_response_params(&json!({
        "response": params["redirectResponse"].clone(),
    }))
}

fn bidi_network_response_data_from_fetch_paused_params(params: &Value) -> Value {
    let request = &params["request"];
    let headers = bidi_network_headers_from_fetch_header_array(&params["responseHeaders"]);
    let status = json_number_as_u64(&params["responseStatusCode"]).unwrap_or_default();
    let status_text = params
        .get("responseStatusText")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            u16::try_from(status)
                .ok()
                .map(http_status_text)
                .unwrap_or("")
                .to_owned()
        });
    let mut response_data = json!({
        "url": request
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "protocol": "",
        "status": status,
        "statusText": status_text,
        "fromCache": false,
        "headers": headers.clone(),
        "mimeType": "",
        "bytesReceived": 0_u64,
        "headersSize": bidi_network_headers_size(&headers),
        "bodySize": 0_u64,
        "content": {
            "size": 0_u64,
        },
    });
    add_bidi_network_response_auth_challenges(&mut response_data, status, &headers);
    response_data
}

fn bidi_network_response_data_from_auth_required_params(params: &Value) -> Value {
    let request = &params["request"];
    let challenge = &params["authChallenge"];
    let source = challenge.get("source").and_then(Value::as_str);
    let (status, status_text) = match source {
        Some("Proxy") => (407_u64, "Proxy Authentication Required"),
        _ => (401_u64, "Unauthorized"),
    };
    json!({
        "url": request
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "protocol": "",
        "status": status,
        "statusText": status_text,
        "fromCache": false,
        "headers": [],
        "mimeType": "",
        "bytesReceived": 0_u64,
        "headersSize": 0_u64,
        "bodySize": 0_u64,
        "content": {
            "size": 0_u64,
        },
        "authChallenges": bidi_network_auth_challenges_from_cdp_auth_required_params(params),
    })
}

fn bidi_network_auth_challenges_from_cdp_auth_required_params(params: &Value) -> Vec<Value> {
    let challenge = &params["authChallenge"];
    vec![json!({
        "scheme": bidi_network_auth_challenge_scheme(
            challenge.get("scheme").and_then(Value::as_str).unwrap_or_default()
        ),
        "realm": challenge.get("realm").and_then(Value::as_str).unwrap_or_default(),
    })]
}

fn add_bidi_network_response_auth_challenges(response: &mut Value, status: u64, headers: &[Value]) {
    let Some(challenges) = bidi_network_auth_challenges_from_response_headers(status, headers)
    else {
        return;
    };
    if let Some(response) = response.as_object_mut() {
        response.insert("authChallenges".to_owned(), Value::Array(challenges));
    }
}

fn bidi_network_auth_challenges_from_response_headers(
    status: u64,
    headers: &[Value],
) -> Option<Vec<Value>> {
    let challenge_header = match status {
        401 => "www-authenticate",
        407 => "proxy-authenticate",
        _ => return None,
    };
    Some(
        headers
            .iter()
            .filter_map(|header| {
                let name = header.get("name")?.as_str()?;
                if !name.eq_ignore_ascii_case(challenge_header) {
                    return None;
                }
                let value = header.get("value")?.get("value")?.as_str()?;
                bidi_network_auth_challenge_from_header_value(value)
            })
            .collect(),
    )
}

fn bidi_network_auth_challenge_from_header_value(value: &str) -> Option<Value> {
    let value = value.trim();
    let (scheme, parameters) = value
        .split_once(char::is_whitespace)
        .map(|(scheme, parameters)| (scheme, parameters.trim()))
        .unwrap_or((value, ""));
    let scheme = scheme.trim_end_matches(',');
    if scheme.is_empty() {
        return None;
    }
    Some(json!({
        "scheme": bidi_network_auth_challenge_scheme(scheme),
        "realm": auth_challenge_parameter_value(parameters, "realm").unwrap_or_default(),
    }))
}

fn auth_challenge_parameter_value(parameters: &str, name: &str) -> Option<String> {
    parameters.split(',').find_map(|parameter| {
        let (key, value) = parameter.trim().split_once('=')?;
        if !key.trim().eq_ignore_ascii_case(name) {
            return None;
        }
        Some(unquote_auth_challenge_parameter_value(value.trim()))
    })
}

fn unquote_auth_challenge_parameter_value(value: &str) -> String {
    let Some(quoted) = value.strip_prefix('"') else {
        return value.to_owned();
    };
    let mut result = String::new();
    let mut escaped = false;
    for character in quoted.chars() {
        if escaped {
            result.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            break;
        } else {
            result.push(character);
        }
    }
    result
}

fn bidi_network_auth_challenge_scheme(scheme: &str) -> String {
    match scheme.to_ascii_lowercase().as_str() {
        "basic" => "Basic".to_owned(),
        "digest" => "Digest".to_owned(),
        _ => scheme.to_owned(),
    }
}

fn blocked_intercepts_from_cdp_params(params: &Value) -> Vec<DevToolsNetworkInterceptId> {
    params
        .get("__moliBlockedInterceptors")
        .and_then(Value::as_array)
        .map(|intercepts| {
            intercepts
                .iter()
                .filter_map(Value::as_str)
                .map(DevToolsNetworkInterceptId::from)
                .collect()
        })
        .unwrap_or_default()
}

fn synthesized_bidi_network_request_state(
    request_id: &str,
    params: &Value,
) -> BidiNetworkRequestState {
    let url = params["response"]
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let resource_type = params
        .get("type")
        .or_else(|| params.get("resourceType"))
        .and_then(Value::as_str);
    let is_data_document = resource_type == Some("Document")
        && url
            .get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:"));
    let method = if is_data_document { "GET" } else { "" };
    BidiNetworkRequestState {
        context: non_empty_json_string(&params["frameId"]),
        navigation: bidi_network_navigation_id_from_cdp_params(params),
        redirect_count: 0,
        request: json!({
            "request": request_id,
            "url": url,
            "method": method,
            "headers": [],
            "cookies": [],
            "headersSize": 0_u64,
            "bodySize": Value::Null,
            "destination": bidi_network_destination(resource_type, None),
            "initiatorType": Value::Null,
            "timings": bidi_network_timings(bidi_network_timestamp_millis_from_cdp_params(params)),
        }),
        response: None,
    }
}

fn synthesized_bidi_network_redirect_request_state(
    request_id: &str,
    params: &Value,
    redirect_count: u64,
) -> BidiNetworkRequestState {
    let mut request = bidi_network_request_data_from_cdp_request_params(params, request_id);
    if let Some(url) = params["redirectResponse"]
        .get("url")
        .and_then(Value::as_str)
        && let Some(request) = request.as_object_mut()
    {
        request.insert("url".to_owned(), json!(url));
    }
    BidiNetworkRequestState {
        context: non_empty_json_string(&params["frameId"]),
        navigation: bidi_network_navigation_id_from_cdp_params(params),
        redirect_count,
        request,
        response: None,
    }
}

fn bidi_network_response_with_encoded_length(
    mut response: Value,
    encoded_length: Option<u64>,
) -> Value {
    if let Some(encoded_length) = encoded_length {
        response["bytesReceived"] = json!(encoded_length);
        response["content"]["size"] = json!(encoded_length);
    }
    response
}

fn bidi_network_headers_from_cdp_headers(headers: &Value) -> Vec<Value> {
    headers
        .as_object()
        .map(|headers| {
            headers
                .iter()
                .map(|(name, value)| {
                    json!({
                        "name": name,
                        "value": {
                            "type": "string",
                            "value": cdp_header_value_to_string(value),
                        },
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn bidi_network_headers_from_fetch_header_array(headers: &Value) -> Vec<Value> {
    headers
        .as_array()
        .map(|headers| {
            headers
                .iter()
                .filter_map(|header| {
                    Some(json!({
                        "name": header.get("name")?.as_str()?,
                        "value": {
                            "type": "string",
                            "value": header.get("value")?.as_str().unwrap_or_default(),
                        },
                    }))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn cdp_header_value_to_string(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn bidi_network_headers_size(headers: &[Value]) -> u64 {
    headers
        .iter()
        .filter_map(|header| {
            Some(format!(
                "{}: {}\r\n",
                header.get("name")?.as_str()?,
                header.get("value")?.get("value")?.as_str()?
            ))
        })
        .map(|line| line.len() as u64)
        .sum()
}

fn bidi_network_request_body_size(
    request: &Value,
    headers: &[Value],
    post_data: Option<&str>,
) -> Value {
    if let Some(post_data) = post_data {
        return json!(post_data.len() as u64);
    }
    if request
        .get("hasPostData")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Value::Null;
    }
    json!(content_length_from_bidi_headers(headers).unwrap_or(0))
}

fn content_length_from_bidi_headers(headers: &[Value]) -> Option<u64> {
    headers.iter().find_map(|header| {
        let name = header.get("name")?.as_str()?;
        if !name.eq_ignore_ascii_case("content-length") {
            return None;
        }
        header
            .get("value")?
            .get("value")?
            .as_str()?
            .parse::<u64>()
            .ok()
    })
}

fn bidi_network_destination(
    resource_type: Option<&str>,
    initiator_type: Option<&str>,
) -> &'static str {
    match resource_type {
        Some("Script") => "script",
        Some("Stylesheet") => "style",
        Some("Image") => "image",
        Some("Document") if initiator_type == Some("parser") => "iframe",
        Some("Document") => "document",
        _ => "",
    }
}

fn bidi_network_request_initiator_type(
    resource_type: Option<&str>,
    initiator_type: Option<&str>,
) -> Option<&'static str> {
    match (resource_type, initiator_type) {
        (Some("Document"), Some("parser")) => Some("iframe"),
        (Some("Font"), Some("parser")) => Some("font"),
        (Some("Image"), Some("parser")) => Some("img"),
        (Some("Script"), Some("parser")) => Some("script"),
        (Some("Stylesheet"), Some("parser")) => Some("link"),
        (Some("Fetch"), _) => Some("fetch"),
        (_, Some("script")) => Some("script"),
        _ => None,
    }
}

fn bidi_network_initiator_from_cdp_params(params: &Value) -> Option<Value> {
    let initiator = &params["initiator"];
    let initiator_type = match initiator.get("type").and_then(Value::as_str) {
        Some("parser") => "parser",
        Some("script") => "script",
        Some("preflight") => "preflight",
        Some("other") | Some(_) => "other",
        None => return None,
    };
    let mut value = json!({ "type": initiator_type });
    if let Some(line) = json_number_as_u64(&initiator["lineNumber"])
        && let Some(object) = value.as_object_mut()
    {
        object.insert("lineNumber".to_owned(), json!(line));
    }
    if let Some(column) = json_number_as_u64(&initiator["columnNumber"])
        && let Some(object) = value.as_object_mut()
    {
        object.insert("columnNumber".to_owned(), json!(column));
    }
    if let Some(request) = initiator.get("requestId").and_then(Value::as_str)
        && let Some(object) = value.as_object_mut()
    {
        object.insert("request".to_owned(), json!(request));
    }
    Some(value)
}

fn bidi_network_timings(timestamp: u64) -> Value {
    json!({
        "timeOrigin": timestamp,
        "requestTime": 0,
        "redirectStart": 0,
        "redirectEnd": 0,
        "fetchStart": 0,
        "dnsStart": 0,
        "dnsEnd": 0,
        "connectStart": 0,
        "connectEnd": 0,
        "tlsStart": 0,
        "requestStart": 0,
        "responseStart": 0,
        "responseEnd": 0,
    })
}

fn bidi_network_timestamp_millis_from_cdp_params(params: &Value) -> u64 {
    params
        .get("wallTime")
        .or_else(|| params.get("timestamp"))
        .and_then(Value::as_f64)
        .map(|timestamp| (timestamp.max(0.0) * 1000.0).round() as u64)
        .unwrap_or_else(bidi_timestamp_millis)
}

fn json_number_as_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
        .or_else(|| {
            value
                .as_f64()
                .filter(|value| value.is_finite() && *value >= 0.0)
                .map(|value| value.round() as u64)
        })
}

#[cfg(test)]
mod tests {
    use moli_protocol::devtools_runtime::{
        AutomationEvent, DevToolsFrameId, DevToolsLoaderId, DevToolsNetworkInterceptId,
        DevToolsNetworkResourceType, DevToolsRequestId, DevToolsTargetId, NetworkRequestEvent,
    };
    use serde_json::json;

    use super::BidiNetworkEventState;

    fn request_will_be_sent(request_id: &str, url: &str) -> serde_json::Value {
        json!({
            "method": "Network.requestWillBeSent",
            "params": {
                "requestId": request_id,
                "loaderId": "LOADER-1",
                "documentURL": "https://example.test/",
                "request": {
                    "url": url,
                    "method": "GET",
                    "headers": {},
                    "hasPostData": false
                },
                "timestamp": 1.0,
                "wallTime": 1.0,
                "initiator": { "type": "parser" },
                "type": "Script",
                "frameId": "FRAME-1"
            }
        })
    }

    fn response_received(request_id: &str, url: &str) -> serde_json::Value {
        json!({
            "method": "Network.responseReceived",
            "params": {
                "requestId": request_id,
                "loaderId": "LOADER-1",
                "timestamp": 1.1,
                "type": "Script",
                "frameId": "FRAME-1",
                "response": {
                    "url": url,
                    "status": 200,
                    "statusText": "OK",
                    "headers": {},
                    "mimeType": "text/javascript",
                    "encodedDataLength": 0,
                    "protocol": "http/1.1",
                    "fromDiskCache": false,
                    "fromPrefetchCache": false
                }
            }
        })
    }

    fn loading_finished(request_id: &str) -> serde_json::Value {
        json!({
            "method": "Network.loadingFinished",
            "params": {
                "requestId": request_id,
                "timestamp": 1.2,
                "encodedDataLength": 0
            }
        })
    }

    #[test]
    fn response_started_maps_cdp_disk_cache_to_bidi_from_cache() {
        let mut state = BidiNetworkEventState::default();
        assert_eq!(
            state
                .events_from_protocol_message(&json!({
                    "method": "Network.requestWillBeSent",
                    "params": {
                        "requestId": "REQ-1",
                        "loaderId": "LOADER-1",
                        "documentURL": "https://example.test/",
                        "request": {
                            "url": "https://example.test/image.png",
                            "method": "GET",
                            "headers": {},
                            "hasPostData": false
                        },
                        "timestamp": 1.0,
                        "wallTime": 1.0,
                        "initiator": { "type": "parser" },
                        "type": "Image",
                        "frameId": "FRAME-1"
                    }
                }))
                .len(),
            1
        );

        let events = state.events_from_protocol_message(&json!({
            "method": "Network.responseReceived",
            "params": {
                "requestId": "REQ-1",
                "loaderId": "LOADER-1",
                "timestamp": 1.1,
                "type": "Image",
                "frameId": "FRAME-1",
                "response": {
                    "url": "https://example.test/image.png",
                    "status": 200,
                    "statusText": "OK",
                    "headers": {},
                    "mimeType": "image/png",
                    "encodedDataLength": 0,
                    "protocol": "http/1.1",
                    "fromDiskCache": true,
                    "fromPrefetchCache": false
                }
            }
        }));

        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["method"], json!("network.responseStarted"));
        assert_eq!(events[0]["params"]["response"]["fromCache"], json!(true));
    }

    #[test]
    fn response_started_maps_typed_cache_flag_to_bidi_from_cache() {
        let mut state = BidiNetworkEventState::default();
        let event = AutomationEvent::NetworkResponseStarted(NetworkRequestEvent {
            target_id: DevToolsTargetId::from("FRAME-1"),
            frame_id: Some(DevToolsFrameId::from("FRAME-1")),
            request_id: DevToolsRequestId::from("REQ-1"),
            loader_id: Some(DevToolsLoaderId::from("LOADER-1")),
            url: "https://example.test/api".to_owned(),
            document_url: None,
            method: None,
            request_headers: Vec::new(),
            request_body: None,
            request_initiator_type: None,
            bidi_request_initiator_type: None,
            redirect_response: None,
            redirect_has_extra_info: false,
            request_cookie_report: None,
            resource_type: Some(DevToolsNetworkResourceType::Fetch),
            timestamp: Some(1.25),
            wall_time: None,
            status: Some(200),
            status_text: None,
            response_headers: Vec::new(),
            response_mime_type: None,
            response_protocol: None,
            has_extra_info: false,
            encoded_data_length: Some(0),
            from_cache: true,
            fetch_request_id: None,
            error_text: None,
            loading_failed_canceled: false,
            blocked_intercepts: Vec::new(),
            network_id: None,

            auth_challenge: None,
        });

        let events = state.events_from_automation_event(&event);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["method"], json!("network.responseStarted"));
        assert_eq!(events[0]["params"]["response"]["fromCache"], json!(true));
    }

    #[test]
    fn request_paused_automation_event_maps_blocked_response_to_fetch_request_id() {
        let mut state = BidiNetworkEventState::default();
        let request = request_will_be_sent("REQ-1", "https://example.test/api");
        assert_eq!(state.events_from_protocol_message(&request).len(), 1);

        let event = AutomationEvent::RequestPaused(NetworkRequestEvent {
            target_id: DevToolsTargetId::from("FRAME-1"),
            frame_id: Some(DevToolsFrameId::from("FRAME-1")),
            request_id: DevToolsRequestId::from("FETCH-1"),
            loader_id: Some(DevToolsLoaderId::from("REQ-1")),
            url: "https://example.test/api".to_owned(),
            document_url: None,
            method: Some("GET".to_owned()),
            request_headers: Vec::new(),
            request_body: None,
            request_initiator_type: None,
            bidi_request_initiator_type: None,
            redirect_response: None,
            redirect_has_extra_info: false,
            request_cookie_report: None,
            resource_type: Some(DevToolsNetworkResourceType::Fetch),
            timestamp: Some(1.25),
            wall_time: None,
            status: Some(200),
            status_text: None,
            response_headers: Vec::new(),
            response_mime_type: None,
            response_protocol: None,
            has_extra_info: false,
            encoded_data_length: None,
            from_cache: false,
            fetch_request_id: None,
            error_text: None,
            loading_failed_canceled: false,
            blocked_intercepts: vec![DevToolsNetworkInterceptId::from("intercept-response")],
            network_id: Some(DevToolsRequestId::from("REQ-1")),

            auth_challenge: None,
        });

        let events = state.events_from_automation_event(&event);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["method"], json!("network.responseStarted"));
        assert_eq!(events[0]["params"]["isBlocked"], json!(true));
        assert_eq!(
            events[0]["params"]["intercepts"],
            json!(["intercept-response"])
        );
        assert_eq!(events[0]["params"]["request"]["request"], json!("FETCH-1"));
        assert_eq!(events[0]["params"]["response"]["status"], json!(200));
    }

    #[test]
    fn duplicate_response_started_after_completion_is_ignored() {
        let mut state = BidiNetworkEventState::default();
        let url = "https://example.test/module.js";

        assert_eq!(
            state
                .events_from_protocol_message(&request_will_be_sent("REQ-1", url))
                .len(),
            1
        );
        assert_eq!(
            state
                .events_from_protocol_message(&response_received("REQ-1", url))
                .len(),
            1
        );
        assert_eq!(
            state
                .events_from_protocol_message(&loading_finished("REQ-1"))
                .len(),
            1
        );

        assert!(
            state
                .events_from_protocol_message(&response_received("REQ-1", url))
                .is_empty(),
            "late duplicate responseReceived for a completed request must not synthesize another responseStarted"
        );
    }

    #[test]
    fn request_will_be_sent_reopens_completed_request_id() {
        let mut state = BidiNetworkEventState::default();
        let first_url = "https://example.test/first.js";
        let second_url = "https://example.test/second.js";

        assert_eq!(
            state
                .events_from_protocol_message(&request_will_be_sent("REQ-1", first_url))
                .len(),
            1
        );
        assert_eq!(
            state
                .events_from_protocol_message(&response_received("REQ-1", first_url))
                .len(),
            1
        );
        assert_eq!(
            state
                .events_from_protocol_message(&loading_finished("REQ-1"))
                .len(),
            1
        );

        assert_eq!(
            state
                .events_from_protocol_message(&request_will_be_sent("REQ-1", second_url))
                .len(),
            1,
            "a new requestWillBeSent owns reopening a reused request id"
        );
        let events = state.events_from_protocol_message(&response_received("REQ-1", second_url));
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0]["params"]["response"]["url"],
            json!("https://example.test/second.js")
        );
    }
}
