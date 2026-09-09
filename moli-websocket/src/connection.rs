use std::collections::VecDeque;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Message};

use crate::{
    Command, ConnectOptions, Event, FrameOpcode,
    events::{EventSender, send_error_and_close, send_event},
    handle::AbortOnDrop,
    headers::header_map_entries,
    limits::{acquire_pending_websocket_handshake_slot, acquire_websocket_connection_slot},
    request::build_websocket_request,
    stream::open_websocket_stream,
};

pub(crate) async fn run_websocket_connection(
    socket_id: u64,
    url: String,
    protocols: Vec<String>,
    context: ConnectOptions,
    mut command_rx: mpsc::UnboundedReceiver<Command>,
    event_tx: EventSender,
) {
    let Some(_connection_slot) = acquire_websocket_connection_slot() else {
        send_error_and_close(
            &event_tx,
            socket_id,
            "WebSocket connection failed: insufficient resources".to_owned(),
        )
        .await;
        return;
    };

    let request = match build_websocket_request(&url, &protocols, &context) {
        Ok(request) => request,
        Err(error) => {
            send_error_and_close(&event_tx, socket_id, error).await;
            return;
        }
    };

    let request_headers = header_map_entries(request.headers());
    let Some(pending_handshake_slot) = acquire_pending_websocket_handshake_slot() else {
        send_error_and_close(
            &event_tx,
            socket_id,
            "WebSocket connection failed: too many pending handshakes".to_owned(),
        )
        .await;
        return;
    };
    let handshake = open_websocket_stream(request, &context);
    tokio::pin!(handshake);
    let (stream, response) = loop {
        tokio::select! {
            biased;
            command = command_rx.recv() => {
                match command {
                    Some(Command::Close { .. }) => {
                        drop(pending_handshake_slot);
                        send_error_and_close(
                            &event_tx,
                            socket_id,
                            "WebSocket connection closed before opening".to_owned(),
                        )
                        .await;
                        return;
                    }
                    Some(Command::SendText(_))
                    | Some(Command::SendBinary(_))
                    | Some(Command::ReceiveText(_))
                    | Some(Command::ReceiveBinary(_))
                    | Some(Command::ServerClose { .. }) => {
                        // Browser-visible `send()` throws while CONNECTING, so these commands
                        // should only appear from direct crate users. Ignore them rather than
                        // queueing frames before the opening handshake has succeeded.
                    }
                    Some(Command::ContinueOpen { .. }) | Some(Command::FailOpen(_)) => {}
                    None => return,
                }
            }
            connected = &mut handshake => {
                match connected {
                    Ok(connected) => {
                        drop(pending_handshake_slot);
                        break connected;
                    }
                    Err(error) => {
                        drop(pending_handshake_slot);
                        send_error_and_close(
                            &event_tx,
                            socket_id,
                            format!("WebSocket connection failed: {error}"),
                        )
                        .await;
                        return;
                    }
                }
            }
        }
    };

    let mut response_status = response.status().as_u16();
    let mut response_headers = header_map_entries(response.headers());
    if context.pause_after_handshake {
        let _ = send_event(
            &event_tx,
            Event::HandshakeResponse {
                socket_id,
                protocol: response_header(&response_headers, "sec-websocket-protocol")
                    .unwrap_or_default()
                    .to_owned(),
                extensions: response_header(&response_headers, "sec-websocket-extensions")
                    .unwrap_or_default()
                    .to_owned(),
                request_headers: request_headers.clone(),
                response_status,
                response_headers: response_headers.clone(),
            },
        )
        .await;
        loop {
            match command_rx.recv().await {
                Some(Command::ContinueOpen {
                    response_status: override_status,
                    response_headers: override_headers,
                }) => {
                    if let Some(override_status) = override_status {
                        response_status = override_status;
                    }
                    if let Some(override_headers) = override_headers {
                        response_headers = override_headers;
                    }
                    break;
                }
                Some(Command::FailOpen(message)) => {
                    send_error_and_close(&event_tx, socket_id, message).await;
                    return;
                }
                Some(Command::Close { .. }) => {
                    send_error_and_close(
                        &event_tx,
                        socket_id,
                        "WebSocket connection closed before opening".to_owned(),
                    )
                    .await;
                    return;
                }
                Some(Command::SendText(_))
                | Some(Command::SendBinary(_))
                | Some(Command::ReceiveText(_))
                | Some(Command::ReceiveBinary(_))
                | Some(Command::ServerClose { .. }) => {
                    // Browser-visible `send()` throws until the open event, so crate users
                    // cannot enqueue application data while a response-stage pause is active.
                }
                None => return,
            }
        }
    }
    let protocol = response_header(&response_headers, "sec-websocket-protocol")
        .unwrap_or_default()
        .to_owned();
    let extensions = response_header(&response_headers, "sec-websocket-extensions")
        .unwrap_or_default()
        .to_owned();
    let _ = send_event(
        &event_tx,
        Event::Open {
            socket_id,
            protocol,
            extensions,
            request_headers,
            response_status,
            response_headers,
        },
    )
    .await;

    run_open_websocket_connection(socket_id, stream, command_rx, event_tx).await;
}

