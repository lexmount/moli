use std::{
    num::NonZeroUsize,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use crossbeam_channel::{self, Receiver, Sender};
use curl::{
    easy::Handler,
    multi::{Multi, MultiWaker},
};
use tracing::debug;

use crate::dns_adapter::{CurlDnsOwnerCompletion, CurlDnsReady};

use super::{
    CurlMultiCompletion, CurlMultiJob, CurlMultiRuntimeConfig, CurlTransferId,
    config::{make_runtime_multi, runtime_wait_timeout},
    residence::{
        CurlActiveTransfer, CurlOwnerState, CurlPendingJob, active_origin_count,
        completed_transfers, enqueue_existing_pending_job, enqueue_pending_job, job_is_eligible,
        pending_origin_count, take_expired_pending_jobs, take_transfers_in_notification_order,
    },
};

#[derive(Debug)]
pub(super) enum CurlRuntimeCommand<H: Handler, C> {
    Request {
        transfer_id: CurlTransferId,
        job: CurlMultiJob<H, C>,
    },
    Shutdown,
}

enum CurlOwnerEvent<H: Handler, C> {
    Command(std::result::Result<CurlRuntimeCommand<H, C>, crossbeam_channel::RecvError>),
    Dns(std::result::Result<CurlDnsOwnerCompletion<CurlTransferId>, crossbeam_channel::RecvError>),
    Deadline,
}

pub(super) struct CurlRuntimeOwner<H: Handler + Send + 'static, C: Send + 'static> {
    config: CurlMultiRuntimeConfig,
    command_rx: Receiver<CurlRuntimeCommand<H, C>>,
    completion_tx: Sender<CurlMultiCompletion<H, C>>,
    waker_tx: Sender<MultiWaker>,
    shutdown_requested: Arc<AtomicBool>,
    #[cfg(test)]
    owner_started: Arc<AtomicBool>,
}

impl<H: Handler + Send + 'static, C: Send + 'static> CurlRuntimeOwner<H, C> {
    pub(super) fn new(
        config: CurlMultiRuntimeConfig,
        command_rx: Receiver<CurlRuntimeCommand<H, C>>,
        completion_tx: Sender<CurlMultiCompletion<H, C>>,
        waker_tx: Sender<MultiWaker>,
        shutdown_requested: Arc<AtomicBool>,
        #[cfg(test)] owner_started: Arc<AtomicBool>,
    ) -> Self {
        Self {
            config,
            command_rx,
            completion_tx,
            waker_tx,
            shutdown_requested,
            #[cfg(test)]
            owner_started,
        }
    }

    pub(super) fn thread_name(&self) -> &str {
        &self.config.thread_name
    }

    pub(super) fn run(self) {
        #[cfg(test)]
        self.owner_started.store(true, Ordering::SeqCst);
        let mut multi = make_runtime_multi(&self.config);
        let _ = self.waker_tx.send(multi.waker());
        let mut state = CurlOwnerState::default();

        loop {
            self.drain_commands(&mut state, &mut multi);
            self.drain_dns_completions(&mut state);
            self.expire_waiting_jobs(&mut state);
            self.start_eligible_jobs(&mut state, &mut multi);
            self.process_completed_transfers(&mut state, &mut multi);

            if state.closed
                && state.pending.is_empty()
                && state.dns.is_empty()
                && state.active.is_empty()
            {
                return;
            }

            if state.active.is_empty() && state.pending.is_empty() {
                self.wait_for_next_owner_event(&mut state, &mut multi);
            } else if !state.active.is_empty() {
                self.wait_for_curl_progress(&multi, &state);
            }
        }
    }

    fn drain_commands(&self, state: &mut CurlOwnerState<H, C>, multi: &mut Multi) {
        loop {
            match self.command_rx.try_recv() {
                Ok(command) => self.handle_command(state, multi, command),
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.close(state, multi);
                    break;
                }
            }
        }
    }

    fn wait_for_next_owner_event(&self, state: &mut CurlOwnerState<H, C>, multi: &mut Multi) {
        if state.closed {
            return;
        }
        if state.dns.is_empty() {
            match self.command_rx.recv() {
                Ok(command) => self.handle_command(state, multi, command),
                Err(_) => self.close(state, multi),
            }
            return;
        }
        let event = if let Some(deadline) = state.dns.next_deadline(|pending| pending.job.deadline)
        {
            let deadline_rx =
                crossbeam_channel::after(deadline.saturating_duration_since(Instant::now()));
            crossbeam_channel::select! {
                recv(self.command_rx) -> command => CurlOwnerEvent::Command(command),
                recv(state.dns.completion_receiver()) -> completion => CurlOwnerEvent::Dns(completion),
                recv(deadline_rx) -> _ => CurlOwnerEvent::Deadline,
            }
        } else {
            crossbeam_channel::select! {
                recv(self.command_rx) -> command => CurlOwnerEvent::Command(command),
                recv(state.dns.completion_receiver()) -> completion => CurlOwnerEvent::Dns(completion),
            }
        };
        match event {
            CurlOwnerEvent::Command(command) => match command {
                Ok(command) => self.handle_command(state, multi, command),
                Err(_) => self.close(state, multi),
            },
            CurlOwnerEvent::Dns(completion) => {
                if let Ok(completion) = completion {
                    self.claim_dns_completion(state, completion);
                }
            }
            CurlOwnerEvent::Deadline => {}
        }
    }

    fn handle_command(
        &self,
        state: &mut CurlOwnerState<H, C>,
        multi: &mut Multi,
        command: CurlRuntimeCommand<H, C>,
    ) {
        match command {
            CurlRuntimeCommand::Request { transfer_id, job } if state.closed => {
                self.send_completion(CurlMultiCompletion {
                    transfer_id,
                    easy: Some(job.easy),
                    context: job.context,
                    result: Err(anyhow!("curl multi runtime is shutting down")),
                });
            }
            CurlRuntimeCommand::Request { transfer_id, job } => {
                self.admit_job(state, transfer_id, job)
            }
            CurlRuntimeCommand::Shutdown => self.close(state, multi),
        }
    }

    fn admit_job(
        &self,
        state: &mut CurlOwnerState<H, C>,
        transfer_id: CurlTransferId,
        job: CurlMultiJob<H, C>,
    ) {
        if curl_runtime_trace_enabled() {
            let origin = job.origin.as_ref();
            tracing::info!(
                target: "moli_cdp_nav_timing",
                transfer_id = %transfer_id,
                label = %job.label,
                origin_scheme = origin.map(|origin| origin.scheme.as_str()).unwrap_or(""),
                origin_host = origin.map(|origin| origin.host.as_str()).unwrap_or(""),
                origin_port = ?origin.and_then(|origin| origin.port),
                origin = ?job.origin,
                priority = job.priority,
                pending_before = state.pending.len(),
                pending_same_origin_before = origin
                    .map(|origin| pending_origin_count(&state.pending, origin))
                    .unwrap_or(0),
                stage = "curl_runtime_job_queued",
            );
        }
        enqueue_pending_job(&mut state.pending, transfer_id, job);
    }

    fn close(&self, state: &mut CurlOwnerState<H, C>, multi: &mut Multi) {
        if state.closed {
            return;
        }
        state.closed = true;
        self.shutdown_requested.store(true, Ordering::SeqCst);
        while let Some(pending) = state.pending.pop_front() {
            let CurlPendingJob {
                transfer_id, job, ..
            } = pending;
            self.send_completion(CurlMultiCompletion {
                transfer_id,
                easy: Some(job.easy),
                context: job.context,
                result: Err(anyhow!("curl multi runtime is shutting down")),
            });
        }
        for pending in state.dns.drain() {
            let CurlPendingJob {
                transfer_id, job, ..
            } = pending;
            self.send_completion(CurlMultiCompletion {
                transfer_id,
                easy: Some(job.easy),
                context: job.context,
                result: Err(anyhow!(
                    "curl multi runtime DNS request cancelled during shutdown"
                )),
            });
        }
        for (transfer_id, active) in state.active.drain() {
            let easy = multi.remove2(active.handle).ok();
            self.send_completion(CurlMultiCompletion {
                transfer_id,
                easy,
                context: active.context,
                result: Err(anyhow!(
                    "curl multi runtime request cancelled during shutdown"
                )),
            });
        }
    }

    fn start_eligible_jobs(&self, state: &mut CurlOwnerState<H, C>, multi: &mut Multi) {
        loop {
            if state.closed || state.active.len() >= self.config.max_active.get() {
                return;
            }
            let Some(index) = state.pending.iter().position(|pending| {
                job_is_eligible(
                    pending.job.origin.as_ref(),
                    state,
                    self.config.max_host_active,
                )
            }) else {
                return;
            };
            let pending = state
                .pending
                .remove(index)
                .expect("pending curl job index should exist");
            let dns_target = pending.job.dns_resolution.target().cloned();
            match dns_target {
                Some(target) => {
                    state
                        .dns
                        .start(pending.transfer_id, pending, target, multi.waker())
                }
                None => self.start_job(state, multi, pending),
            }
        }
    }

    fn drain_dns_completions(&self, state: &mut CurlOwnerState<H, C>) {
        while let Some(ready) = state.dns.try_claim_next() {
            self.handle_dns_completion(state, ready);
        }
    }

    fn claim_dns_completion(
        &self,
        state: &mut CurlOwnerState<H, C>,
        completion: CurlDnsOwnerCompletion<CurlTransferId>,
    ) {
        let Some(ready) = state.dns.claim(completion) else {
            return;
        };
        self.handle_dns_completion(state, ready);
    }

    fn handle_dns_completion(
        &self,
        state: &mut CurlOwnerState<H, C>,
        ready: CurlDnsReady<CurlPendingJob<H, C>>,
    ) {
        let mut pending = ready.pending;
        if state.closed {
            let CurlPendingJob {
                transfer_id, job, ..
            } = pending;
            self.send_completion(CurlMultiCompletion {
                transfer_id,
                easy: Some(job.easy),
                context: job.context,
                result: Err(anyhow!("curl multi runtime is shutting down")),
            });
            return;
        }
        if pending.deadline_reached(Instant::now()) {
            self.complete_timed_out_job(pending, "while waiting for DNS");
            return;
        }
        match ready.result {
            Ok(addresses) => {
                if let Err(error) = pending
                    .job
                    .dns_resolution
                    .install(&mut pending.job.easy, addresses.as_ref())
                {
                    let CurlPendingJob {
                        transfer_id, job, ..
                    } = pending;
                    self.send_completion(CurlMultiCompletion {
                        transfer_id,
                        easy: Some(job.easy),
                        context: job.context,
                        result: Err(error),
                    });
                    return;
                }
                enqueue_existing_pending_job(&mut state.pending, pending);
            }
            Err(error) => {
                let CurlPendingJob {
                    transfer_id, job, ..
                } = pending;
                self.send_completion(CurlMultiCompletion {
                    transfer_id,
                    easy: Some(job.easy),
                    context: job.context,
                    result: Err(anyhow!(error.to_string())),
                });
            }
        }
    }

    fn start_job(
        &self,
        state: &mut CurlOwnerState<H, C>,
        multi: &mut Multi,
        pending: CurlPendingJob<H, C>,
    ) {
        if pending.deadline_reached(Instant::now()) {
            self.complete_timed_out_job(pending, "while waiting to start");
            return;
        }
        let transfer_id = pending.transfer_id;
        let queued_for = pending.enqueued_at.elapsed();
        let mut job = pending.job;
        let label = job.label.clone();
        if let Some(deadline) = job.deadline {
            let Some(remaining) = curl_timeout_for_deadline(deadline, Instant::now()) else {
                self.send_completion(CurlMultiCompletion {
                    transfer_id,
                    easy: Some(job.easy),
                    context: job.context,
                    result: Err(curl_runtime_timeout_error("while waiting to start")),
                });
                return;
            };
            if let Err(error) = job.easy.timeout(remaining) {
                self.send_completion(CurlMultiCompletion {
                    transfer_id,
                    easy: Some(job.easy),
                    context: job.context,
                    result: Err(error).context("failed to apply remaining curl request deadline"),
                });
                return;
            }
        }
        match multi
            .add2(job.easy)
            .with_context(|| anyhow!("failed to add curl easy handle for {label}"))
        {
            Ok(mut handle) => {
                handle
                    .set_token(transfer_id.token())
                    .expect("active curl handle must accept its transfer token");
                if curl_runtime_trace_enabled() {
                    let origin = job.origin.as_ref();
                    tracing::info!(
                        target: "moli_cdp_nav_timing",
                        transfer_id = %transfer_id,
                        label = %label,
                        origin_scheme = origin.map(|origin| origin.scheme.as_str()).unwrap_or(""),
                        origin_host = origin.map(|origin| origin.host.as_str()).unwrap_or(""),
                        origin_port = ?origin.and_then(|origin| origin.port),
                        origin = ?job.origin,
                        priority = job.priority,
                        queued_ms = queued_for.as_millis(),
                        active_before = state.active.len(),
                        active_same_origin_before = origin
                            .map(|origin| active_origin_count(&state.active, origin))
                            .unwrap_or(0),
                        pending_after = state.pending.len(),
                        pending_same_origin_after = origin
                            .map(|origin| pending_origin_count(&state.pending, origin))
                            .unwrap_or(0),
                        max_active = self.config.max_active.get(),
                        max_host_active = ?self.config.max_host_active.map(NonZeroUsize::get),
                        max_host_connections = ?self.config.max_host_connections.map(NonZeroUsize::get),
                        max_total_connections = ?self.config.max_total_connections.map(NonZeroUsize::get),
                        max_concurrent_streams = ?self.config.max_concurrent_streams.map(NonZeroUsize::get),
                        multiplex = self.config.multiplex,
                        stage = "curl_runtime_job_start",
                    );
                }
                let previous = state.active.insert(
                    transfer_id,
                    CurlActiveTransfer {
                        handle,
                        context: job.context,
                        origin: job.origin,
                        priority: job.priority,
                        label,
                        started_at: Instant::now(),
                        queued_for,
                    },
                );
                assert!(previous.is_none(), "curl transfer identity is unique");
            }
            Err(error) => self.send_completion(CurlMultiCompletion {
                transfer_id,
                easy: None,
                context: job.context,
                result: Err(error),
            }),
        }
    }

    fn process_completed_transfers(&self, state: &mut CurlOwnerState<H, C>, multi: &mut Multi) {
        if let Err(error) = multi.perform() {
            debug!("curl multi runtime perform failed: {error}");
        }
        let completed = completed_transfers(multi, &state.active);
        for (transfer_id, active, result) in
            take_transfers_in_notification_order(&mut state.active, completed)
        {
            self.finish_active_transfer(
                state,
                multi,
                transfer_id,
                active,
                result.map_err(Into::into),
            );
        }
    }

    fn expire_waiting_jobs(&self, state: &mut CurlOwnerState<H, C>) {
        let now = Instant::now();
        for pending in take_expired_pending_jobs(&mut state.pending, now) {
            self.complete_timed_out_job(pending, "while waiting in the scheduler");
        }
        for pending in state.dns.take_expired(now, |pending| pending.job.deadline) {
            self.complete_timed_out_job(pending, "while waiting for DNS");
        }
    }

    fn complete_timed_out_job(&self, pending: CurlPendingJob<H, C>, stage: &'static str) {
        let CurlPendingJob {
            transfer_id, job, ..
        } = pending;
        self.send_completion(CurlMultiCompletion {
            transfer_id,
            easy: Some(job.easy),
            context: job.context,
            result: Err(curl_runtime_timeout_error(stage)),
        });
    }

    fn finish_active_transfer(
        &self,
        state: &CurlOwnerState<H, C>,
        multi: &mut Multi,
        transfer_id: CurlTransferId,
        active: CurlActiveTransfer<H, C>,
        result: Result<()>,
    ) {
        let easy = match multi.remove2(active.handle) {
            Ok(easy) => Some(easy),
            Err(error) => {
                self.send_completion(CurlMultiCompletion {
                    transfer_id,
                    easy: None,
                    context: active.context,
                    result: Err(anyhow!(
                        "failed to remove curl easy handle for {}: {error}",
                        active.label
                    )),
                });
                return;
            }
        };
        if curl_runtime_trace_enabled() {
            let origin = active.origin.as_ref();
            match &result {
                Ok(()) => {
                    tracing::info!(
                        target: "moli_cdp_nav_timing",
                        transfer_id = %transfer_id,
                        label = %active.label,
                        origin_scheme = origin.map(|origin| origin.scheme.as_str()).unwrap_or(""),
                        origin_host = origin.map(|origin| origin.host.as_str()).unwrap_or(""),
                        origin_port = ?origin.and_then(|origin| origin.port),
                        origin = ?active.origin,
                        priority = active.priority,
                        ok = true,
                        active_ms = active.started_at.elapsed().as_millis(),
                        queued_ms = active.queued_for.as_millis(),
                        active_remaining = state.active.len(),
                        active_same_origin_remaining = origin
                            .map(|origin| active_origin_count(&state.active, origin))
                            .unwrap_or(0),
                        pending_after = state.pending.len(),
                        pending_same_origin_after = origin
                            .map(|origin| pending_origin_count(&state.pending, origin))
                            .unwrap_or(0),
                        stage = "curl_runtime_job_done",
                    );
                }
                Err(error) => {
                    tracing::info!(
                        target: "moli_cdp_nav_timing",
                        transfer_id = %transfer_id,
                        label = %active.label,
                        origin_scheme = origin.map(|origin| origin.scheme.as_str()).unwrap_or(""),
                        origin_host = origin.map(|origin| origin.host.as_str()).unwrap_or(""),
                        origin_port = ?origin.and_then(|origin| origin.port),
                        origin = ?active.origin,
                        priority = active.priority,
                        ok = false,
                        error = %error,
                        active_ms = active.started_at.elapsed().as_millis(),
                        queued_ms = active.queued_for.as_millis(),
                        active_remaining = state.active.len(),
                        active_same_origin_remaining = origin
                            .map(|origin| active_origin_count(&state.active, origin))
                            .unwrap_or(0),
                        pending_after = state.pending.len(),
                        pending_same_origin_after = origin
                            .map(|origin| pending_origin_count(&state.pending, origin))
                            .unwrap_or(0),
                        stage = "curl_runtime_job_done",
                    );
                }
            }
        }
        let result = result.with_context(|| {
            anyhow!(
                "curl request failed for {} after active={}ms queued={}ms",
                active.label,
                active.started_at.elapsed().as_millis(),
                active.queued_for.as_millis()
            )
        });
        self.send_completion(CurlMultiCompletion {
            transfer_id,
            easy,
            context: active.context,
            result,
        });
    }

    fn wait_for_curl_progress(&self, multi: &Multi, state: &CurlOwnerState<H, C>) {
        let mut wait_timeout = runtime_wait_timeout(multi, self.config.poll_interval)
            .unwrap_or(self.config.poll_interval);
        if let Some(deadline) = state.next_waiting_deadline() {
            wait_timeout = wait_timeout.min(deadline.saturating_duration_since(Instant::now()));
        }
        if wait_timeout.is_zero() {
            return;
        }
        if let Err(error) = multi.poll(&mut [], wait_timeout) {
            debug!("curl multi runtime poll failed: {error}");
        }
    }

    fn send_completion(&self, completion: CurlMultiCompletion<H, C>) {
        let _ = self.completion_tx.send(completion);
    }
}

fn curl_runtime_timeout_error(stage: &str) -> anyhow::Error {
    anyhow!("curl multi runtime request timed out {stage}")
}

fn curl_timeout_for_deadline(deadline: Instant, now: Instant) -> Option<Duration> {
    let remaining = deadline.saturating_duration_since(now);
    // curl-rust converts CURLOPT_TIMEOUT_MS with `Duration::as_millis()`. A
    // positive sub-millisecond value would therefore become zero, which
    // libcurl interprets as disabling the timeout entirely.
    (remaining >= Duration::from_millis(1)).then_some(remaining)
}

fn curl_runtime_trace_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        env_flag_enabled("MOLI_CDP_NAV_TIMING") || env_flag_enabled("MOLI_CURL_RUNTIME_TRACE")
    })
}

fn env_flag_enabled(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| {
        let value = value.trim();
        !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_millisecond_deadline_never_disables_the_libcurl_timeout() {
        let now = Instant::now();
        assert_eq!(
            curl_timeout_for_deadline(now + Duration::from_micros(999), now),
            None
        );
        assert_eq!(
            curl_timeout_for_deadline(now + Duration::from_millis(1), now),
            Some(Duration::from_millis(1))
        );
    }
}
