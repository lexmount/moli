//! Browser session state on top of the native frame transport.
use crate::{
    Command, Event, FrameOpcode,
    commands::{
        CommandReceiver, MAX_QUEUED_BYTES, MAX_QUEUED_MESSAGES, QueuedCommand, Reservation,
    },
    events::{EventResult, EventSender, send_event},
    frames::{Assembler, Received, close_payload},
};
use moli_curl::websocket::{
    CurlWebSocketConnection, CurlWebSocketEvent, CurlWebSocketSend, MAX_SEND_FRAME_BYTES, WsFlags,
};
use std::{collections::VecDeque, future::Future, pin::Pin};
use tokio::time::{Duration, Instant};

type Delivery = Pin<Box<dyn Future<Output = EventResult> + Send>>;
type Sending = Pin<Box<dyn Future<Output = Result<Flight, String>> + Send>>;
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
// Browser delivery budget, independent of native queue capacity.
const MAX_PENDING_INCOMING_MESSAGES: usize = 9;

#[cfg(test)]
mod tests;

struct Outgoing {
    data: Vec<u8>,
    opcode: FrameOpcode,
    offset: usize,
    _reservation: Reservation,
}

#[derive(Clone, Copy)]
enum Flight {
    Data { count: usize, last: bool },
    Pong,
    Close,
}

#[derive(Default)]
struct Closing {
    requested: Option<Vec<u8>>,
    sent: bool,
    received: Option<(u16, String)>,
    deadline: Option<Instant>,
}

enum Activity {
    Sent(Result<Flight, String>),
    Delivered(EventResult),
    Command(Option<QueuedCommand>),
    Native(Option<CurlWebSocketEvent>),
    CloseTimeout,
}

struct Session {
    socket_id: u64,
    outbox: VecDeque<Event>,
    outgoing: VecDeque<Outgoing>,
    pongs: VecDeque<Vec<u8>>,
    assembler: Assembler,
    closing: Closing,
    terminal: bool,
}

pub(super) async fn run_open_session(
    socket_id: u64,
    mut connection: CurlWebSocketConnection,
    mut commands: CommandReceiver,
    event_tx: EventSender,
) -> EventResult {
    let sender = connection.sender();
    let mut reading = true;
    sender.set_reading(reading);
    let mut session = Session {
        socket_id,
        outbox: VecDeque::new(),
        outgoing: VecDeque::new(),
        pongs: VecDeque::new(),
        assembler: Assembler::default(),
        closing: Closing::default(),
        terminal: false,
    };
    let mut delivery: Option<Delivery> = None;
    let mut sending: Option<Sending> = None;
    loop {
        if session.terminal {
            // Release physical transport and all reservations before awaiting a
            // potentially blocked final event. Drop/cancel also aborts delivery.
            sender.cancel();
            sending = None;
            session.outgoing.clear();
            session.pongs.clear();
            session.assembler = Assembler::default();
        }
        if delivery.is_none()
            && let Some(event) = session.outbox.pop_front()
        {
            let sink = event_tx.clone();
            delivery = Some(Box::pin(async move { send_event(&sink, event).await }));
        }
        if session.terminal && delivery.is_none() {
            return Ok(());
        }
        if !session.terminal
            && sending.is_none()
            && let Some((frame, next_flight)) = session.next_frame()
        {
            let sender = sender.clone();
            sending = Some(Box::pin(async move {
                sender
                    .send_frame(frame)
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(next_flight)
            }));
        }
        // Pause reads at the source while application delivery is backpressured.
        let should_read = !session.terminal
            && ((delivery.is_none() && session.outbox.is_empty())
                || session.closing.requested.is_some());
        if reading != should_read {
            reading = should_read;
            sender.set_reading(reading);
        }
        let deadline = session
            .closing
            .deadline
            .unwrap_or_else(|| Instant::now() + CLOSE_TIMEOUT);
        let activity = tokio::select! {
            // Already completed writes precede newly observed incoming messages.
            // Other sources remain fair; completed message residence is bounded.
            biased;
            result = async { sending.as_mut().expect("send exists").await }, if sending.is_some() => Activity::Sent(result),
            activity = async {
                tokio::select! {
                    result = async { delivery.as_mut().expect("delivery exists").await }, if delivery.is_some() => Activity::Delivered(result),
                    command = commands.recv(), if !session.terminal => Activity::Command(command),
                    event = connection.recv(), if should_read => Activity::Native(event),
                    _ = tokio::time::sleep_until(deadline), if !session.terminal && session.closing.deadline.is_some() => Activity::CloseTimeout,
                }
            } => activity,
        };
        // The owner publishes a frame's completion before reading its echo. It
        // can finish between the select's send poll and its receive poll, so
        // recheck completion before turning native input into browser events.
        if matches!(activity, Activity::Native(_))
            && let Some(pending) = sending.as_mut()
        {
            let completion =
                std::future::poll_fn(|cx| std::task::Poll::Ready(pending.as_mut().poll(cx))).await;
            if let std::task::Poll::Ready(result) = completion {
                sending = None;
                match result {
                    Ok(completed) => session.sent(completed),
                    Err(error) => session.fail(error),
                }
                if session.terminal {
                    continue;
                }
            }
        }
        match activity {
            Activity::Sent(result) => {
                sending = None;
                match result {
                    Ok(completed) => session.sent(completed),
                    Err(error) => session.fail(error),
                }
            }
            Activity::Delivered(result) => {
                delivery = None;
                result?;
            }
            Activity::Command(Some(command)) => session.command(command),
            Activity::Command(None) => return Ok(()),
            Activity::Native(Some(CurlWebSocketEvent::Chunk { data, frame })) => {
                match session.assembler.push(data, frame) {
                    Ok(Some(received)) => session.received(received),
                    Ok(None) => {}
                    Err(error) => session.fail(error),
                }
            }
            Activity::Native(Some(CurlWebSocketEvent::Closed { result })) => {
                // Closure can race the first poll above. Terminal delivery
                // settles the pending write and rejects new sends before this await.
                if let Some(pending) = sending.take()
                    && let Ok(completed) = pending.await
                {
                    session.sent(completed);
                }
                if !session.terminal {
                    session.fail(result.err().unwrap_or_else(|| {
                        "WebSocket closed without completing the closing handshake".to_owned()
                    }));
                }
            }
            Activity::Native(Some(CurlWebSocketEvent::Handshake { .. })) => {
                session.fail("WebSocket received a duplicate handshake".to_owned())
            }
            Activity::Native(None) => session.fail("WebSocket transport stopped".to_owned()),
            Activity::CloseTimeout => {
                session.fail("WebSocket closing handshake timed out".to_owned())
            }
        }
    }
}

