//! Conservative retained-memory estimates for renderer output admission.
//!
//! These estimates intentionally live beside the renderer-neutral payload
//! definitions, where private capacities and file-backed response state are
//! visible. They do not model serialized CDP JSON; protocol projection has not
//! happened while these values are queued.

use super::*;

impl DocumentNodeSnapshot {
    #[doc(hidden)]
    pub fn renderer_transport_charge_bytes(&self) -> usize {
        let mut total = 0usize;
        let mut pending = vec![self];
        while let Some(node) = pending.pop() {
            total = total
                .saturating_add(std::mem::size_of::<Self>())
                .saturating_add(node.frame_id.as_deref().map(string_charge).unwrap_or(0))
                .saturating_add(string_charge(&node.node_name))
                .saturating_add(string_charge(&node.local_name))
                .saturating_add(string_charge(&node.node_value))
                .saturating_add(string_charge(&node.document_url))
                .saturating_add(string_charge(&node.base_url))
                .saturating_add(
                    node.namespace_uri
                        .as_deref()
                        .map(string_charge)
                        .unwrap_or(0),
                )
                .saturating_add(
                    node.document_type_name
                        .as_deref()
                        .map(string_charge)
                        .unwrap_or(0),
                )
                .saturating_add(node.public_id.as_deref().map(string_charge).unwrap_or(0))
                .saturating_add(node.system_id.as_deref().map(string_charge).unwrap_or(0))
                .saturating_add(
                    node.shadow_root_type
                        .as_deref()
                        .map(string_charge)
                        .unwrap_or(0),
                )
                .saturating_add(node.pseudo_type.as_deref().map(string_charge).unwrap_or(0));
            total = node.attributes.iter().fold(total, |total, attribute| {
                total
                    .saturating_add(std::mem::size_of_val(attribute))
                    .saturating_add(string_charge(&attribute.local_name))
                    .saturating_add(string_charge(&attribute.value))
            });
            pending.extend(node.shadow_roots.iter());
            pending.extend(node.pseudo_elements.iter());
            pending.extend(node.children.iter());
            if let Some(associated) = node.associated_node() {
                pending.push(associated);
            }
        }
        total
    }
}

impl ScriptNetworkOutputItem {
    #[doc(hidden)]
    pub fn renderer_transport_charge_bytes(&self) -> usize {
        match self {
            Self::SubresourceNetworkRecord(record) => network_record_charge(record),
            Self::SubresourceRequestStarted(request) => request_started_charge(request),
            Self::SubresourceResponseStarted(response) => response_started_charge(response),
            Self::SubresourceDataReceived(_) => 0,
            Self::SubresourceEventSourceMessageReceived(message) => [
                message.event_name.as_str(),
                message.event_id.as_str(),
                message.data.as_str(),
            ]
            .into_iter()
            .map(string_charge)
            .sum(),
            Self::SubresourceBodyFinished(finished) => match &finished.result {
                SubresourceBodyFinishedResult::Ready(body) => {
                    body.renderer_transport_retained_memory_bytes()
                }
                SubresourceBodyFinishedResult::Failed(error) => string_charge(error),
                SubresourceBodyFinishedResult::FailedWithPartialBody {
                    error_text,
                    partial_body,
                } => string_charge(error_text)
                    .saturating_add(partial_body.renderer_transport_retained_memory_bytes()),
            },
            Self::WebSocketNetworkEvent(event) => {
                url_charge(&event.document_url).saturating_add(url_charge(&event.url))
            }
            Self::WebSocketLifecycleEvent(event) => url_charge(&event.document_url)
                .saturating_add(url_charge(&event.url))
                .saturating_add(event.error_text.as_deref().map(string_charge).unwrap_or(0))
                .saturating_add(
                    event
                        .close_reason
                        .as_deref()
                        .map(string_charge)
                        .unwrap_or(0),
                ),
        }
    }
}

impl InspectorIssueSnapshot {
    #[doc(hidden)]
    pub fn renderer_transport_charge_bytes(&self) -> usize {
        match self {
            Self::QuirksMode(issue) => string_charge(issue.url()),
            Self::ContentSecurityPolicy(issue) => {
                let mut total = string_charge(issue.violated_directive());
                total = total.saturating_add(issue.blocked_url().map(string_charge).unwrap_or(0));
                if let Some(location) = issue.source_code_location() {
                    total = total.saturating_add(string_charge(location.url()));
                }
                total
            }
        }
    }
}

impl SubresourceResponseBody {
    #[doc(hidden)]
    pub fn renderer_transport_retained_memory_bytes(&self) -> usize {
        match self.inner.as_ref() {
            SubresourceResponseBodyInner::Memory(bytes) => bytes
                .capacity()
                .saturating_add(std::mem::size_of::<SubresourceResponseBodyInner>()),
            SubresourceResponseBodyInner::File { path, .. } => path
                .as_os_str()
                .len()
                .saturating_mul(2)
                .saturating_add(std::mem::size_of::<SubresourceResponseBodyInner>()),
        }
    }
}

impl PendingSubresourceFetchInfo {
    #[doc(hidden)]
    pub fn renderer_transport_charge_bytes(&self) -> usize {
        self.frame_id
            .as_deref()
            .map(string_charge)
            .unwrap_or(0)
            .saturating_add(url_charge(&self.document_url))
            .saturating_add(url_charge(&self.url))
            .saturating_add(string_charge(&self.method))
            .saturating_add(headers_charge(&self.request_headers))
            .saturating_add(self.request_body.as_deref().map(string_charge).unwrap_or(0))
            .saturating_add(
                self.request_body_bytes
                    .as_ref()
                    .map(Vec::capacity)
                    .unwrap_or(0),
            )
    }
}

