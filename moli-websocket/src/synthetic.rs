use crate::commands::CommandReceiver;

use crate::{
    Command, Event, FrameOpcode,
    events::{EventResult, EventSender, send_error_and_close, send_event},
    limits::acquire_websocket_connection_slot,
};

pub(crate) async fn run_synthetic_websocket_connection(
    socket_id: u64,
    mut command_rx: CommandReceiver,
    event_tx: EventSender,
    request_headers: Vec<(String, String)>,
    response_status: u16,
    response_headers: Vec<(String, String)>,
) -> EventResult {
    let Some(_connection_slot) = acquire_websocket_connection_slot() else {
        drop(command_rx);
        send_error_and_close(
            &event_tx,
            socket_id,
            "WebSocket connection failed: insufficient resources".to_owned(),
        )
        .await?;
        return Ok(());
    };

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

    while let Some(queued) = command_rx.recv().await {
        let reservation = queued.reservation;
        match queued.command {
            Command::SendText(text) => {
                let amount = text.len();
                send_event(
                    &event_tx,
                    Event::SendCompleted {
                        socket_id,
                        opcode: FrameOpcode::Text,
                        payload_length: amount,
                    },
                )
                .await?;
            }
            Command::SendBinary(bytes) => {
                let amount = bytes.len();
                send_event(
                    &event_tx,
                    Event::SendCompleted {
                        socket_id,
                        opcode: FrameOpcode::Binary,
                        payload_length: amount,
                    },
                )
                .await?;
            }
            Command::ReceiveText(data) => {
                send_event(&event_tx, Event::TextMessage { socket_id, data }).await?;
            }
            Command::ReceiveBinary(data) => {
                send_event(&event_tx, Event::BinaryMessage { socket_id, data }).await?;
            }
            Command::ServerClose { code, reason } => {
                drop(command_rx);
                drop(reservation);
                let close_event_code = code.unwrap_or(1005);
                let close_event_reason = code.map(|_| reason).unwrap_or_default();
                send_event(
                    &event_tx,
                    Event::Close {
                        socket_id,
                        code: close_event_code,
                        reason: close_event_reason,
                        was_clean: true,
                    },
                )
                .await?;
                return Ok(());
            }
            Command::Close { code, reason } => {
                drop(command_rx);
                drop(reservation);
                let close_event_code = code.unwrap_or(1005);
                let close_event_reason = code.map(|_| reason).unwrap_or_default();
                send_event(&event_tx, Event::Closing { socket_id }).await?;
                send_event(
                    &event_tx,
                    Event::Close {
                        socket_id,
                        code: close_event_code,
                        reason: close_event_reason,
                        was_clean: true,
                    },
                )
                .await?;
                return Ok(());
            }
            Command::Fail(message) => {
                drop(command_rx);
                drop(reservation);
                send_error_and_close(&event_tx, socket_id, message).await?;
                return Ok(());
            }
        }
    }

    // Owner drop can close this channel before task cancellation is observed.
    Ok(())
}

fn response_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        sync::mpsc,
        time::{Duration, timeout},
    };

    #[tokio::test]
    async fn command_channel_closure_does_not_publish_close() {
        let (commands, receiver) = crate::commands::command_channel();
        let (events, mut incoming) = mpsc::channel(1);
        let task = tokio::spawn(run_synthetic_websocket_connection(
            97,
            receiver,
            events.into(),
            Vec::new(),
            101,
            Vec::new(),
        ));
        assert!(matches!(
            timeout(Duration::from_secs(3), incoming.recv())
                .await
                .unwrap(),
            Some(Event::Open { socket_id: 97, .. })
        ));
        // CommandPort can drop before the owner's AbortOnDrop runs. Exercise
        // that ordering directly, without racing the runtime's cancellation poll.
        drop(commands);
        timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let event = incoming.try_recv();
        assert!(
            matches!(event, Err(mpsc::error::TryRecvError::Disconnected)),
            "dropping command producers must be silent: {event:?}"
        );
    }
}