impl Session {
    fn command(&mut self, queued: QueuedCommand) {
        let QueuedCommand {
            command,
            reservation,
        } = queued;
        match command {
            Command::SendText(text) if self.closing.requested.is_none() => {
                self.outgoing.push_back(Outgoing {
                    data: text.into_bytes(),
                    opcode: FrameOpcode::Text,
                    offset: 0,
                    _reservation: reservation,
                })
            }
            Command::SendBinary(data) if self.closing.requested.is_none() => {
                self.outgoing.push_back(Outgoing {
                    data,
                    opcode: FrameOpcode::Binary,
                    offset: 0,
                    _reservation: reservation,
                })
            }
            Command::Close { code, reason } if self.closing.requested.is_none() => {
                match close_payload(code, reason) {
                    Ok(payload) => self.begin_close(payload, true),
                    Err(error) => self.fail(error),
                }
            }
            Command::Fail(message) => self.fail(message),
            _ => {}
        }
    }

    fn begin_close(&mut self, payload: Vec<u8>, publish_closing: bool) {
        if self.closing.requested.is_none() {
            self.closing.requested = Some(payload);
            if publish_closing {
                self.outbox.push_back(Event::Closing {
                    socket_id: self.socket_id,
                });
            }
        }
    }

    fn next_frame(&mut self) -> Option<(CurlWebSocketSend, Flight)> {
        if let Some(data) = self.pongs.pop_front() {
            return Some((
                CurlWebSocketSend {
                    flags: WsFlags::PONG,
                    data,
                },
                Flight::Pong,
            ));
        }
        if let Some(message) = self.outgoing.front() {
            // Native completion releases the data reservation, but its browser
            // notifications can still be blocked. Bound that separate residence
            // at message boundaries so a fragmented message always finishes.
            if message.offset == 0
                && self
                    .outbox
                    .iter()
                    .filter(|event| matches!(event, Event::SendCompleted { .. }))
                    .count()
                    >= MAX_QUEUED_MESSAGES
            {
                return None;
            }
            let end = (message.offset + MAX_SEND_FRAME_BYTES).min(message.data.len());
            let last = end == message.data.len();
            let mut flags = match message.opcode {
                FrameOpcode::Text => WsFlags::TEXT,
                FrameOpcode::Binary => WsFlags::BINARY,
            };
            if !last {
                flags |= WsFlags::CONT;
            }
            let data = message.data[message.offset..end].to_vec();
            return Some((
                CurlWebSocketSend { flags, data },
                Flight::Data {
                    count: end - message.offset,
                    last,
                },
            ));
        }
        if !self.closing.sent
            && let Some(data) = self.closing.requested.clone()
        {
            // Local close waits for previously accepted data. Only submitting
            // Close starts its handshake deadline, not draining that data.
            self.closing
                .deadline
                .get_or_insert_with(|| Instant::now() + CLOSE_TIMEOUT);
            return Some((
                CurlWebSocketSend {
                    flags: WsFlags::CLOSE,
                    data,
                },
                Flight::Close,
            ));
        }
        None
    }