impl PendingSubresourceContinueEvent {
    #[doc(hidden)]
    pub fn renderer_transport_charge_bytes(&self) -> usize {
        match self {
            Self::Completed { .. } => 0,
            Self::ResponsePaused(response) => url_charge(&response.url)
                .saturating_add(url_charge(&response.final_url))
                .saturating_add(string_charge(&response.method))
                .saturating_add(headers_charge(&response.request_headers))
                .saturating_add(
                    response
                        .request_body
                        .as_deref()
                        .map(string_charge)
                        .unwrap_or(0),
                )
                .saturating_add(
                    response
                        .network_request_headers
                        .as_deref()
                        .map(headers_charge)
                        .unwrap_or(0),
                )
                .saturating_add(headers_charge(&response.response_headers))
                .saturating_add(
                    response
                        .response_body
                        .renderer_transport_retained_memory_bytes(),
                ),
            Self::AuthRequired(auth) => url_charge(&auth.url)
                .saturating_add(string_charge(&auth.method))
                .saturating_add(headers_charge(&auth.request_headers))
                .saturating_add(auth.request_body.as_deref().map(string_charge).unwrap_or(0))
                .saturating_add(
                    auth.network_request_headers
                        .as_deref()
                        .map(headers_charge)
                        .unwrap_or(0),
                )
                .saturating_add(string_charge(&auth.challenge.source))
                .saturating_add(string_charge(&auth.challenge.scheme))
                .saturating_add(string_charge(&auth.challenge.realm)),
        }
    }
}

impl RuntimeContextRestoreEvent {
    #[doc(hidden)]
    pub fn renderer_transport_charge_bytes_with(
        &self,
        string_charge: impl Fn(&str) -> usize,
    ) -> usize {
        let (Self::Created(event) | Self::Destroyed(event)) = self else {
            return 0;
        };
        [
            event.realm_id.as_deref(),
            event.frame_id.as_deref(),
            event.origin.as_deref(),
            event.name.as_deref(),
            event.context_type.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(string_charge)
        .sum()
    }
}

fn string_charge(value: &str) -> usize {
    value.len().saturating_mul(2)
}

fn url_charge(value: &Url) -> usize {
    string_charge(value.as_str())
}

fn headers_charge(headers: &[(String, String)]) -> usize {
    headers.iter().fold(
        headers
            .len()
            .saturating_mul(std::mem::size_of::<(String, String)>()),
        |total, (name, value)| {
            total
                .saturating_add(string_charge(name))
                .saturating_add(string_charge(value))
        },
    )
}

fn request_started_charge(request: &SubresourceRequestStarted) -> usize {
    request
        .frame_id
        .as_deref()
        .map(string_charge)
        .unwrap_or(0)
        .saturating_add(url_charge(&request.document_url))
        .saturating_add(url_charge(&request.url))
        .saturating_add(string_charge(&request.method))
        .saturating_add(headers_charge(&request.request_headers))
        .saturating_add(
            request
                .request_body
                .as_deref()
                .map(string_charge)
                .unwrap_or(0),
        )
        .saturating_add(
            request
                .request_body_bytes
                .as_ref()
                .map(Vec::capacity)
                .unwrap_or(0),
        )
}

fn response_started_charge(response: &SubresourceResponseStarted) -> usize {
    response
        .redirect_chain
        .len()
        .saturating_mul(256)
        .saturating_add(url_charge(&response.final_url))
        .saturating_add(
            response
                .status_text
                .as_deref()
                .map(string_charge)
                .unwrap_or(0),
        )
        .saturating_add(headers_charge(&response.response_headers))
        .saturating_add(
            response
                .network_request_headers
                .as_deref()
                .map(headers_charge)
                .unwrap_or(0),
        )
        .saturating_add(response.cookie_set_reports.len().saturating_mul(256))
}

fn network_record_charge(record: &SubresourceNetworkRecord) -> usize {
    let record = record.inner.as_ref();
    let mut total = record
        .frame_id
        .as_deref()
        .map(string_charge)
        .unwrap_or(0)
        .saturating_add(url_charge(&record.document_url))
        .saturating_add(url_charge(&record.url))
        .saturating_add(string_charge(&record.method))
        .saturating_add(headers_charge(&record.request_headers))
        .saturating_add(
            record
                .request_body
                .as_deref()
                .map(string_charge)
                .unwrap_or(0),
        )
        .saturating_add(
            record
                .request_body_bytes
                .as_ref()
                .map(Vec::capacity)
                .unwrap_or(0),
        )
        .saturating_add(record.cookie_set_reports.len().saturating_mul(256))
        .saturating_add(
            record
                .network_request_headers
                .as_deref()
                .map(headers_charge)
                .unwrap_or(0),
        );
    total = total.saturating_add(match &record.outcome {
        SubresourceNetworkOutcome::Success {
            redirect_chain,
            final_url,
            status_text,
            response_headers,
            response_body,
            ..
        } => redirect_chain
            .len()
            .saturating_mul(256)
            .saturating_add(url_charge(final_url))
            .saturating_add(status_text.as_deref().map(string_charge).unwrap_or(0))
            .saturating_add(headers_charge(response_headers))
            .saturating_add(response_body.renderer_transport_retained_memory_bytes()),
        SubresourceNetworkOutcome::Failure { error_text } => string_charge(error_text),
    });
    total
}
