//! One native owner drives both protocols. Registries own their easy handles;
//! only this loop performs curl work, drains CURLMSG_DONE and waits for readiness.

#[cfg(test)]
mod tests;

use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender};
use curl::{
    easy::Handler,
    multi::{Multi, MultiWaker},
};
use tracing::debug;

use super::{
    CurlMultiRuntimeConfig, CurlRuntimeCommand,
    config::{make_runtime_multi, make_websocket_multi, runtime_wait_timeout},
    diagnostics::Diagnostics,
    poll_fds::MultiPollFds,
};
use crate::{
    CurlMultiCompletion, CurlTransferId,
    dns_adapter::CurlDnsOwnerCompletion,
    http::registry::HttpRegistry,
    websocket::{Submission, registry::WebSocketRegistry},
};

enum CurlOwnerEvent<H: Handler, C> {
    Command(Result<CurlRuntimeCommand<H, C>, crossbeam_channel::RecvError>),
    Dns(Result<CurlDnsOwnerCompletion<CurlTransferId>, crossbeam_channel::RecvError>),
    WebSocket(Result<Submission, crossbeam_channel::RecvError>),
    Deadline,
}

pub(super) struct CurlRuntimeOwner<H: Handler, C> {
    command_rx: Receiver<CurlRuntimeCommand<H, C>>,
    shutdown_requested: Arc<AtomicBool>,
    closed: bool,
    poll_interval: Duration,
    diagnostics: Diagnostics,
    http: HttpRegistry<H, C>,
    websockets: WebSocketRegistry,
    max_websocket_connections: NonZeroUsize,
    websocket_poll_fds: MultiPollFds,
    // Drop the registries' easy handles before their Multi, including on unwind.
    multi: Multi,
    // Allocated only when the first WebSocket is admitted. HTTP-only runtimes
    // keep their existing single Multi and owner-thread footprint.
    websocket_multi: Option<Multi>,
}

impl<H: Handler, C> CurlRuntimeOwner<H, C> {
    /// Construct all native state on its owner thread.
    pub(super) fn run(
        config: CurlMultiRuntimeConfig,
        command_rx: Receiver<CurlRuntimeCommand<H, C>>,
        completion_tx: Sender<CurlMultiCompletion<H, C>>,
        waker_tx: Sender<MultiWaker>,
        shutdown_requested: Arc<AtomicBool>,
        websocket_rx: Receiver<Submission>,
        #[cfg(test)] owner_started: Arc<AtomicBool>,
    ) {
        #[cfg(test)]
        owner_started.store(true, Ordering::SeqCst);
        let multi = make_runtime_multi(&config);
        let owner_waker = multi.waker();
        let max_websocket_connections = config.max_websocket_connections;
        let _ = waker_tx.send(multi.waker());
        let mut owner = Self {
            command_rx,
            shutdown_requested,
            closed: false,
            poll_interval: config.poll_interval,
            diagnostics: Diagnostics::from_env(),
            http: HttpRegistry::new(config, completion_tx),
            websockets: WebSocketRegistry::new(websocket_rx, owner_waker),
            max_websocket_connections,
            websocket_poll_fds: MultiPollFds::default(),
            multi,
            websocket_multi: None,
        };
        owner.drive();
    }

    fn drive(&mut self) {
        loop {
            self.drain_commands();
            self.http.advance(&mut self.multi);
            self.process_completed_transfers();
            if self.websocket_multi.is_none() && !self.websockets.submissions().is_empty() {
                self.websocket_multi = Some(make_websocket_multi(self.max_websocket_connections));
            }
            let progressed = self
                .websocket_multi
                .as_mut()
                .is_some_and(|multi| self.websockets.advance(multi, &mut self.diagnostics));
            if let Some(counters) = self.diagnostics.counters() {
                counters.turns += 1;
                counters.progressed_turns += u64::from(progressed);
            }

            // close() retires both registries. Drain racing HTTP submissions so
            // every accepted job still receives its terminal completion.
            if self.closed && self.command_rx.is_empty() {
                self.diagnostics.report(0, true);
                return;
            }

            // HTTP completions can release slots after advance() has run.
            let runnable =
                progressed || !self.command_rx.is_empty() || self.http.has_eligible_jobs();
            if !runnable && !self.http.has_curl_work() && self.websockets.is_empty() {
                self.wait_for_next_owner_event();
            } else {
                self.wait_for_curl_progress(runnable);
            }
        }
    }

