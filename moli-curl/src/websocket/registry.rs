//! Persistent WebSocket residences on the common HTTP/WebSocket Multi.
//! The runtime owns perform, completion dispatch and the single wait loop.

use std::{
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};

use curl::{easy::Easy2, multi::Multi};

use super::{
    SessionIo, Submission,
    connection_pool::ConnectionPool,
    diagnostics::Diagnostics,
    request::{self, Handshake},
    scheduling::SocketPoll,
    session::{Session, Step},
};
use crate::{CurlDnsResolution, CurlTransferId, dns_adapter::CurlDnsOwnerResidence};

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
    poll: SocketPoll,
    receive: Vec<u8>,
    diagnostics: Diagnostics,
    pool: Option<Rc<ConnectionPool>>,
    closed: bool,
}

impl WebSocketRegistry {
    pub(crate) fn new(submissions: crossbeam_channel::Receiver<Submission>) -> Self {
        Self {
            submissions,
            sessions: HashMap::new(),
            dns: CurlDnsOwnerResidence::default(),
            poll: SocketPoll::default(),
            receive: Vec::new(),
            diagnostics: Diagnostics::from_env(),
            pool: None,
            closed: false,
        }
    }

    pub(crate) fn submissions(&self) -> &crossbeam_channel::Receiver<Submission> {
        &self.submissions
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.sessions.is_empty() && self.dns.is_empty()
    }

    pub(crate) fn shutdown(&mut self, multi: &mut Multi) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.fail_sessions(multi, "curl WebSocket runtime shut down");
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
        self.diagnostics.report(0, true);
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
        let easy = request::configure(&submission.request).and_then(|mut easy| {
            if self.pool.is_none() {
                self.pool = Some(ConnectionPool::new()?);
            }
            self.pool
                .as_ref()
                .expect("pool initialized")
                .bind(&mut easy)?;
            Ok(easy)
        });
        let easy = match easy {
            Ok(easy) => easy,
            Err(error) => {
                submission.io.finish(Err(error.to_string()));
                return;
            }
        };
        let Some(deadline) = Instant::now().checked_add(submission.request.handshake_timeout)
        else {
            submission
                .io
                .finish(Err("WebSocket handshake timeout is too large".to_owned()));
            return;
        };
        let pending = Pending {
            id: submission.id,
            easy,
            dns: submission.request.dns_resolution,
            deadline,
            io: submission.io,
        };
        match pending.dns.target().cloned() {
            Some(target) => self.dns.start(pending.id, pending, target, multi.waker()),
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

    pub(crate) fn advance(&mut self, multi: &mut Multi) -> bool {
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
            match session.advance(&mut self.receive, &mut self.diagnostics) {
                Ok(Step::Progress) => progressed = true,
                Ok(Step::Idle) => {}
                terminal => retired.push((*id, terminal)),
            }
        }
        for (id, step) in retired {
            self.apply_step(id, step, multi);
        }
        if let Some(counters) = self.diagnostics.counters() {
            counters.turns += 1;
            counters.progressed_turns += u64::from(progressed);
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

    pub(crate) fn wait(&mut self, multi: &mut Multi, timeout: Duration, progressed: bool) {
        if let Err(error) = self.poll.wait(
            multi,
            &mut self.sessions,
            timeout,
            progressed,
            &mut self.diagnostics,
        ) {
            self.fail_sessions(multi, &error.to_string());
        }
        self.diagnostics.report(self.sessions.len(), false);
    }

    pub(crate) fn fail_sessions(&mut self, multi: &mut Multi, error: &str) {
        for (_, session) in self.sessions.drain() {
            session.finish(multi, Err(error.to_owned()));
        }
    }
}