async fn run_open_websocket_connection(
    socket_id: u64,
    stream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    command_rx: mpsc::UnboundedReceiver<Command>,
    event_tx: EventSender,
) {
    let (write, mut read) = stream.split();
    let (writer_event_tx, mut writer_event_rx) = mpsc::unbounded_channel();
    let (writer_control_tx, writer_control_rx) = mpsc::unbounded_channel();
    let writer = tokio::spawn(run_websocket_writer(
        write,
        command_rx,
        writer_control_rx,
        writer_event_tx,
    ));
    let _writer_task = AbortOnDrop::new(writer.abort_handle());
    let mut sent_close: Option<(u16, String)> = None;
    let mut pending_buffered_amount = VecDeque::new();
    let mut writer_done = false;
    loop {
        tokio::select! {
            biased;
            // Incoming close/error frames should decide browser-visible state before
            // a concurrent writer-side send failure caused by the same remote close.
            message = read.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        send_next_buffered_amount(
                            &event_tx,
                            socket_id,
                            &mut pending_buffered_amount,
                        )
                        .await;
                        let _ = send_event(&event_tx, Event::TextMessage {
                            socket_id,
                            data: text.to_string(),
                        })
                        .await;
                    }
                    Some(Ok(Message::Binary(data))) => {
                        send_next_buffered_amount(
                            &event_tx,
                            socket_id,
                            &mut pending_buffered_amount,
                        )
                        .await;
                        let _ = send_event(&event_tx, Event::BinaryMessage {
                            socket_id,
                            data: data.to_vec(),
                        })
                        .await;
                    }
                    Some(Ok(Message::Close(frame))) => {
                        flush_pending_buffered_amount(
                            &event_tx,
                            socket_id,
                            &mut pending_buffered_amount,
                        )
                        .await;
                        let (code, reason) = frame
                            .map(|frame| (u16::from(frame.code), frame.reason.to_string()))
                            .unwrap_or((1005, String::new()));
                        let _ = send_event(
                            &event_tx,
                            Event::Close {
                                socket_id,
                                code,
                                reason,
                                was_clean: true,
                            },
                        )
                        .await;
                        break;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ =
                            writer_control_tx.send(WebSocketWriterControl::Pong(payload.to_vec()));
                    }
                    Some(Ok(Message::Pong(_))) | Some(Ok(Message::Frame(_))) => {}
                    Some(Err(error)) => {
                        // The main loop's `biased; read.next()` arm wins races with
                        // `writer_event_rx`. Under load the client-initiated
                        // `Command::Close` can already be sitting in
                        // `writer_event_rx` as `WebSocketWriterEvent::Closing` by
                        // the time the server's TCP reset surfaces as a reader
                        // `Err`. Without draining the writer queue here we'd see
                        // `sent_close == None`, take the "unexpected error" path,
                        // and report `wasClean=false` plus a spurious `error`
                        // event — even though the JS caller had explicitly invoked
                        // `socket.close(...)`. Drain pending writer events first so
                        // the close classification reflects the caller's intent.
                        if matches!(
                            drain_pending_writer_events(
                                &mut writer_event_rx,
                                &event_tx,
                                socket_id,
                                &mut sent_close,
                                &mut pending_buffered_amount,
                                &mut writer_done,
                            )
                            .await,
                            WriterEventOutcome::Terminate
                        ) {
                            // The drain itself surfaced a writer `Error` and
                            // already emitted the `Error` + `Close` terminal
                            // events — don't emit a second terminal close
                            // (which would otherwise mask the real failure
                            // with a clean-looking `wasClean=true`).
                            break;
                        }
                        if let Some((code, reason)) = sent_close.clone() {
                            // Many servers reset the socket after receiving our close frame.
                            // Browser-observable state treats our initiated close as clean.
                            flush_pending_buffered_amount(
                                &event_tx,
                                socket_id,
                                &mut pending_buffered_amount,
                            )
                            .await;
                            let _ = send_event(
                                &event_tx,
                                Event::Close {
                                    socket_id,
                                    code,
                                    reason,
                                    was_clean: true,
                                },
                            )
                            .await;
                        } else {
                            flush_pending_buffered_amount(
                                &event_tx,
                                socket_id,
                                &mut pending_buffered_amount,
                            )
                            .await;
                            send_error_and_close(
                                &event_tx,
                                socket_id,
                                format!("WebSocket receive failed: {error}"),
                            )
                            .await;
                        }
                        break;
                    }
                    None => {
                        // Same race protection as the `Some(Err(_))` arm above:
                        // an EOF on the reader side can land before the writer's
                        // `Closing` event reaches us, so drain pending writer
                        // events to recover the caller's close intent.
                        if matches!(
                            drain_pending_writer_events(
                                &mut writer_event_rx,
                                &event_tx,
                                socket_id,
                                &mut sent_close,
                                &mut pending_buffered_amount,
                                &mut writer_done,
                            )
                            .await,
                            WriterEventOutcome::Terminate
                        ) {
                            break;
                        }
                        flush_pending_buffered_amount(
                            &event_tx,
                            socket_id,
                            &mut pending_buffered_amount,
                        )
                        .await;
                        let (code, reason, was_clean) = sent_close
                            .clone()
                            .map(|(code, reason)| (code, reason, true))
                            .unwrap_or((1006, String::new(), false));
                        let _ = send_event(
                            &event_tx,
                            Event::Close {
                                socket_id,
                                code,
                                reason,
                                was_clean,
                            },
                        )
                        .await;
                        break;
                    }
                }
            }
            writer_event = writer_event_rx.recv(), if !writer_done => {
                let Some(writer_event) = writer_event else {
                    writer_done = true;
                    continue;
                };
                if matches!(
                    handle_writer_event(
                        writer_event,
                        &event_tx,
                        socket_id,
                        &mut sent_close,
                        &mut pending_buffered_amount,
                        &mut writer_done,
                    )
                    .await,
                    WriterEventOutcome::Terminate
                ) {
                    break;
                }
            }
        }
    }
}

