use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use curl::{
    easy::Easy2,
    multi::{Easy2Handle, Multi, MultiWaker, WaitFd},
};

use super::{
    CurlWebSocketEvent, SessionIo, Submission,
    request::{self, Handshake},
};
use crate::{CurlDnsResolution, CurlTransferId, dns_adapter::CurlDnsOwnerResidence};

const CHUNK_BYTES: usize = 16 * 1024;
const IO_BUDGET: usize = 8;
const IDLE_WAIT: Duration = Duration::from_secs(1);

struct Pending {
    id: CurlTransferId,
    easy: Easy2<Handshake>,
    dns: CurlDnsResolution,
    deadline: Instant,
    io: SessionIo,
}

struct Session {
    handle: Easy2Handle<Handshake>,
    deadline: Instant,
    open: bool,
    received_close: bool,
    io: SessionIo,
}

pub(super) fn run(
    submissions: crossbeam_channel::Receiver<Submission>,
    waker_tx: crossbeam_channel::Sender<MultiWaker>,
    shutdown: Arc<AtomicBool>,
) {
    let mut multi = Multi::new();
    let _ = waker_tx.send(multi.waker());
    let mut sessions = HashMap::<CurlTransferId, Session>::new();
    let mut dns = CurlDnsOwnerResidence::<CurlTransferId, Pending>::default();
    while !shutdown.load(Ordering::Acquire) {
        for submission in submissions.try_iter().take(super::SESSION_CAPACITY) {
            if submission.io.cancelled() {
                continue;
            }
            let easy = match request::configure(&submission.request) {
                Ok(easy) => easy,
                Err(error) => {
                    submission.io.finish(Err(error.to_string()));
                    continue;
                }
            };
            let Some(deadline) = Instant::now().checked_add(submission.request.handshake_timeout)
            else {
                submission
                    .io
                    .finish(Err("WebSocket handshake timeout is too large".to_owned()));
                continue;
            };
            let pending = Pending {
                id: submission.id,
                easy,
                dns: submission.request.dns_resolution,
                deadline,
                io: submission.io,
            };
            match pending.dns.target().cloned() {
                Some(target) => dns.start(pending.id, pending, target, multi.waker()),
                None => start(&mut multi, &mut sessions, pending),
            }
        }
        for pending in dns
            .take_matching(|pending| pending.io.cancelled() || pending.deadline <= Instant::now())
        {
            pending
                .io
                .finish(Err("WebSocket DNS cancelled or timed out".to_owned()));
        }
        while let Some(ready) = dns.try_claim_next() {
            let mut pending = ready.pending;
            if pending.io.cancelled() {
                continue;
            }
            let result = ready
                .result
                .map_err(|error| error.to_string())
                .and_then(|addresses| {
                    pending
                        .dns
                        .install(&mut pending.easy, &addresses)
                        .map_err(|error| error.to_string())
                });
            match result {
                Ok(()) => start(&mut multi, &mut sessions, pending),
                Err(error) => pending.io.finish(Err(error)),
            }
        }
        if let Err(error) = multi.perform() {
            for (_, session) in sessions.drain() {
                finish(&mut multi, session, Err(error.to_string()));
            }
        }
        let mut completed = Vec::new();
        multi.messages(|message| {
            if let (Ok(token), Some(result)) = (message.token(), message.result())
                && let Some(id) = CurlTransferId::from_token(token)
            {
                completed.push((id, result));
            }
        });
        for (id, result) in completed {
            let Some(session) = sessions.get_mut(&id) else {
                continue;
            };
            if session.open {
                continue;
            }
            let handshake = session.handle.get_mut();
            let native_result = match handshake.error.take() {
                Some(error) => Err(error),
                None => result.map_err(|error| error.to_string()),
            };
            let event = CurlWebSocketEvent::Handshake {
                request: std::mem::take(&mut handshake.request),
                response: std::mem::take(&mut handshake.response),
                result: native_result.clone(),
            };
            // No data events precede the handshake, so this queue has a free slot.
            if session.io.events.try_send(event).is_err() || native_result.is_err() {
                let session = sessions.remove(&id).expect("completed session is resident");
                finish(&mut multi, session, native_result);
            } else {
                session.open = true;
            }
        }

        let mut retired = Vec::new();
        let mut progressed = false;
        for (id, session) in &mut sessions {
            if session.io.cancelled() {
                retired.push((*id, Ok(())));
            } else if !session.open && session.deadline <= Instant::now() {
                retired.push((*id, Err("WebSocket handshake timed out".to_owned())));
            } else if session.open {
                match session.drive() {
                    Ok(progress) => progressed |= progress,
                    Err(result) => retired.push((*id, result)),
                }
            }
        }
        for (id, result) in retired {
            if let Some(session) = sessions.remove(&id) {
                finish(&mut multi, session, result);
            }
        }
        if progressed {
            continue;
        }

        let mut fds: Vec<_> = sessions.values().filter_map(Session::wait_fd).collect();
        let deadline = sessions
            .values()
            .filter(|session| !session.open)
            .map(|session| session.deadline)
            .chain(dns.next_deadline(|pending| Some(pending.deadline)))
            .min();
        let timeout = deadline
            .map(|deadline| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(IDLE_WAIT)
            })
            .unwrap_or(IDLE_WAIT);
        // poll includes libcurl's handshake sockets plus our upgraded sockets.
        // Completed CONNECT_ONLY handles deliberately remain attached to this Multi.
        if let Err(error) = multi.poll(&mut fds, timeout) {
            for (_, session) in sessions.drain() {
                finish(&mut multi, session, Err(error.to_string()));
            }
        }
    }
    for (_, session) in sessions.drain() {
        finish(
            &mut multi,
            session,
            Err("curl WebSocket runtime shut down".to_owned()),
        );
    }
    for pending in dns.drain() {
        pending
            .io
            .finish(Err("curl WebSocket runtime shut down".to_owned()));
    }
}

