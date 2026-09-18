use std::{collections::HashMap, hash::Hash, net::IpAddr, time::Instant};

use anyhow::{Context, Result, anyhow};
use crossbeam_channel::{Receiver, Sender};
use curl::{
    easy::{Easy2, Handler, List},
    multi::MultiWaker,
};
use moli_dns_resolver::{DnsCachePartition, DnsLookupResult, DnsResolverService, DnsTarget};

use crate::NetworkAddressPolicy;

/// Curl-side policy for DNS ownership before a transfer enters the multi set.
///
/// `endpoint == None` means no shared endpoint lookup is required. The configured
/// curl handle may already have a fixed address, use an IP literal or proxy, or
/// deliberately retain libcurl's resolver behavior. `Some` means the transfer
/// must first wait in [`CurlDnsOwnerResidence`]. After that residence installs
/// the exact address list with `CURLOPT_RESOLVE`, the shared lookup is consumed
/// so a requeued transfer cannot resolve twice.
#[derive(Debug)]
pub struct CurlDnsResolution {
    endpoint: Option<Box<CurlDnsEndpointResolution>>,
}

#[derive(Debug)]
struct CurlDnsEndpointResolution {
    target: DnsTarget,
    /// Existing caller-provided `CURLOPT_RESOLVE` entries that must remain
    /// installed when the generated endpoint answer is added.
    static_entries: Vec<String>,
    address_policy: NetworkAddressPolicy,
    policy_target: String,
}

impl CurlDnsResolution {
    /// Creates a policy that does not request Moli's shared endpoint resolver.
    ///
    /// This keeps `moli-curl` transport-neutral: callers may use an IP literal,
    /// preconfigure `CURLOPT_RESOLVE`, delegate the target to a proxy, or retain
    /// libcurl's resolver behavior.
    pub fn no_shared_resolution() -> Self {
        Self { endpoint: None }
    }

    #[deprecated(
        note = "use `no_shared_resolution`; no shared lookup does not necessarily mean curl performs DNS"
    )]
    pub fn curl_managed() -> Self {
        Self::no_shared_resolution()
    }

    pub fn resolve_endpoint(target: DnsTarget, static_entries: Vec<String>) -> Self {
        let policy_target = format!("{}:{}", target.host(), target.port());
        Self {
            endpoint: Some(Box::new(CurlDnsEndpointResolution {
                target,
                static_entries,
                address_policy: NetworkAddressPolicy::default(),
                policy_target,
            })),
        }
    }

    #[deprecated(note = "use `resolve_endpoint`; shared DNS may resolve a proxy endpoint")]
    pub fn resolve_origin(target: DnsTarget, static_entries: Vec<String>) -> Self {
        Self::resolve_endpoint(target, static_entries)
    }

    /// Applies address admission to the shared-resolver result before it is
    /// installed on curl. The target text is retained for actionable errors.
    pub fn with_network_address_policy(
        mut self,
        address_policy: NetworkAddressPolicy,
        policy_target: impl Into<String>,
    ) -> Self {
        self.set_network_address_policy(address_policy, policy_target);
        self
    }

    pub(crate) fn set_network_address_policy(
        &mut self,
        address_policy: NetworkAddressPolicy,
        policy_target: impl Into<String>,
    ) {
        if let Some(resolution) = self.endpoint.as_mut() {
            resolution.address_policy = address_policy;
            resolution.policy_target = policy_target.into();
        }
    }

    pub(crate) fn target(&self) -> Option<&DnsTarget> {
        self.endpoint.as_ref().map(|resolution| &resolution.target)
    }

    /// Installs a successful shared-resolver answer and consumes that policy.
    pub(crate) fn install<H: Handler>(
        &mut self,
        easy: &mut Easy2<H>,
        addresses: &[IpAddr],
    ) -> Result<()> {
        let Some(resolution) = self.endpoint.as_ref() else {
            return Ok(());
        };
        resolution
            .address_policy
            .check_addresses(addresses, &resolution.policy_target)?;
        // Preserve the complete checked answer. Filtering forbidden addresses
        // would change curl's selection/fallback semantics, while resolving a
        // second time would reopen the DNS-rebinding TOCTOU window.
        let mut resolve = List::new();
        for entry in &resolution.static_entries {
            resolve
                .append(entry)
                .with_context(|| anyhow!("failed to preserve curl host resolve entry `{entry}`"))?;
        }
        let addresses = addresses
            .iter()
            .map(|address| match address {
                IpAddr::V4(address) => address.to_string(),
                IpAddr::V6(address) => format!("[{address}]"),
            })
            .collect::<Vec<_>>()
            .join(",");
        let generated_entry = format!(
            "{}:{}:{addresses}",
            resolution.target.host(),
            resolution.target.port()
        );
        resolve.append(&generated_entry).with_context(|| {
            anyhow!("failed to build curl DNS resolve entry `{generated_entry}`")
        })?;
        easy.resolve(resolve)
            .context("failed to install shared DNS result on curl request")?;
        // CURLOPT_RESOLVE entries without `+` are permanent for this easy
        // handle. Consuming the endpoint also prevents a requeued job from
        // issuing a second lookup before it enters curl.
        self.endpoint = None;
        Ok(())
    }
}