enum WebSocketWriterEvent {
    FrameSent {
        opcode: FrameOpcode,
        payload_length: usize,
    },
    Closing {
        code: u16,
        reason: String,
    },
    Error(String),
    Done,
}

enum WebSocketWriterControl {
    Pong(Vec<u8>),
}

async fn run_websocket_writer<S>(
    mut write: futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<S>, Message>,
    mut command_rx: mpsc::UnboundedReceiver<Command>,
    mut control_rx: mpsc::UnboundedReceiver<WebSocketWriterControl>,
    writer_event_tx: mpsc::UnboundedSender<WebSocketWriterEvent>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let mut control_done = false;
    loop {
        tokio::select! {
            biased;
            control = control_rx.recv(), if !control_done => {
                match control {
                    Some(WebSocketWriterControl::Pong(payload)) => {
                        if let Err(error) = write.send(Message::Pong(payload.into())).await {
                            let _ = writer_event_tx.send(WebSocketWriterEvent::Error(format!(
                                "WebSocket pong failed: {error}"
                            )));
                            return;
                        }
                    }
                    None => {
                        control_done = true;
                    }
                }
            }
            command = command_rx.recv() => {
                let Some(command) = command else {
                    let _ = write.send(Message::Close(None)).await;
                    let _ = write.close().await;
                    let _ = writer_event_tx.send(WebSocketWriterEvent::Done);
                    return;
                };
                if handle_websocket_writer_command(&mut write, command, &writer_event_tx).await {
                    return;
                }
            }
        }
    }
}

