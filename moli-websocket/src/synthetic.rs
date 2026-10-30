use crate::{
    commands::{CommandReceiver, Reservation},
    payload::Payload,
};
use std::collections::VecDeque;

struct Outgoing {
    data: Payload,
    opcode: FrameOpcode,
    _reservation: Reservation,
}

use crate::{
    Command, Event, FrameOpcode,
    events::{EventResult, EventSender, send_error_and_close, send_event},
    limits::acquire_websocket_connection_slot,
};

pub(crate) async fn run_synthetic_websocket_connection(
    socket_id: u64,
    mut command_rx: CommandReceiver,
    event_tx: EventSender,
    request_headers: moli_header_field::HeaderFields,
    response_status: u16,
    response_headers: moli_header_field::HeaderFields,
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

    let mut outgoing: VecDeque<Outgoing> = VecDeque::new();
    let mut closing: Option<(Option<u16>, String, Reservation)> = None;
    loop {
        if let Some(message) = outgoing.front_mut() {
            message.data.start_read();
            if message.data.bytes().is_some() {
                let message = outgoing.pop_front().expect("ready message");
                send_event(
                    &event_tx,
                    Event::SendCompleted {
                        socket_id,
                        opcode: message.opcode,
                        payload_length: message.data.len(),
                    },
                )
                .await?;
                continue;
            }
        } else if let Some((code, reason, reservation)) = closing.take() {
            drop(command_rx);
            drop(reservation);
            send_event(
                &event_tx,
                Event::Close {
                    socket_id,
                    code: code.unwrap_or(1005),
                    reason: code.map(|_| reason).unwrap_or_default(),
                    was_clean: true,
                },
            )
            .await?;
            return Ok(());
        }
        let preparing = outgoing
            .front()
            .is_some_and(|message| message.data.is_reading());
        let queued = tokio::select! {
            result = async { outgoing.front_mut().expect("preparing message").data.finish_read().await }, if preparing => {
                if let Err(error) = result {
                    drop(command_rx);
                    outgoing.clear();
                    send_error_and_close(&event_tx, socket_id, error).await?;
                    return Ok(());
                }
                continue;
            }
            queued = command_rx.recv() => match queued {
                Some(queued) => queued,
                None => return Ok(()),
            },
        };
        let reservation = queued.reservation;
        match queued.command {
            Command::SendText(text) if closing.is_none() => {
                outgoing.push_back(Outgoing {
                    data: Payload::Bytes(text.into_bytes()),
                    opcode: FrameOpcode::Text,
                    _reservation: reservation,
                });
            }
            Command::SendBinary(bytes) if closing.is_none() => {
                outgoing.push_back(Outgoing {
                    data: Payload::Bytes(bytes),
                    opcode: FrameOpcode::Binary,
                    _reservation: reservation,
                });
            }
            Command::SendBlob(data) if closing.is_none() => {
                outgoing.push_back(Outgoing {
                    data: Payload::Blob(data),
                    opcode: FrameOpcode::Binary,
                    _reservation: reservation,
                });
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
                outgoing.clear();
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
            Command::Close { code, reason } if closing.is_none() => {
                if outgoing.is_empty() {
                    // Nothing remains to drain: end admission before the event
                    // sink can block, as in the original immediate-close path.
                    drop(command_rx);
                    drop(reservation);
                    send_event(&event_tx, Event::Closing { socket_id }).await?;
                    send_event(
                        &event_tx,
                        Event::Close {
                            socket_id,
                            code: code.unwrap_or(1005),
                            reason: code.map(|_| reason).unwrap_or_default(),
                            was_clean: true,
                        },
                    )
                    .await?;
                    return Ok(());
                }
                closing = Some((code, reason, reservation));
                send_event(&event_tx, Event::Closing { socket_id }).await?;
            }
            Command::Fail(message) => {
                drop(command_rx);
                drop(reservation);
                send_error_and_close(&event_tx, socket_id, message).await?;
                return Ok(());
            }
            _ => {}
        }
    }
}

fn response_header<'a>(
    headers: &'a moli_header_field::HeaderFields,
    name: &str,
) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .and_then(|(_, value)| std::str::from_utf8(value).ok())
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
            Vec::new().into(),
            101,
            Vec::new().into(),
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
