//! Coordinates admission, DNS, handshakes and scheduling. Native frame I/O and
//! the lifetime of an attached easy handle belong to Session.

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
    multi::{Multi, MultiWaker},
};

use super::{
    SessionIo, Submission,
    diagnostics::Diagnostics,
    request::{self, Handshake},
    scheduling::SocketPoll,
    session::{Session, Step},
};
use crate::{CurlDnsResolution, CurlTransferId, dns_adapter::CurlDnsOwnerResidence};

const IDLE_WAIT: Duration = Duration::from_secs(1);

struct Pending {
    id: CurlTransferId,
    easy: Easy2<Handshake>,
    dns: CurlDnsResolution,
    deadline: Instant,
    io: SessionIo,
}

struct Owner {
    multi: Multi,
    sessions: HashMap<CurlTransferId, Session>,
    dns: CurlDnsOwnerResidence<CurlTransferId, Pending>,
    poll: SocketPoll,
    receive: Vec<u8>,
    diagnostics: Diagnostics,
}

pub(super) fn run(
    submissions: crossbeam_channel::Receiver<Submission>,
    waker_tx: crossbeam_channel::Sender<MultiWaker>,
    shutdown: Arc<AtomicBool>,
) {
    let mut owner = Owner {
        multi: Multi::new(),
        sessions: HashMap::new(),
        dns: CurlDnsOwnerResidence::default(),
        poll: SocketPoll::default(),
        receive: Vec::new(),
        diagnostics: Diagnostics::from_env(),
    };
    let _ = waker_tx.send(owner.multi.waker());
    while !shutdown.load(Ordering::Acquire) {
        owner.accept_submissions(&submissions);
        owner.resolve_dns();
        owner.advance_handshakes();
        let progressed = owner.advance_sessions();
        if let Some(counters) = owner.diagnostics.counters() {
            counters.turns += 1;
            counters.progressed_turns += u64::from(progressed);
        }
        owner.wait_for_work(progressed);
    }
    owner.fail_sessions("curl WebSocket runtime shut down");
    for pending in owner.dns.drain() {
        pending
            .io
            .finish(Err("curl WebSocket runtime shut down".to_owned()));
    }
    owner.diagnostics.report(0, true);
}

impl Owner {
    fn accept_submissions(&mut self, submissions: &crossbeam_channel::Receiver<Submission>) {
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
                Some(target) => self
                    .dns
                    .start(pending.id, pending, target, self.multi.waker()),
                None => self.start(pending),
            }
        }
    }

    fn resolve_dns(&mut self) {
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
                Ok(()) => self.start(pending),
                Err(error) => pending.io.finish(Err(error)),
            }
        }
    }

    fn start(&mut self, pending: Pending) {
        if let Some(session) = Session::attach(
            &mut self.multi,
            pending.id,
            pending.easy,
            pending.io,
            pending.deadline,
        ) {
            self.sessions.insert(pending.id, session);
        }
    }

    fn advance_handshakes(&mut self) {
        if let Err(error) = self.multi.perform() {
            self.fail_sessions(&error.to_string());
        }
        let mut completed = Vec::new();
        self.multi.messages(|message| {
            if let (Ok(token), Some(result)) = (message.token(), message.result())
                && let Some(id) = CurlTransferId::from_token(token)
            {
                completed.push((id, result));
            }
        });
        for (id, result) in completed {
            if let Some(session) = self.sessions.get_mut(&id) {
                let step = session.complete_handshake(result);
                self.apply_step(id, step);
            }
        }
    }

    fn advance_sessions(&mut self) -> bool {
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
            self.apply_step(id, step);
        }
        progressed
    }

    fn apply_step(&mut self, id: CurlTransferId, step: Result<Step, String>) {
        let result = match step {
            Ok(Step::Progress | Step::Idle) => return,
            Ok(Step::Closed) => Ok(()),
            Err(error) => Err(error),
        };
        if let Some(session) = self.sessions.remove(&id) {
            session.finish(&mut self.multi, result);
        }
    }

    fn wait_for_work(&mut self, progressed: bool) {
        let deadline = self
            .sessions
            .values()
            .filter_map(Session::handshake_deadline)
            .chain(self.dns.next_deadline(|pending| Some(pending.deadline)))
            .min();
        let timeout = if progressed {
            // Keep draining local work, but collect other sockets' readiness on
            // every turn so a busy session cannot starve a newly readable one.
            Duration::ZERO
        } else {
            deadline
                .map(|deadline| {
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(IDLE_WAIT)
                })
                .unwrap_or(IDLE_WAIT)
        };
        if let Err(error) = self.poll.wait(
            &self.multi,
            &mut self.sessions,
            timeout,
            progressed,
            &mut self.diagnostics,
        ) {
            self.fail_sessions(&error.to_string());
        }
        self.diagnostics.report(self.sessions.len(), false);
    }

    fn fail_sessions(&mut self, error: &str) {
        for (_, session) in self.sessions.drain() {
            session.finish(&mut self.multi, Err(error.to_owned()));
        }
    }
}