async fn handle_websocket_writer_command<S>(
    write: &mut futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<S>, Message>,
    command: Command,
    writer_event_tx: &mpsc::UnboundedSender<WebSocketWriterEvent>,
) -> bool
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    match command {
        Command::SendText(text) => {
            let amount = text.len();
            match write.send(Message::Text(text.into())).await {
                Ok(()) => {
                    let _ = writer_event_tx.send(WebSocketWriterEvent::FrameSent {
                        opcode: FrameOpcode::Text,
                        payload_length: amount,
                    });
                }
                Err(error) => {
                    let _ = writer_event_tx.send(WebSocketWriterEvent::Error(format!(
                        "WebSocket send failed: {error}"
                    )));
                    return true;
                }
            }
        }
        Command::SendBinary(bytes) => {
            let amount = bytes.len();
            match write.send(Message::Binary(bytes.into())).await {
                Ok(()) => {
                    let _ = writer_event_tx.send(WebSocketWriterEvent::FrameSent {
                        opcode: FrameOpcode::Binary,
                        payload_length: amount,
                    });
                }
                Err(error) => {
                    let _ = writer_event_tx.send(WebSocketWriterEvent::Error(format!(
                        "WebSocket send failed: {error}"
                    )));
                    return true;
                }
            }
        }
        Command::ReceiveText(_) | Command::ReceiveBinary(_) | Command::ServerClose { .. } => {
            // Synthetic-only commands are consumed by the synthetic transport, not real sockets.
        }
        Command::Close { code, reason } => {
            let close_event_code = code.unwrap_or(1005);
            let close_event_reason = code.map(|_| reason.clone()).unwrap_or_else(String::new);
            let _ = writer_event_tx.send(WebSocketWriterEvent::Closing {
                code: close_event_code,
                reason: close_event_reason,
            });
            let frame = code.map(|code| CloseFrame {
                code: code.into(),
                reason: reason.into(),
            });
            if let Err(error) = write.send(Message::Close(frame)).await {
                let _ = writer_event_tx.send(WebSocketWriterEvent::Error(format!(
                    "WebSocket close failed: {error}"
                )));
                return true;
            }
            let _ = write.close().await;
            return true;
        }
        Command::ContinueOpen { .. } | Command::FailOpen(_) => {}
    }
    false
}

fn response_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

async fn send_next_buffered_amount(
    event_tx: &EventSender,
    socket_id: u64,
    pending_buffered_amount: &mut VecDeque<usize>,
) {
    let Some(amount) = pending_buffered_amount.pop_front() else {
        return;
    };
    let _ = send_event(
        event_tx,
        Event::BufferedAmountConsumed { socket_id, amount },
    )
    .await;
}

