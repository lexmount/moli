//! Persistent WebSocket residences on their own socket-pool Multi.
//! The runtime owns perform, completion dispatch and the single wait loop.

use std::{collections::HashMap, sync::Arc, time::Instant};

use curl::{
    easy::Easy2,
    multi::{Multi, MultiWaker, WaitFd},
};
use parking_lot::Mutex;

use super::{
    SessionIo, Submission,
    endpoint::{EndpointGate, EndpointLocks},
    readiness::SocketReadiness,
    request::{self, Handshake},
    session::{Session, Step},
};
use crate::{
    CurlDnsResolution, CurlTransferId, dns_adapter::CurlDnsOwnerResidence,
    runtime::diagnostics::Diagnostics,
};

struct Pending {
    id: CurlTransferId,
    easy: Easy2<Handshake>,
    dns: CurlDnsResolution,
    deadline: Instant,
    io: SessionIo,
}

pub(crate) struct WebSocketRegistry {
    submissions: crossbeam_channel::Receiver<Submission>,
    sessions: HashMap<CurlTransferId, Session>,
    dns: CurlDnsOwnerResidence<CurlTransferId, Pending>,
    readiness: SocketReadiness,
    receive: Vec<u8>,
    closed: bool,
    owner_waker: MultiWaker,
    endpoints: Arc<Mutex<EndpointLocks>>,
}

impl WebSocketRegistry {
    pub(crate) fn new(
        submissions: crossbeam_channel::Receiver<Submission>,
        owner_waker: MultiWaker,
    ) -> Self {
        Self {
            submissions,
            sessions: HashMap::new(),
            dns: CurlDnsOwnerResidence::default(),
            readiness: SocketReadiness::default(),
            receive: Vec::new(),
            closed: false,
            owner_waker,
            endpoints: Arc::default(),
        }
    }

    pub(crate) fn submissions(&self) -> &crossbeam_channel::Receiver<Submission> {
        &self.submissions
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.sessions.is_empty() && self.dns.is_empty()
    }

    pub(crate) fn shutdown(&mut self, multi: Option<&mut Multi>) {
        if self.closed {
            return;
        }
        self.closed = true;
        if let Some(multi) = multi {
            self.fail_sessions(multi, "curl WebSocket runtime shut down");
        } else {
            debug_assert!(self.sessions.is_empty());
        }
        for pending in self.dns.drain() {
            pending
                .io
                .finish(Err("curl WebSocket runtime shut down".to_owned()));
        }
        for submission in self.submissions.try_iter() {
            submission
                .io
                .finish(Err("curl WebSocket runtime shut down".to_owned()));
        }
    }

    pub(crate) fn admit(&mut self, submission: Submission, multi: &mut Multi) {
        if submission.io.cancelled() {
            return;
        }
        if self.closed {
            submission
                .io
                .finish(Err("curl WebSocket runtime shut down".to_owned()));
            return;
        }
        let mut easy = match request::configure(&submission.request) {
            Ok(easy) => easy,
            Err(error) => {
                submission.io.finish(Err(error.to_string()));
                return;
            }
        };
        #[cfg(test)]
        {
            easy.get_mut().pool_waiting = Some(submission.io.control.pool_waiting.clone());
        }
        let Some(deadline) = Instant::now().checked_add(submission.request.handshake_timeout)
        else {
            submission
                .io
                .finish(Err("WebSocket handshake timeout is too large".to_owned()));
            return;
        };
        if let Err(error) = EndpointGate::install(
            &mut easy,
            &submission.request,
            submission.id,
            self.endpoints.clone(),
            #[cfg(test)]
            submission.io.control.endpoint_waiting.clone(),
        ) {
            submission.io.finish(Err(error.to_string()));
            return;
        }
        let pending = Pending {
            id: submission.id,
            easy,
            dns: submission.request.dns_resolution,
            deadline,
            io: submission.io,
        };
        match pending.dns.target().cloned() {
            Some(target) => self
                .dns
                .start(pending.id, pending, target, self.owner_waker.clone()),
            None => self.start(pending, multi),
        }
    }