pub(crate) struct CurlDnsOwnerCompletion<I> {
    request_id: I,
    result: DnsLookupResult,
}

pub(crate) struct CurlDnsReady<P> {
    pub(crate) pending: P,
    pub(crate) result: DnsLookupResult,
}

/// Curl-owner residence for transfers parked on shared DNS resolution.
///
/// The generic pending transfer stays here while `moli-dns-resolver`
/// owns only the lookup. Completion returns through a channel and is claimed by
/// exact request identity on the curl owner; a late completion after shutdown
/// cannot recover or mutate a retired transfer.
pub(crate) struct CurlDnsOwnerResidence<I, P> {
    partition: DnsCachePartition,
    completion_tx: Sender<CurlDnsOwnerCompletion<I>>,
    completion_rx: Receiver<CurlDnsOwnerCompletion<I>>,
    waiting: HashMap<I, P>,
}

impl<I, P> Default for CurlDnsOwnerResidence<I, P> {
    fn default() -> Self {
        let (completion_tx, completion_rx) = crossbeam_channel::unbounded();
        Self {
            partition: DnsCachePartition::fresh(),
            completion_tx,
            completion_rx,
            waiting: HashMap::new(),
        }
    }
}

impl<I, P> CurlDnsOwnerResidence<I, P>
where
    I: Copy + Eq + Hash + Send + 'static,
{
    pub(crate) fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }

    pub(crate) fn completion_receiver(&self) -> &Receiver<CurlDnsOwnerCompletion<I>> {
        &self.completion_rx
    }

    pub(crate) fn start(
        &mut self,
        request_id: I,
        pending: P,
        target: DnsTarget,
        owner_waker: MultiWaker,
    ) {
        let previous = self.waiting.insert(request_id, pending);
        assert!(
            previous.is_none(),
            "curl owner DNS request identity is unique"
        );

        let completion_tx = self.completion_tx.clone();
        match DnsResolverService::shared() {
            Ok(service) => service.resolve(self.partition, target, move |result| {
                let _ = completion_tx.send(CurlDnsOwnerCompletion { request_id, result });
                let _ = owner_waker.wakeup();
            }),
            Err(error) => {
                let _ = self.completion_tx.send(CurlDnsOwnerCompletion {
                    request_id,
                    result: Err(error),
                });
                let _ = owner_waker.wakeup();
            }
        }
    }

    pub(crate) fn claim(
        &mut self,
        completion: CurlDnsOwnerCompletion<I>,
    ) -> Option<CurlDnsReady<P>> {
        let pending = self.waiting.remove(&completion.request_id)?;
        Some(CurlDnsReady {
            pending,
            result: completion.result,
        })
    }

    pub(crate) fn try_claim_next(&mut self) -> Option<CurlDnsReady<P>> {
        loop {
            let completion = self.completion_rx.try_recv().ok()?;
            if let Some(ready) = self.claim(completion) {
                return Some(ready);
            }
        }
    }

    pub(crate) fn next_deadline(
        &self,
        mut deadline_for: impl FnMut(&P) -> Option<Instant>,
    ) -> Option<Instant> {
        self.waiting.values().filter_map(&mut deadline_for).min()
    }

    /// Retires expired requests from the owner residence.
    ///
    /// The resolver may still publish a late completion because a blocking
    /// system lookup cannot be cancelled safely. Exact request identity makes
    /// that completion harmless: [`Self::claim`] ignores it after this method
    /// removes the waiting entry.
    pub(crate) fn take_expired(
        &mut self,
        now: Instant,
        mut deadline_for: impl FnMut(&P) -> Option<Instant>,
    ) -> Vec<P> {
        let expired_ids = self
            .waiting
            .iter()
            .filter_map(|(request_id, pending)| {
                deadline_for(pending)
                    .is_some_and(|deadline| deadline <= now)
                    .then_some(*request_id)
            })
            .collect::<Vec<_>>();
        expired_ids
            .into_iter()
            .filter_map(|request_id| self.waiting.remove(&request_id))
            .collect()
    }

    pub(crate) fn drain(&mut self) -> impl Iterator<Item = P> + '_ {
        self.waiting.drain().map(|(_, pending)| pending)
    }

    /// Removes retired work without letting a late resolver result recover it.
    pub(crate) fn take_matching(&mut self, mut predicate: impl FnMut(&P) -> bool) -> Vec<P> {
        let ids: Vec<_> = self
            .waiting
            .iter()
            .filter_map(|(id, pending)| predicate(pending).then_some(*id))
            .collect();
        ids.into_iter()
            .filter_map(|id| self.waiting.remove(&id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::{net::IpAddr, sync::Arc};

    use curl::easy::Handler;

    use super::*;

    #[derive(Debug)]
    struct TestHandler;

    impl Handler for TestHandler {}

    fn test_result() -> DnsLookupResult {
        Ok(Arc::from([IpAddr::from([127, 0, 0, 1])]))
    }

    #[test]
    fn installed_endpoint_consumes_shared_resolution() {
        let target = DnsTarget::new("example.test", 443);
        let mut policy = CurlDnsResolution::resolve_endpoint(target.clone(), Vec::new());
        let mut easy = Easy2::new(TestHandler);

        assert_eq!(policy.target(), Some(&target));
        policy
            .install(&mut easy, &[IpAddr::from([127, 0, 0, 1])])
            .expect("resolved endpoint should install on curl");
        assert_eq!(policy.target(), None);
    }

    #[test]
    fn blocked_address_prevents_installing_the_complete_dns_answer() {
        let target = DnsTarget::new("example.test", 443);
        let mut resolution = CurlDnsResolution::resolve_endpoint(target.clone(), Vec::new())
            .with_network_address_policy(
                NetworkAddressPolicy::new(true, Vec::new()),
                "https://example.test/",
            );
        let mut easy = Easy2::new(TestHandler);

        let error = resolution
            .install(
                &mut easy,
                &[
                    IpAddr::from([93, 184, 216, 34]),
                    IpAddr::from([127, 0, 0, 1]),
                ],
            )
            .expect_err("one private answer must reject the complete DNS result");

        assert!(error.to_string().contains("127.0.0.1"));
        assert_eq!(
            resolution.target(),
            Some(&target),
            "a rejected answer must never transition to an installed state"
        );
    }

    #[test]
    fn exact_completion_can_be_claimed_only_once() {
        let mut residence = CurlDnsOwnerResidence::<u64, _>::default();
        let request_id = 7;
        residence.waiting.insert(request_id, "pending");

        let ready = residence
            .claim(CurlDnsOwnerCompletion {
                request_id,
                result: test_result(),
            })
            .expect("exact pending request should be claimed");
        assert_eq!(ready.pending, "pending");
        assert!(
            residence
                .claim(CurlDnsOwnerCompletion {
                    request_id,
                    result: test_result(),
                })
                .is_none(),
            "late duplicate completion must not recover retired work"
        );
    }

    #[test]
    fn expired_dns_residence_rejects_a_late_completion() {
        let mut residence = CurlDnsOwnerResidence::<u64, _>::default();
        let request_id = 11;
        let now = Instant::now();
        residence.waiting.insert(request_id, ("pending", Some(now)));

        assert_eq!(residence.next_deadline(|pending| pending.1), Some(now));
        assert_eq!(
            residence.take_expired(now, |pending| pending.1),
            [("pending", Some(now))]
        );
        assert!(
            residence
                .claim(CurlDnsOwnerCompletion {
                    request_id,
                    result: test_result(),
                })
                .is_none(),
            "a resolver completion must not recover a timed-out transfer"
        );
    }

    #[test]
    fn queued_stale_completion_does_not_hide_next_ready_request() {
        let mut residence = CurlDnsOwnerResidence::<u64, _>::default();
        let stale_id = 3;
        let ready_id = 4;
        residence.waiting.insert(ready_id, "ready");
        residence
            .completion_tx
            .send(CurlDnsOwnerCompletion {
                request_id: stale_id,
                result: test_result(),
            })
            .expect("stale completion should enter the test queue");
        residence
            .completion_tx
            .send(CurlDnsOwnerCompletion {
                request_id: ready_id,
                result: test_result(),
            })
            .expect("ready completion should enter the test queue");

        let ready = residence
            .try_claim_next()
            .expect("drain should skip stale completion and claim ready work");
        assert_eq!(ready.pending, "ready");
    }
}