async fn flush_pending_buffered_amount(
    event_tx: &EventSender,
    socket_id: u64,
    pending_buffered_amount: &mut VecDeque<usize>,
) {
    while !pending_buffered_amount.is_empty() {
        send_next_buffered_amount(event_tx, socket_id, pending_buffered_amount).await;
    }
}

/// Outcome of processing a single `WebSocketWriterEvent`. `Terminate` means
/// the event itself emitted the connection's final `Event::Error` +
/// `Event::Close` pair (today only the `Error` writer event does this), so
/// the caller should break out of the read/write loop without emitting any
/// further terminal events of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriterEventOutcome {
    Continue,
    Terminate,
}

/// Single-event processor shared by the main `tokio::select!` writer arm and
/// the reader-side terminal drain. Centralising the per-variant side effects
/// here keeps the two code paths in lock-step — when we add a new
/// `WebSocketWriterEvent` variant in the future, both call sites pick up the
/// new behaviour automatically.
async fn handle_writer_event(
    writer_event: WebSocketWriterEvent,
    event_tx: &EventSender,
    socket_id: u64,
    sent_close: &mut Option<(u16, String)>,
    pending_buffered_amount: &mut VecDeque<usize>,
    writer_done: &mut bool,
) -> WriterEventOutcome {
    match writer_event {
        WebSocketWriterEvent::FrameSent {
            opcode,
            payload_length,
        } => {
            let _ = send_event(
                event_tx,
                Event::FrameSent {
                    socket_id,
                    opcode,
                    payload_length,
                },
            )
            .await;
            pending_buffered_amount.push_back(payload_length);
            WriterEventOutcome::Continue
        }
        WebSocketWriterEvent::Closing { code, reason } => {
            *sent_close = Some((code, reason));
            let _ = send_event(event_tx, Event::Closing { socket_id }).await;
            flush_pending_buffered_amount(event_tx, socket_id, pending_buffered_amount).await;
            WriterEventOutcome::Continue
        }
        WebSocketWriterEvent::Error(message) => {
            flush_pending_buffered_amount(event_tx, socket_id, pending_buffered_amount).await;
            send_error_and_close(event_tx, socket_id, message).await;
            WriterEventOutcome::Terminate
        }
        WebSocketWriterEvent::Done => {
            *writer_done = true;
            WriterEventOutcome::Continue
        }
    }
}

/// Drain whatever writer events have already been published into
/// `writer_event_rx` but haven't been processed by the main select loop yet.
/// Used by the reader's terminal arms (`Some(Err(_))` and `None`) so that a
/// `Closing` event published by the writer between the reader's terminal
/// signal landing in the OS socket and the main loop polling it doesn't get
/// dropped — without this drain the close would be reported with
/// `wasClean=false` plus a spurious `error` event, even when the JS caller
/// had explicitly invoked `socket.close(...)`.
///
/// Returns `Terminate` if the drain itself surfaced a writer `Error` (which
/// emits the final `Error` + `Close` pair on its own); the reader-side
/// caller must then break without emitting another terminal close, lest the
/// JS layer see `wasClean=true` despite the writer never actually putting
/// the close frame on the wire.
async fn drain_pending_writer_events(
    writer_event_rx: &mut mpsc::UnboundedReceiver<WebSocketWriterEvent>,
    event_tx: &EventSender,
    socket_id: u64,
    sent_close: &mut Option<(u16, String)>,
    pending_buffered_amount: &mut VecDeque<usize>,
    writer_done: &mut bool,
) -> WriterEventOutcome {
    while let Ok(writer_event) = writer_event_rx.try_recv() {
        if matches!(
            handle_writer_event(
                writer_event,
                event_tx,
                socket_id,
                sent_close,
                pending_buffered_amount,
                writer_done,
            )
            .await,
            WriterEventOutcome::Terminate
        ) {
            return WriterEventOutcome::Terminate;
        }
    }
    WriterEventOutcome::Continue
}