    fn drain_commands(&mut self) {
        // Continuous HTTP submissions must not starve native I/O or WebSockets.
        for _ in 0..256 {
            match self.command_rx.try_recv() {
                Ok(command) => self.handle_command(command),
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.close();
                    break;
                }
            }
        }
    }

    fn handle_command(&mut self, command: CurlRuntimeCommand<H, C>) {
        match command {
            CurlRuntimeCommand::Request { transfer_id, job } => self.http.admit(transfer_id, job),
            CurlRuntimeCommand::Shutdown => self.close(),
        }
    }

    fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.shutdown_requested.store(true, Ordering::SeqCst);
        self.http.shutdown(&mut self.multi);
        self.websockets.shutdown(self.websocket_multi.as_mut());
    }

    fn wait_for_next_owner_event(&mut self) {
        if self.closed {
            return;
        }
        let event = if let Some(deadline) = self.http.next_deadline() {
            let deadline_rx =
                crossbeam_channel::after(deadline.saturating_duration_since(Instant::now()));
            crossbeam_channel::select! {
                recv(self.command_rx) -> command => CurlOwnerEvent::Command(command),
                recv(self.websockets.submissions()) -> submission => CurlOwnerEvent::WebSocket(submission),
                recv(self.http.dns_completions()) -> completion => CurlOwnerEvent::Dns(completion),
                recv(deadline_rx) -> _ => CurlOwnerEvent::Deadline,
            }
        } else {
            crossbeam_channel::select! {
                recv(self.command_rx) -> command => CurlOwnerEvent::Command(command),
                recv(self.websockets.submissions()) -> submission => CurlOwnerEvent::WebSocket(submission),
                recv(self.http.dns_completions()) -> completion => CurlOwnerEvent::Dns(completion),
            }
        };
        match event {
            CurlOwnerEvent::Command(Ok(command)) => self.handle_command(command),
            CurlOwnerEvent::WebSocket(Ok(submission)) => {
                let multi = self
                    .websocket_multi
                    .get_or_insert_with(|| make_websocket_multi(self.max_websocket_connections));
                self.websockets.admit(submission, multi)
            }
            CurlOwnerEvent::Command(Err(_)) | CurlOwnerEvent::WebSocket(Err(_)) => self.close(),
            CurlOwnerEvent::Dns(Ok(completion)) => self.http.claim_dns_completion(completion),
            CurlOwnerEvent::Dns(Err(_)) | CurlOwnerEvent::Deadline => {}
        }
    }

    fn process_completed_transfers(&mut self) {
        if let Err(error) = self.multi.perform() {
            debug!("curl multi runtime perform failed: {error}");
        }
        let mut completed = Vec::new();
        self.multi.messages(|message| {
            let Some(id) = message.token().ok().and_then(CurlTransferId::from_token) else {
                return;
            };
            if let Some(result) = self.http.completion_result(id, &message) {
                completed.push((id, result));
            }
        });
        self.http.complete(&mut self.multi, completed);

        let Some(multi) = &mut self.websocket_multi else {
            return;
        };
        if let Err(error) = multi.perform() {
            debug!("curl WebSocket runtime perform failed: {error}");
            self.websockets.fail_sessions(multi, &error.to_string());
        }
        // A WebSocket DONE starts its open residence. Its easy handle must
        // stay attached to this pool until the WebSocket actually terminates.
        let mut completed = Vec::new();
        multi.messages(|message| {
            let Some(id) = message.token().ok().and_then(CurlTransferId::from_token) else {
                return;
            };
            if let Some(result) = self.websockets.handshake_result(id, &message) {
                completed.push((id, result));
            }
        });
        for (id, result) in completed {
            self.websockets.complete_handshake(id, result, multi);
        }
    }

    fn wait_for_curl_progress(&mut self, runnable: bool) {
        // HTTP cancellation uses its configured progress interval. Idle WS
        // sessions use socket/waker readiness and need not inherit that cadence.
        let interval = if self.http.has_active() {
            self.poll_interval
        } else {
            Duration::from_secs(1)
        };
        let mut timeout = runtime_wait_timeout(&self.multi, interval).unwrap_or(interval);
        if let Some(multi) = &mut self.websocket_multi {
            timeout = timeout.min(runtime_wait_timeout(multi, interval).unwrap_or(interval));
            if let Err(error) = self.websocket_poll_fds.refresh(multi) {
                self.websockets.fail_sessions(multi, &error.to_string());
            }
        }
        for deadline in [self.http.next_deadline(), self.websockets.next_deadline()]
            .into_iter()
            .flatten()
        {
            timeout = timeout.min(deadline.saturating_duration_since(Instant::now()));
        }
        if runnable {
            timeout = Duration::ZERO;
        }
        // One poll waits for HTTP, WebSocket handshakes, open WS I/O and the
        // common owner waker. Both registries and DNS use that same waker.
        let fds = self.websockets.poll_fds(self.websocket_poll_fds.iter());
        let started = self.diagnostics.poll_start();
        let result = self.multi.poll(fds, timeout);
        self.diagnostics.polled(started, timeout, runnable);
        match result {
            Ok(_) => self.websockets.apply_readiness(),
            Err(error) => {
                if let Some(multi) = &mut self.websocket_multi {
                    self.websockets.fail_sessions(multi, &error.to_string());
                }
            }
        }
        self.diagnostics
            .report(self.websockets.session_count(), false);
    }
}