    fn resolve_dns(&mut self, multi: &mut Multi) {
        for pending in self
            .dns
            .take_matching(|pending| pending.io.cancelled() || pending.deadline <= Instant::now())
        {
            pending
                .io
                .finish(Err("WebSocket DNS cancelled or timed out".to_owned()));
        }
        while let Some(ready) = self.dns.try_claim_next() {
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
                Ok(()) => self.start(pending, multi),
                // As with HTTP, a failed lookup or address-policy check ends
                // the request before Session::attach can add curl to Multi.
                Err(error) => pending.io.finish(Err(error)),
            }
        }
    }

    fn start(&mut self, pending: Pending, multi: &mut Multi) {
        if let Some(session) = Session::attach(
            multi,
            pending.id,
            pending.easy,
            pending.io,
            pending.deadline,
        ) {
            self.sessions.insert(pending.id, session);
        }
    }

    pub(crate) fn complete_handshake(
        &mut self,
        id: CurlTransferId,
        result: Result<(), curl::Error>,
        multi: &mut Multi,
    ) {
        if let Some(session) = self.sessions.get_mut(&id) {
            let step = session.complete_handshake(result);
            self.apply_step(id, step, multi);
        }
    }

    pub(crate) fn handshake_result(
        &self,
        id: CurlTransferId,
        message: &curl::multi::Message<'_>,
    ) -> Option<Result<(), curl::Error>> {
        self.sessions.get(&id)?.handshake_result(message)
    }

    pub(crate) fn advance(&mut self, multi: &mut Multi, diagnostics: &mut Diagnostics) -> bool {
        for _ in 0..super::SESSION_CAPACITY {
            let Ok(submission) = self.submissions.try_recv() else {
                break;
            };
            self.admit(submission, multi);
        }
        self.resolve_dns(multi);
        let mut retired = Vec::new();
        let mut progressed = false;
        for (id, session) in &mut self.sessions {
            match session.advance(&mut self.receive, diagnostics) {
                Ok(Step::Progress) => progressed = true,
                Ok(Step::Idle) => {}
                terminal => retired.push((*id, terminal)),
            }
        }
        for (id, step) in retired {
            self.apply_step(id, step, multi);
        }
        let ready = self.endpoints.lock().take_ready();
        for id in ready {
            if let Some(session) = self.sessions.get_mut(&id) {
                // Releasing a handshake lease wakes only real endpoint waiters.
                // They have no socket yet, so socket polling cannot resume them.
                progressed = true;
                if let Err(error) = session.resume_handshake() {
                    self.apply_step(id, Err(error), multi);
                }
            }
        }
        progressed
    }

    fn apply_step(&mut self, id: CurlTransferId, step: Result<Step, String>, multi: &mut Multi) {
        let result = match step {
            Ok(Step::Progress | Step::Idle) => return,
            Ok(Step::Closed) => Ok(()),
            Err(error) => Err(error),
        };
        if let Some(session) = self.sessions.remove(&id) {
            session.finish(multi, result);
        }
    }

    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.sessions
            .values()
            .filter_map(Session::handshake_deadline)
            .chain(self.dns.next_deadline(|pending| Some(pending.deadline)))
            .min()
    }

    pub(crate) fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub(crate) fn poll_fds(&mut self, handshakes: impl Iterator<Item = WaitFd>) -> &mut [WaitFd] {
        self.readiness.prepare(&self.sessions, handshakes)
    }

    pub(crate) fn apply_readiness(&mut self) {
        self.readiness.dispatch(&mut self.sessions);
    }

    pub(crate) fn fail_sessions(&mut self, multi: &mut Multi, error: &str) {
        for (_, session) in self.sessions.drain() {
            session.finish(multi, Err(error.to_owned()));
        }
    }
}