fn start(multi: &mut Multi, sessions: &mut HashMap<CurlTransferId, Session>, mut pending: Pending) {
    if pending.io.cancelled() {
        return;
    }
    let remaining = pending.deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        pending
            .io
            .finish(Err("WebSocket handshake timed out".to_owned()));
        return;
    }
    // timeout belongs to the transfer (opening handshake), not the upgraded session.
    if let Err(error) = pending.easy.timeout(remaining) {
        pending.io.finish(Err(error.to_string()));
        return;
    }
    match multi.add2(pending.easy) {
        Ok(mut handle) => {
            if let Err(error) = handle.set_token(pending.id.token()) {
                let _ = multi.remove2(handle);
                pending.io.finish(Err(error.to_string()));
                return;
            }
            sessions.insert(
                pending.id,
                Session {
                    handle,
                    deadline: pending.deadline,
                    open: false,
                    received_close: false,
                    io: pending.io,
                },
            );
        }
        Err(error) => pending.io.finish(Err(error.to_string())),
    }
}

fn finish(multi: &mut Multi, session: Session, result: std::result::Result<(), String>) {
    // Physically release the socket before publishing terminal channel closure.
    let _ = multi.remove2(session.handle);
    session.io.finish(result);
}

impl Session {
    /// Err carries the terminal transport result; Ok reports useful work only.
    fn drive(&mut self) -> std::result::Result<bool, std::result::Result<(), String>> {
        let mut progressed = false;
        for _ in 0..IO_BUDGET {
            if self.io.cancelled() {
                return Err(Ok(()));
            }
            {
                // Keep the single frame resident across partial nonblocking writes.
                let mut pending = self.io.control.send.lock();
                if let Some(send) = &mut *pending {
                    match self
                        .handle
                        .ws_send(&send.frame.data[send.offset..], 0, send.frame.flags)
                    {
                        Ok(count) => {
                            send.offset += count;
                            progressed = true;
                            if send.offset == send.frame.data.len() {
                                let send = pending.take().expect("completed frame");
                                let _ = send.completed.send(send.offset);
                            }
                        }
                        Err(error) if error.is_again() => {
                            #[cfg(test)]
                            self.io.control.write_blocked.notify_one();
                        }
                        Err(error) => return Err(Err(format!("WebSocket send failed: {error}"))),
                    }
                }
            }
            if self.received_close || !self.io.control.reading.load(Ordering::Acquire) {
                break;
            }
            let permit = match self.io.events.try_reserve() {
                Ok(permit) => permit,
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    #[cfg(test)]
                    self.io.control.read_blocked.notify_one();
                    break;
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => return Err(Ok(())),
            };
            let mut data = vec![0; CHUNK_BYTES];
            match self.handle.ws_recv(&mut data) {
                Ok((count, frame)) => {
                    data.truncate(count);
                    // Let the caller answer Close before probing EOF. A peer can
                    // half-close TCP in the same packet as its Close frame.
                    self.received_close =
                        frame.flags().contains(super::WsFlags::CLOSE) && frame.bytes_left() == 0;
                    permit.send(CurlWebSocketEvent::Chunk { data, frame });
                    progressed = true;
                }
                Err(error) if error.is_again() => break,
                Err(error) if error.is_got_nothing() => return Err(Ok(())),
                Err(error) => return Err(Err(format!("WebSocket receive failed: {error}"))),
            }
        }
        Ok(progressed)
    }

    fn wait_fd(&self) -> Option<WaitFd> {
        if !self.open {
            return None;
        }
        let reading = self.io.events.capacity() > 0
            && !self.received_close
            && self.io.control.reading.load(Ordering::Acquire);
        let writing = self.io.control.send.lock().is_some();
        if !reading && !writing {
            return None;
        }
        let mut fd = WaitFd::new();
        fd.set_fd(self.handle.active_socket().ok()??);
        fd.poll_on_read(reading).poll_on_write(writing);
        Some(fd)
    }
}
