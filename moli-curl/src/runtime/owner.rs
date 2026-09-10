//! One native owner drives both protocols. Registries own their easy handles;
//! only this loop performs curl work, drains CURLMSG_DONE and waits for readiness.

#[cfg(test)]
mod tests;

use std::{
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
    config::{make_runtime_multi, runtime_wait_timeout},
    diagnostics::Diagnostics,
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
    // Drop the registries' easy handles before their Multi, including on unwind.
    multi: Multi,
}

impl<H: Handler, C> CurlRuntimeOwner<H, C> {
    /// Construct all native state on its owner thread, including the shared pool.
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
        let _ = waker_tx.send(multi.waker());
        let mut owner = Self {
            command_rx,
            shutdown_requested,
            closed: false,
            poll_interval: config.poll_interval,
            diagnostics: Diagnostics::from_env(),
            http: HttpRegistry::new(config, completion_tx),
            websockets: WebSocketRegistry::new(websocket_rx),
            multi,
        };
        owner.drive();
    }

    fn drive(&mut self) {
        loop {
            self.drain_commands();
            self.http.advance(&mut self.multi);
            self.process_completed_transfers();
            let progressed = self
                .websockets
                .advance(&mut self.multi, &mut self.diagnostics);
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
        self.websockets.shutdown(&mut self.multi);
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
                self.websockets.admit(submission, &mut self.multi)
            }
            CurlOwnerEvent::Command(Err(_)) | CurlOwnerEvent::WebSocket(Err(_)) => self.close(),
            CurlOwnerEvent::Dns(Ok(completion)) => self.http.claim_dns_completion(completion),
            CurlOwnerEvent::Dns(Err(_)) | CurlOwnerEvent::Deadline => {}
        }
    }

    fn process_completed_transfers(&mut self) {
        if let Err(error) = self.multi.perform() {
            debug!("curl multi runtime perform failed: {error}");
            self.websockets
                .fail_sessions(&mut self.multi, &error.to_string());
        }
        // Drain CURLMSG_DONE once. A WebSocket DONE starts its open residence;
        // HTTP DONE removes a finished transfer, preserving notification order.
        let mut completed = Vec::new();
        self.multi.messages(|message| {
            let Some(id) = message.token().ok().and_then(CurlTransferId::from_token) else {
                return;
            };
            // Both registries use result_for2 to retain the easy error buffer.
            let result = self
                .http
                .completion_result(id, &message)
                .or_else(|| self.websockets.handshake_result(id, &message));
            if let Some(result) = result {
                completed.push((id, result));
            }
        });
        for (id, result) in &completed {
            if !self.http.contains(*id) {
                self.websockets
                    .complete_handshake(*id, result.clone(), &mut self.multi);
            }
        }
        self.http.complete(&mut self.multi, completed);
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
        for deadline in [self.http.next_deadline(), self.websockets.next_deadline()]
            .into_iter()
            .flatten()
        {
            timeout = timeout.min(deadline.saturating_duration_since(Instant::now()));
        }
        if runnable {
            timeout = Duration::ZERO;
        }
        // libcurl adds HTTP/handshake sockets and its waker to these open WS fds.
        let fds = self.websockets.poll_fds();
        let started = self.diagnostics.poll_start();
        let result = self.multi.poll(fds, timeout);
        self.diagnostics.polled(started, timeout, runnable);
        match result {
            Ok(_) => self.websockets.apply_readiness(),
            Err(error) => self
                .websockets
                .fail_sessions(&mut self.multi, &error.to_string()),
        }
        self.diagnostics
            .report(self.websockets.session_count(), false);
    }
}
