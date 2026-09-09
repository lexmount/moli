use crate::{
    Command, ConnectOptions, Event,
    commands::CommandReceiver,
    events::{EventResult, EventSender, send_error_and_close, send_event},
    handle::HandshakeDecision,
    headers::header_map_entries,
    limits::{acquire_pending_websocket_handshake_slot, acquire_websocket_connection_slot},
    request::prepare_websocket_request,
    transport::{OpenedConnection, open_websocket_connection},
};

pub(crate) async fn run_websocket_connection(
    socket_id: u64,
    url: String,
    protocols: Vec<String>,
    context: ConnectOptions,
    mut command_rx: CommandReceiver,
    event_tx: EventSender,
    decision: Option<tokio::sync::oneshot::Receiver<HandshakeDecision>>,
) -> EventResult {
    let Some(_connection_slot) = acquire_websocket_connection_slot() else {
        send_error_and_close(
            &event_tx,
            socket_id,
            "WebSocket connection failed: insufficient resources".to_owned(),
        )
        .await?;
        return Ok(());
    };

    let request = match prepare_websocket_request(&url, &protocols, &context) {
        Ok(request) => request,
        Err(error) => {
            send_error_and_close(&event_tx, socket_id, error).await?;
            return Ok(());
        }
    };

    let Some(pending_handshake_slot) = acquire_pending_websocket_handshake_slot() else {
        send_error_and_close(
            &event_tx,
            socket_id,
            "WebSocket connection failed: too many pending handshakes".to_owned(),
        )
        .await?;
        return Ok(());
    };
    let handshake = open_websocket_connection(request, &context);
    tokio::pin!(handshake);
    let OpenedConnection {
        connection,
        handshake,
    } = loop {
        tokio::select! {
            biased;
            command = command_rx.recv() => {
                match command.map(|queued| queued.command) {
                    Some(Command::Close { .. }) => {
                        drop(pending_handshake_slot);
                        send_error_and_close(
                            &event_tx,
                            socket_id,
                            "WebSocket connection closed before opening".to_owned(),
                        )
                        .await?;
                        return Ok(());
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
                    Some(Command::Fail(message)) => {
                        drop(pending_handshake_slot);
                        send_error_and_close(&event_tx, socket_id, message).await?;
                        return Ok(());
                    }
                    None => return Ok(()),
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
                        .await?;
                        return Ok(());
                    }
                }
            }
        }
    };

    let request_headers = header_map_entries(&handshake.request_headers);
    let mut response_status = handshake.response.status().as_u16();
    let mut response_headers = header_map_entries(handshake.response.headers());
    if let Some(mut decision) = decision {
        send_event(
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
        .await?;
        loop {
            tokio::select! {
                // A Close already admitted while paused must precede Open.
                biased;
                command = command_rx.recv() => {
                    match command.map(|queued| queued.command) {
                        Some(Command::Fail(message)) => {
                            drop(connection);
                            send_error_and_close(&event_tx, socket_id, message).await?;
                            return Ok(());
                        }
                        Some(Command::Close { .. }) => {
                            drop(connection);
                            send_error_and_close(
                                &event_tx, socket_id,
                                "WebSocket connection closed before opening".to_owned(),
                            ).await?;
                            return Ok(());
                        }
                        None => return Ok(()),
                        // The browser rejects sends until the Open event.
                        _ => {}
                    }
                }
                decision = &mut decision => {
                    match decision.unwrap_or_else(|_| HandshakeDecision::Fail(
                        "WebSocket handshake decision was dropped".to_owned(),
                    )) {
                        HandshakeDecision::Continue { response_status: status, response_headers: headers } => {
                            if let Some(status) = status { response_status = status; }
                            if let Some(headers) = headers { response_headers = headers; }
                            break;
                        }
                        HandshakeDecision::Fail(message) => {
                            drop(connection);
                            send_error_and_close(&event_tx, socket_id, message).await?;
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
    let protocol = response_header(&response_headers, "sec-websocket-protocol")
        .unwrap_or_default()
        .to_owned();
    let extensions = response_header(&response_headers, "sec-websocket-extensions")
        .unwrap_or_default()
        .to_owned();
    send_event(
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
    .await?;

    crate::session::run_open_session(socket_id, connection, command_rx, event_tx).await
}

fn response_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}
