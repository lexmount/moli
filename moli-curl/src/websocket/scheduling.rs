//! Admission and socket readiness are separate. Paused I/O needs application
//! work/capacity; WaitingForSocket I/O has already returned AGAIN. A successful
//! operation stays Runnable so libcurl's buffered data can be drained as well.

use std::{collections::HashMap, time::Duration};

use curl::multi::{Multi, WaitFd};

use super::{diagnostics::Diagnostics, session::Session};
use crate::CurlTransferId;

#[derive(Default, PartialEq, Eq)]
pub(super) enum IoState {
    #[default]
    Paused,
    Runnable,
    WaitingForSocket,
}

impl IoState {
    pub(super) fn can_run(&mut self, enabled: bool) -> bool {
        if !enabled {
            self.pause();
        } else if *self == Self::Paused {
            *self = Self::Runnable;
        }
        *self == Self::Runnable
    }

    pub(super) fn pause(&mut self) {
        *self = Self::Paused;
    }

    pub(super) fn would_block(&mut self) {
        *self = Self::WaitingForSocket;
    }

    pub(super) fn socket_ready(&mut self) {
        if self.waiting_for_socket() {
            *self = Self::Runnable;
        }
    }

    pub(super) fn waiting_for_socket(&self) -> bool {
        *self == Self::WaitingForSocket
    }
}

/// Keeps each extra fd paired with its session across curl_multi_poll.
/// libcurl also polls opening handshakes and the cross-thread waker here.
#[derive(Default)]
pub(super) struct SocketPoll {
    ids: Vec<CurlTransferId>,
    fds: Vec<WaitFd>,
}

impl SocketPoll {
    pub(super) fn wait(
        &mut self,
        multi: &Multi,
        sessions: &mut HashMap<CurlTransferId, Session>,
        timeout: Duration,
        progressed: bool,
        diagnostics: &mut Diagnostics,
    ) -> Result<(), curl::MultiError> {
        self.ids.clear();
        self.fds.clear();
        for (id, session) in sessions.iter() {
            if let Some(fd) = session.wait_fd() {
                self.ids.push(*id);
                self.fds.push(fd);
            }
        }
        let started = diagnostics.poll_start();
        let result = multi.poll(&mut self.fds, timeout);
        diagnostics.polled(started, timeout, progressed);
        result?;
        for (id, fd) in self.ids.iter().zip(&self.fds) {
            if fd.received_read() || fd.received_write() {
                // A socket event permits a retry, not guaranteed progress.
                // curl maps HUP/ERR into read/write bits; retry both directions
                // so a write-only session can also observe peer shutdown.
                sessions
                    .get_mut(id)
                    .expect("polled session is resident")
                    .socket_ready();
            }
        }
        Ok(())
    }
}
