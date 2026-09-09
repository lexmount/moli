use crate::{
    Command, ConnectOptions, Event,
    commands::CommandReceiver,
    events::{EventResult, EventSender, send_error_and_close, send_event},
    headers::header_map_entries,
    limits::{acquire_pending_websocket_handshake_slot, acquire_websocket_connection_slot},
    request::build_websocket_request,
    transport::open_websocket_stream,
};

pub(crate) async fn run_websocket_connection(
    socket_id: u64,
    url: String,
    protocols: Vec<String>,
    context: ConnectOptions,
    mut command_rx: CommandReceiver,
    event_tx: EventSender,
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

    let request = match build_websocket_request(&url, &protocols, &context) {
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
    let handshake = open_websocket_stream(request, &context);
    tokio::pin!(handshake);
    let (stream, response, request_headers) = loop {
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
                    Some(Command::ContinueOpen { .. }) => {}
                    Some(Command::FailOpen(message)) => {
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

    let mut response_status = response.status().as_u16();
    let mut response_headers = header_map_entries(response.headers());
    if context.pause_after_handshake {
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
            match command_rx.recv().await.map(|queued| queued.command) {
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
                    send_error_and_close(&event_tx, socket_id, message).await?;
                    return Ok(());
                }
                Some(Command::Close { .. }) => {
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
                    // Browser-visible `send()` throws until the open event, so crate users
                    // cannot enqueue application data while a response-stage pause is active.
                }
                None => return Ok(()),
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

    crate::session::run_open_session(socket_id, stream, command_rx, event_tx).await
}

fn response_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}
