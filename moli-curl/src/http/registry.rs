//! HTTP jobs move through the priority queue, DNS and active easy handles.
//! The runtime alone calls perform/messages/poll and dispatches completions here.

use super::{
    CurlMultiCompletion, CurlMultiJob,
    scheduling::{
        CurlActiveTransfer, CurlPendingJob, active_origin_count, enqueue_existing_pending_job,
        enqueue_pending_job, job_is_eligible, pending_origin_count, take_expired_pending_jobs,
        take_transfers_in_notification_order,
    },
};
use crate::{
    CurlMultiRuntimeConfig, CurlTransferId,
    dns_adapter::{CurlDnsOwnerCompletion, CurlDnsOwnerResidence, CurlDnsReady},
};
use anyhow::{Context, Result, anyhow};
use crossbeam_channel::{Receiver, Sender};
use curl::{
    easy::Handler,
    multi::{Message, Multi},
};
use std::{
    collections::{HashMap, VecDeque},
    num::NonZeroUsize,
    sync::OnceLock,
    time::{Duration, Instant},
};

pub(crate) struct HttpRegistry<H: Handler, C> {
    config: CurlMultiRuntimeConfig,
    completion_tx: Sender<CurlMultiCompletion<H, C>>,
    closed: bool,
    pending: VecDeque<CurlPendingJob<H, C>>,
    dns: CurlDnsOwnerResidence<CurlTransferId, CurlPendingJob<H, C>>,
    active: HashMap<CurlTransferId, CurlActiveTransfer<H, C>>,
}

impl<H: Handler, C> HttpRegistry<H, C> {
    pub(crate) fn new(
        config: CurlMultiRuntimeConfig,
        completion_tx: Sender<CurlMultiCompletion<H, C>>,
    ) -> Self {
        Self {
            config,
            completion_tx,
            closed: false,
            pending: VecDeque::new(),
            dns: CurlDnsOwnerResidence::default(),
            active: HashMap::new(),
        }
    }

    pub(crate) fn has_active(&self) -> bool {
        !self.active.is_empty()
    }

    /// DNS-only work can wait on its completion channel without polling curl.
    pub(crate) fn has_curl_work(&self) -> bool {
        self.has_active() || !self.pending.is_empty()
    }

