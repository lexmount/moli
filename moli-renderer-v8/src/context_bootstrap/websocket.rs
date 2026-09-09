use super::media_queries::{
    dispatch_simple_event_target_event, simple_event_target_add_event_listener_callback,
    simple_event_target_dispatch_event_callback,
    simple_event_target_remove_event_listener_callback, simple_object_event_set_ordered_handler,
};
use super::shared::{
    READABLE_STREAM_HWM_SLOT, READABLE_STREAM_PENDING_READS_SLOT,
    SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, SIMPLE_EVENT_TARGET_SLOT, WRITABLE_STREAM_SINK_SLOT,
    define_non_enumerable_string_property, global_constructor_prototype,
    new_uint8_array_from_bytes,
};
use super::stream_adapter::{
    close_stream, enqueue_chunk, error_stream, error_writable_stream_with_value, stream_slot_array,
    stream_slot_number, stream_slot_object,
};
use crate::{
    blob,
    util::{
        callback_data_index_value, callback_data_item, context_host_ptr_from_global_bridge,
        get_private_value, set_private_value, throw_type_error, v8_string, v8str,
    },
};
use moli_websocket::Event as WebSocketEvent;

const WEBSOCKET_LISTENERS_SLOT: &str = "__moliWebSocketListeners";
const WEBSOCKET_ID_SLOT: &str = "__moliWebSocketId";
const WEBSOCKET_URL_SLOT: &str = "__moliWebSocketUrl";
const WEBSOCKET_READY_STATE_SLOT: &str = "__moliWebSocketReadyState";
const WEBSOCKET_BUFFERED_AMOUNT_SLOT: &str = "__moliWebSocketBufferedAmount";
const WEBSOCKET_EXTENSIONS_SLOT: &str = "__moliWebSocketExtensions";
const WEBSOCKET_PROTOCOL_SLOT: &str = "__moliWebSocketProtocol";
const WEBSOCKET_BINARY_TYPE_SLOT: &str = "__moliWebSocketBinaryType";
const WEBSOCKET_ONOPEN_SLOT: &str = "__moliWebSocketOnOpen";
const WEBSOCKET_ONMESSAGE_SLOT: &str = "__moliWebSocketOnMessage";
const WEBSOCKET_ONERROR_SLOT: &str = "__moliWebSocketOnError";
const WEBSOCKET_ONCLOSE_SLOT: &str = "__moliWebSocketOnClose";

const WEBSOCKET_STREAM_URL_SLOT: &str = "__moliWebSocketStreamUrl";
const WEBSOCKET_STREAM_OPENED_SLOT: &str = "__moliWebSocketStreamOpened";
const WEBSOCKET_STREAM_CLOSED_SLOT: &str = "__moliWebSocketStreamClosed";
const WEBSOCKET_STREAM_OPENED_RESOLVE_SLOT: &str = "__moliWebSocketStreamOpenedResolve";
const WEBSOCKET_STREAM_OPENED_REJECT_SLOT: &str = "__moliWebSocketStreamOpenedReject";
const WEBSOCKET_STREAM_CLOSED_RESOLVE_SLOT: &str = "__moliWebSocketStreamClosedResolve";
const WEBSOCKET_STREAM_CLOSED_REJECT_SLOT: &str = "__moliWebSocketStreamClosedReject";
const WEBSOCKET_STREAM_READABLE_SLOT: &str = "__moliWebSocketStreamReadable";
const WEBSOCKET_STREAM_WRITABLE_SLOT: &str = "__moliWebSocketStreamWritable";
const WEBSOCKET_STREAM_ERROR_SLOT: &str = "__moliWebSocketStreamError";
const WEBSOCKET_STREAM_PROMISE_SLOT: &str = "__moliWebSocketStreamPromise";
const WEBSOCKET_STREAM_PROMISE_RESOLVE_SLOT: &str = "__moliWebSocketStreamPromiseResolve";
const WEBSOCKET_STREAM_PROMISE_REJECT_SLOT: &str = "__moliWebSocketStreamPromiseReject";
const WEBSOCKET_STREAM_SINK_CLOSED_PROMISE_SLOT: &str = "__moliWebSocketStreamSinkClosedPromise";
const WEBSOCKET_STREAM_PENDING_WRITES_SLOT: &str = "__moliWebSocketStreamPendingWrites";
const WEBSOCKET_STREAM_SEND_REJECTED_SLOT: &str = "__moliWebSocketStreamSendRejected";

const CONNECTING: f64 = 0.0;
const OPEN: f64 = 1.0;
const CLOSING: f64 = 2.0;
const CLOSED: f64 = 3.0;

mod accessors;
mod bindings;
mod constructor;
mod dispatch;
mod events;
mod helpers;
mod methods;
mod payload;
mod realm;
mod stream;

pub(super) use bindings::{install_websocket_bindings, install_websocket_stream_bindings};
pub(super) use constructor::{
    websocket_constructor_callback, websocket_error_constructor_callback,
    websocket_stream_constructor_callback,
};
pub(crate) use dispatch::{WebSocketDispatchResult, dispatch_websocket_event};
