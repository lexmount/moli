mod bindings;
mod parser;
mod request;

use crate::context_bootstrap::{
    SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, SIMPLE_EVENT_TARGET_SLOT,
    dispatch_simple_event_target_event, simple_object_event_set_ordered_handler,
};
use crate::util::{
    callback_data_index_value, callback_data_item, context_host_ptr_from_global_bridge,
    get_private_value, set_private_value, throw_type_error, v8_string, v8str,
};

pub(crate) use bindings::{
    EventSourceTerminalMode, dispatch_event_source_message, event_source_active_request_id,
    event_source_connection_url, event_source_last_event_id, event_source_ready_state,
    event_source_reconnect_delay_ms, event_source_with_credentials, fail_event_source_connection,
    initialize_event_source_object, install_event_source_bindings, open_event_source_connection,
    schedule_event_source_connect, set_event_source_active_request_id,
    update_event_source_stream_state,
};
pub(crate) use parser::{EventSourceMessage, EventSourceParser};
pub(crate) use request::event_source_constructor_callback;

const EVENT_SOURCE_URL_SLOT: &str = "__lmEventSourceUrl";
const EVENT_SOURCE_WITH_CREDENTIALS_SLOT: &str = "__lmEventSourceWithCredentials";
const EVENT_SOURCE_READY_STATE_SLOT: &str = "__lmEventSourceReadyState";
const EVENT_SOURCE_ACTIVE_REQUEST_ID_SLOT: &str = "__lmEventSourceActiveRequestId";
const EVENT_SOURCE_RECONNECT_TIMER_SLOT: &str = "__lmEventSourceReconnectTimer";
const EVENT_SOURCE_RECONNECT_DELAY_SLOT: &str = "__lmEventSourceReconnectDelay";
const EVENT_SOURCE_LAST_EVENT_ID_SLOT: &str = "__lmEventSourceLastEventId";
const EVENT_SOURCE_RESPONSE_URL_SLOT: &str = "__lmEventSourceResponseUrl";
const EVENT_SOURCE_LISTENERS_SLOT: &str = "__lmEventSourceListeners";
const EVENT_SOURCE_ONOPEN_SLOT: &str = "__lmEventSourceOnOpen";
const EVENT_SOURCE_ONMESSAGE_SLOT: &str = "__lmEventSourceOnMessage";
const EVENT_SOURCE_ONERROR_SLOT: &str = "__lmEventSourceOnError";

pub(crate) const EVENT_SOURCE_CONNECTING: f64 = 0.0;
pub(crate) const EVENT_SOURCE_OPEN: f64 = 1.0;
pub(crate) const EVENT_SOURCE_CLOSED: f64 = 2.0;
pub(crate) const EVENT_SOURCE_DEFAULT_RECONNECT_DELAY_MS: u64 = 3_000;

pub(crate) fn event_source_response_error(head: &moli_fetch::ResponseHead) -> Option<String> {
    if head.status != 200 {
        return Some(format!(
            "EventSource response has HTTP status {}",
            head.status
        ));
    }
    let Some(content_type) = moli_web_mime::response_header_value(&head.headers, "content-type")
    else {
        return Some("EventSource response is missing Content-Type".to_owned());
    };
    if moli_web_mime::mime_essence(&content_type).as_deref() != Some("text/event-stream") {
        return Some(format!(
            "EventSource response has invalid Content-Type `{content_type}`"
        ));
    }
    if let Some(charset) = moli_web_mime::mime_charset(&content_type)
        && !charset.eq_ignore_ascii_case("utf-8")
    {
        return Some(format!(
            "EventSource response has unsupported charset `{charset}`"
        ));
    }
    None
}