    /// Pending work can run after completions release scheduler capacity.
    /// Use the same global and per-origin limits as the actual startup path.
    pub(crate) fn has_eligible_jobs(&self) -> bool {
        self.next_eligible_job_index().is_some()
    }

    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.pending
            .iter()
            .filter_map(|pending| pending.job.deadline)
            .chain(self.dns.next_deadline(|pending| pending.job.deadline))
            .min()
    }

    pub(crate) fn dns_completions(&self) -> &Receiver<CurlDnsOwnerCompletion<CurlTransferId>> {
        self.dns.completion_receiver()
    }

    pub(crate) fn advance(&mut self, multi: &mut Multi) {
        self.drain_dns_completions();
        self.expire_waiting_jobs();
        self.start_eligible_jobs(multi);
    }

    pub(crate) fn contains(&self, id: CurlTransferId) -> bool {
        self.active.contains_key(&id)
    }

    pub(crate) fn completion_result(
        &self,
        id: CurlTransferId,
        message: &Message<'_>,
    ) -> Option<Result<(), curl::Error>> {
        message.result_for2(&self.active.get(&id)?.handle)
    }

    pub(crate) fn complete(
        &mut self,
        multi: &mut Multi,
        completed: Vec<(CurlTransferId, Result<(), curl::Error>)>,
    ) {
        for (transfer_id, active, result) in
            take_transfers_in_notification_order(&mut self.active, completed)
        {
            self.finish_active_transfer(multi, transfer_id, active, result.map_err(Into::into));
        }
    }

    pub(crate) fn admit(&mut self, transfer_id: CurlTransferId, job: CurlMultiJob<H, C>) {
        if self.closed {
            self.send_completion(CurlMultiCompletion {
                transfer_id,
                easy: Some(job.easy),
                context: job.context,
                result: Err(anyhow!("curl multi runtime is shutting down")),
            });
            return;
        }
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
                pending_before = self.pending.len(),
                pending_same_origin_before = origin
                    .map(|origin| pending_origin_count(&self.pending, origin))
                    .unwrap_or(0),
                stage = "curl_runtime_job_queued",
            );
        }
        enqueue_pending_job(&mut self.pending, transfer_id, job);
    }

    pub(crate) fn shutdown(&mut self, multi: &mut Multi) {
        if self.closed {
            return;
        }
        self.closed = true;
        while let Some(pending) = self.pending.pop_front() {
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
        for pending in self.dns.drain() {
            let CurlPendingJob {
                transfer_id, job, ..
            } = pending;
            let _ = self.completion_tx.send(CurlMultiCompletion {
                transfer_id,
                easy: Some(job.easy),
                context: job.context,
                result: Err(anyhow!(
                    "curl multi runtime DNS request cancelled during shutdown"
                )),
            });
        }
        for (transfer_id, active) in self.active.drain() {
            let easy = multi.remove2(active.handle).ok();
            let _ = self.completion_tx.send(CurlMultiCompletion {
                transfer_id,
                easy,
                context: active.context,
                result: Err(anyhow!(
                    "curl multi runtime request cancelled during shutdown"
                )),
            });
        }
    }

    fn next_eligible_job_index(&self) -> Option<usize> {
        if self.closed || self.active.len() >= self.config.max_active.get() {
            return None;
        }
        self.pending.iter().position(|pending| {
            job_is_eligible(
                pending.job.origin.as_ref(),
                &self.active,
                self.config.max_host_active,
            )
        })
    }

    fn start_eligible_jobs(&mut self, multi: &mut Multi) {
        while let Some(index) = self.next_eligible_job_index() {
            let pending = self
                .pending
                .remove(index)
                .expect("pending curl job index should exist");
            let dns_target = pending.job.dns_resolution.target().cloned();
            match dns_target {
                Some(target) => self
                    .dns
                    .start(pending.transfer_id, pending, target, multi.waker()),
                None => self.start_job(multi, pending),
            }
        }
    }

    fn drain_dns_completions(&mut self) {
        while let Some(ready) = self.dns.try_claim_next() {
            self.handle_dns_completion(ready);
        }
    }

    pub(crate) fn claim_dns_completion(
        &mut self,
        completion: CurlDnsOwnerCompletion<CurlTransferId>,
    ) {
        let Some(ready) = self.dns.claim(completion) else {
            return;
        };
        self.handle_dns_completion(ready);
    }

    fn handle_dns_completion(&mut self, ready: CurlDnsReady<CurlPendingJob<H, C>>) {
        let mut pending = ready.pending;
        if self.closed {
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
                enqueue_existing_pending_job(&mut self.pending, pending);
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

    fn start_job(&mut self, multi: &mut Multi, pending: CurlPendingJob<H, C>) {
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
                        active_before = self.active.len(),
                        active_same_origin_before = origin
                            .map(|origin| active_origin_count(&self.active, origin))
                            .unwrap_or(0),
                        pending_after = self.pending.len(),
                        pending_same_origin_after = origin
                            .map(|origin| pending_origin_count(&self.pending, origin))
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
                let previous = self.active.insert(
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

    fn expire_waiting_jobs(&mut self) {
        let now = Instant::now();
        for pending in take_expired_pending_jobs(&mut self.pending, now) {
            self.complete_timed_out_job(pending, "while waiting in the scheduler");
        }
        for pending in self.dns.take_expired(now, |pending| pending.job.deadline) {
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
                        active_remaining = self.active.len(),
                        active_same_origin_remaining = origin
                            .map(|origin| active_origin_count(&self.active, origin))
                            .unwrap_or(0),
                        pending_after = self.pending.len(),
                        pending_same_origin_after = origin
                            .map(|origin| pending_origin_count(&self.pending, origin))
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
                        active_remaining = self.active.len(),
                        active_same_origin_remaining = origin
                            .map(|origin| active_origin_count(&self.active, origin))
                            .unwrap_or(0),
                        pending_after = self.pending.len(),
                        pending_same_origin_after = origin
                            .map(|origin| pending_origin_count(&self.pending, origin))
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
    use crate::{CurlDnsResolution, CurlOriginKey};
    use curl::easy::Easy2;
    #[derive(Debug)]
    struct TestHandler;

    impl Handler for TestHandler {}

    fn test_job(
        label: &str,
        priority: u8,
        origin: Option<CurlOriginKey>,
    ) -> CurlMultiJob<TestHandler, String> {
        CurlMultiJob {
            easy: Easy2::new(TestHandler),
            context: label.to_owned(),
            origin,
            deadline: None,
            dns_resolution: CurlDnsResolution::curl_managed(),
            priority,
            label: label.to_owned(),
        }
    }

    fn test_transfer_id(sequence: usize) -> CurlTransferId {
        CurlTransferId::from_token(sequence).expect("test transfer ID is non-zero")
    }

    #[test]
    fn eligible_jobs_respect_global_and_origin_capacity_after_completions() {
        let config = CurlMultiRuntimeConfig {
            max_active: NonZeroUsize::new(2).unwrap(),
            max_host_active: NonZeroUsize::new(1),
            ..CurlMultiRuntimeConfig::default()
        };
        let (completion_tx, _completed) = crossbeam_channel::unbounded();
        let mut multi = Multi::new();
        let mut registry = HttpRegistry::new(config, completion_tx);
        let origin = |host: &str| CurlOriginKey {
            scheme: "https".to_owned(),
            host: host.to_owned(),
            port: Some(443),
        };
        let first = test_transfer_id(1);
        let blocked = test_transfer_id(2);
        let other_origin = test_transfer_id(3);
        let no_origin = test_transfer_id(4);

        registry.admit(first, test_job("first", 1, Some(origin("a.test"))));
        registry.advance(&mut multi);
        assert!(registry.contains(first));
        assert!(!registry.has_eligible_jobs());

        // A free global slot does not bypass the origin cap.
        registry.admit(blocked, test_job("blocked", 2, Some(origin("a.test"))));
        assert!(!registry.has_eligible_jobs());
        // A blocked high-priority head must not hide eligible work elsewhere.
        registry.admit(other_origin, test_job("other", 0, Some(origin("b.test"))));
        assert!(registry.has_eligible_jobs());
        registry.advance(&mut multi);
        assert!(registry.contains(other_origin));
        assert!(!registry.contains(blocked));

        // The global cap also applies to jobs without an origin key.
        registry.admit(no_origin, test_job("no-origin", 0, None));
        assert!(!registry.has_eligible_jobs());
        registry.complete(&mut multi, vec![(other_origin, Ok(()))]);
        assert!(registry.has_eligible_jobs());
        registry.advance(&mut multi);
        assert!(registry.contains(no_origin));
        assert!(!registry.contains(blocked));

        registry.complete(&mut multi, vec![(first, Ok(()))]);
        assert!(registry.has_eligible_jobs());
        registry.advance(&mut multi);
        assert!(registry.contains(blocked));
        assert!(!registry.has_eligible_jobs());
        registry.shutdown(&mut multi);
        assert!(!registry.has_eligible_jobs());
    }

    #[test]
    fn active_transfer_wait_is_capped_by_the_earliest_queued_deadline() {
        let now = Instant::now();
        let later = now + Duration::from_secs(2);
        let earlier = now + Duration::from_secs(1);
        let mut later_job = test_job("later", 1, None);
        later_job.deadline = Some(later);
        let mut earlier_job = test_job("earlier", 1, None);
        earlier_job.deadline = Some(earlier);
        let mut pending = VecDeque::new();
        enqueue_pending_job(&mut pending, test_transfer_id(1), later_job);
        enqueue_pending_job(&mut pending, test_transfer_id(2), earlier_job);
        let (completion_tx, _) = crossbeam_channel::unbounded();
        let mut registry = HttpRegistry::new(CurlMultiRuntimeConfig::default(), completion_tx);
        registry.pending = pending;

        assert_eq!(registry.next_deadline(), Some(earlier));
    }

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