    fn received(&mut self, received: Received) {
        let socket_id = self.socket_id;
        match received {
            Received::Text(data) => self.queue_message(Event::TextMessage { socket_id, data }),
            Received::Binary(data) => self.queue_message(Event::BinaryMessage { socket_id, data }),
            Received::Ping(data) => {
                if self.pongs.len() == 4 {
                    self.fail("WebSocket control queue capacity exceeded".to_owned());
                } else if !self.closing.sent {
                    self.pongs.push_back(data);
                }
            }
            Received::Close {
                code,
                reason,
                payload,
            } => {
                self.closing.received = Some((code, reason));
                // A peer Close already starts the handshake, even if a data
                // frame is still in flight. Do not extend an existing deadline.
                self.closing
                    .deadline
                    .get_or_insert_with(|| Instant::now() + CLOSE_TIMEOUT);
                self.outgoing.clear();
                self.pongs.clear();
                self.begin_close(payload, false);
                self.finish_close();
            }
            _ => {}
        }
    }

    fn queue_message(&mut self, event: Event) {
        fn message_size(event: &Event) -> Option<usize> {
            match event {
                Event::TextMessage { data, .. } => Some(data.len()),
                Event::BinaryMessage { data, .. } => Some(data.len()),
                _ => None,
            }
        }
        // Preserve chunks already admitted when native reads paused. Closing
        // also keeps reading control frames; its message backlog stays bounded.
        let (count, bytes) = self
            .outbox
            .iter()
            .filter_map(message_size)
            .fold((0, 0usize), |(count, bytes), len| (count + 1, bytes + len));
        let size = message_size(&event).expect("only message events use this queue");
        if count >= MAX_PENDING_INCOMING_MESSAGES || bytes.saturating_add(size) > MAX_QUEUED_BYTES {
            self.fail("WebSocket delivery queue capacity exceeded".to_owned());
        } else {
            self.outbox.push_back(event);
        }
    }

    fn sent(&mut self, flight: Flight) {
        match flight {
            Flight::Data { count, last } => {
                // A remote Close may have discarded the rest of this message.
                if let Some(message) = self.outgoing.front_mut() {
                    message.offset += count;
                    if last {
                        let message = self
                            .outgoing
                            .pop_front()
                            .expect("completed message is resident");
                        self.outbox.push_back(Event::SendCompleted {
                            socket_id: self.socket_id,
                            opcode: message.opcode,
                            payload_length: message.data.len(),
                        });
                        // The admission reservation drops only at native completion.
                    }
                }
            }
            Flight::Close => {
                self.closing.sent = true;
                self.finish_close();
            }
            Flight::Pong => {}
        }
    }

    fn finish_close(&mut self) {
        if self.closing.sent
            && let Some((code, reason)) = self.closing.received.take()
        {
            self.terminal = true;
            self.outbox.push_back(Event::Close {
                socket_id: self.socket_id,
                code,
                reason,
                was_clean: true,
            });
        }
    }

    fn fail(&mut self, message: String) {
        if self.terminal {
            return;
        }
        self.terminal = true;
        self.outbox.push_back(Event::Error {
            socket_id: self.socket_id,
            message,
        });
        self.outbox.push_back(Event::Close {
            socket_id: self.socket_id,
            code: 1006,
            reason: String::new(),
            was_clean: false,
        });
    }
}
