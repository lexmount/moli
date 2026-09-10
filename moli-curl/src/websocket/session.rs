//! One attached libcurl handle. A completed CONNECT_ONLY transfer becomes an
//! open session; it stays in Multi until finish releases the socket.

use std::{sync::atomic::Ordering, time::Instant};

use curl::{
    easy::Easy2,
    multi::{Easy2Handle, Multi, WaitFd},
};

use super::{CurlWebSocketEvent, SessionIo, WsFlags, request::Handshake};
use crate::CurlTransferId;

const CHUNK_BYTES: usize = 16 * 1024;
const IO_BUDGET: usize = 8;

/// Result of advancing one native operation. Failures use Result::Err.
pub(super) enum Step {
    Progress,
    Idle,
    /// EOF, cancellation or a dropped receiver. Browser close policy is external.
    Closed,
}

enum Phase {
    Opening { deadline: Instant },
    Open,
    ReceivedClose,
}

pub(super) struct Session {
    handle: Easy2Handle<Handshake>,
    io: SessionIo,
    phase: Phase,
}

impl Session {
    pub(super) fn attach(
        multi: &mut Multi,
        id: CurlTransferId,
        mut easy: Easy2<Handshake>,
        io: SessionIo,
        deadline: Instant,
    ) -> Option<Self> {
        if io.cancelled() {
            return None;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            io.finish(Err("WebSocket handshake timed out".to_owned()));
            return None;
        }
        // timeout belongs to the opening handshake, not the upgraded session.
        if let Err(error) = easy.timeout(remaining) {
            io.finish(Err(error.to_string()));
            return None;
        }
        match multi.add2(easy) {
            Ok(mut handle) => {
                if let Err(error) = handle.set_token(id.token()) {
                    let _ = multi.remove2(handle);
                    io.finish(Err(error.to_string()));
                    return None;
                }
                Some(Self {
                    handle,
                    io,
                    phase: Phase::Opening { deadline },
                })
            }
            Err(error) => {
                io.finish(Err(error.to_string()));
                None
            }
        }
    }

    pub(super) fn handshake_deadline(&self) -> Option<Instant> {
        match self.phase {
            Phase::Opening { deadline } => Some(deadline),
            Phase::Open | Phase::ReceivedClose => None,
        }
    }

    pub(super) fn complete_handshake(
        &mut self,
        result: Result<(), curl::Error>,
    ) -> Result<Step, String> {
        if self.handshake_deadline().is_none() {
            return Ok(Step::Idle);
        }
        let handshake = self.handle.get_mut();
        let result = match handshake.error.take() {
            Some(error) => Err(error),
            None => result.map_err(|error| error.to_string()),
        };
        let event = CurlWebSocketEvent::Handshake {
            request: std::mem::take(&mut handshake.request),
            response: std::mem::take(&mut handshake.response),
            result: result.clone(),
        };
        // No data events precede the handshake, so this queue has a free slot.
        let delivered = self.io.events.try_send(event).is_ok();
        result?;
        if !delivered {
            return Ok(Step::Closed);
        }
        self.phase = Phase::Open;
        Ok(Step::Idle)
    }

    pub(super) fn advance(&mut self) -> Result<Step, String> {
        if self.io.cancelled() {
            return Ok(Step::Closed);
        }
        if let Some(deadline) = self.handshake_deadline() {
            return if deadline <= Instant::now() {
                Err("WebSocket handshake timed out".to_owned())
            } else {
                Ok(Step::Idle)
            };
        }
        let mut progressed = false;
        for _ in 0..IO_BUDGET {
            if self.io.cancelled() {
                return Ok(Step::Closed);
            }
            // Receive delivery must never hold up a pending write.
            progressed |= self.write_pending()?;
            match self.read_chunk()? {
                Step::Progress => progressed = true,
                Step::Idle => break,
                Step::Closed => return Ok(Step::Closed),
            }
        }
        Ok(if progressed {
            Step::Progress
        } else {
            Step::Idle
        })
    }

    fn write_pending(&mut self) -> Result<bool, String> {
        // Keep the single frame resident across partial nonblocking writes.
        let mut pending = self.io.control.send.lock();
        let Some(send) = &mut *pending else {
            return Ok(false);
        };
        match self
            .handle
            .ws_send(&send.frame.data[send.offset..], 0, send.frame.flags)
        {
            Ok(count) => {
                send.offset += count;
                if send.offset == send.frame.data.len() {
                    let send = pending.take().expect("completed frame");
                    let _ = send.completed.send(send.offset);
                }
                Ok(true)
            }
            Err(error) if error.is_again() => {
                #[cfg(test)]
                self.io.control.write_blocked.notify_one();
                Ok(false)
            }
            Err(error) => Err(format!("WebSocket send failed: {error}")),
        }
    }

    fn reading_enabled(&self) -> bool {
        matches!(self.phase, Phase::Open) && self.io.control.reading.load(Ordering::Acquire)
    }

    fn read_chunk(&mut self) -> Result<Step, String> {
        if !self.reading_enabled() {
            return Ok(Step::Idle);
        }
        let permit = match self.io.events.try_reserve() {
            Ok(permit) => permit,
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                #[cfg(test)]
                self.io.control.read_blocked.notify_one();
                return Ok(Step::Idle);
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => return Ok(Step::Closed),
        };
        let mut data = vec![0; CHUNK_BYTES];
        match self.handle.ws_recv(&mut data) {
            Ok((count, frame)) => {
                data.truncate(count);
                // Let the caller answer Close before probing EOF. A peer can
                // half-close TCP in the same packet as its Close frame.
                if frame.flags().contains(WsFlags::CLOSE) && frame.bytes_left() == 0 {
                    self.phase = Phase::ReceivedClose;
                }
                permit.send(CurlWebSocketEvent::Chunk { data, frame });
                Ok(Step::Progress)
            }
            Err(error) if error.is_again() => Ok(Step::Idle),
            Err(error) if error.is_got_nothing() => Ok(Step::Closed),
            Err(error) => Err(format!("WebSocket receive failed: {error}")),
        }
    }

    pub(super) fn wait_fd(&self) -> Option<WaitFd> {
        if self.handshake_deadline().is_some() {
            return None;
        }
        let reading = self.reading_enabled() && self.io.events.capacity() > 0;
        let writing = self.io.control.send.lock().is_some();
        if !reading && !writing {
            return None;
        }
        let mut fd = WaitFd::new();
        fd.set_fd(self.handle.active_socket().ok()??);
        fd.poll_on_read(reading).poll_on_write(writing);
        Some(fd)
    }

    pub(super) fn finish(self, multi: &mut Multi, result: Result<(), String>) {
        // Physically release the socket before publishing terminal channel closure.
        let _ = multi.remove2(self.handle);
        self.io.finish(result);
    }
}
