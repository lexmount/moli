use std::{
    num::NonZeroUsize,
    sync::{Arc, OnceLock},
    thread,
    time::{Duration, Instant},
};

use parking_lot::Mutex;

use crate::{
    DnsCachePartition, DnsLookupResult, DnsTarget,
    identity::DnsLookupKey,
    lookup::{DnsLookup, system_dns_lookup},
    state::{DnsLookupAdmission, DnsResolverState},
    worker_pool::{DnsWorkerPool, publish_lookup_result},
};

const MAX_SHARED_DNS_WORKERS: usize = 4;
const DEFAULT_POSITIVE_CACHE_TTL: Duration = Duration::from_secs(60);

/// Process-level residence for bounded blocking DNS resolution.
///
/// Identical in-flight lookups within one cache partition share one worker
/// command. Positive results are cached for a short fixed lifetime. Completion
/// may run immediately on the caller for a cache hit or on a resolver worker
/// after a system lookup; it must therefore remain cheap and thread-safe.
pub struct DnsResolverService {
    state: Arc<Mutex<DnsResolverState>>,
    worker_pool: DnsWorkerPool,
    positive_cache_ttl: Duration,
}

impl DnsResolverService {
    pub fn shared() -> Result<&'static Self, Arc<str>> {
        static SERVICE: OnceLock<Result<DnsResolverService, Arc<str>>> = OnceLock::new();
        match SERVICE.get_or_init(|| {
            Self::new(
                shared_dns_worker_count(),
                DEFAULT_POSITIVE_CACHE_TTL,
                Arc::new(system_dns_lookup),
            )
            .map_err(|error| Arc::from(format!("failed to start shared DNS resolver: {error}")))
        }) {
            Ok(service) => Ok(service),
            Err(error) => Err(Arc::clone(error)),
        }
    }

    /// Registers one terminal callback for an exact partition and target.
    ///
    /// The callback is consumed exactly once. It never executes while the
    /// resolver state lock is held.
    pub fn resolve(
        &self,
        partition: DnsCachePartition,
        target: DnsTarget,
        completion: impl FnOnce(DnsLookupResult) + Send + 'static,
    ) {
        if target_is_reserved_invalid(&target) {
            completion(Err(Arc::from(format!(
                "failed to resolve {}:{}: reserved .invalid domain",
                target.host(),
                target.port()
            ))));
            return;
        }
        let key = DnsLookupKey { partition, target };
        let admission = {
            let mut state = self.state.lock();
            state.admit(key, Box::new(completion), Instant::now())
        };
        match admission {
            DnsLookupAdmission::CompleteCached {
                addresses,
                completion,
            } => completion(Ok(addresses)),
            DnsLookupAdmission::Coalesced => {}
            DnsLookupAdmission::Start(key) => {
                if let Err(error) = self.worker_pool.resolve(key) {
                    publish_lookup_result(
                        &self.state,
                        error.0,
                        Err(Arc::from("shared DNS resolver is shutting down")),
                        self.positive_cache_ttl,
                    );
                }
            }
        }
    }

    pub(crate) fn new(
        worker_count: NonZeroUsize,
        positive_cache_ttl: Duration,
        lookup: Arc<DnsLookup>,
    ) -> std::io::Result<Self> {
        let state = Arc::new(Mutex::new(DnsResolverState::default()));
        let worker_pool =
            DnsWorkerPool::start(worker_count, Arc::clone(&state), lookup, positive_cache_ttl)?;
        Ok(Self {
            state,
            worker_pool,
            positive_cache_ttl,
        })
    }
}

/// `.invalid` is a special-use top-level domain whose names are guaranteed not
/// to exist. Resolve it locally so a deterministic negative answer cannot be
/// delayed behind blocking host-resolver work or depend on the machine's DNS
/// configuration.
fn target_is_reserved_invalid(target: &DnsTarget) -> bool {
    let host = target.host().trim_end_matches('.');
    host.eq_ignore_ascii_case("invalid")
        || host
            .get(host.len().saturating_sub(".invalid".len())..)
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".invalid"))
}

fn shared_dns_worker_count() -> NonZeroUsize {
    let available = thread::available_parallelism()
        .map(NonZeroUsize::get)
        .unwrap_or(1);
    NonZeroUsize::new(available.min(MAX_SHARED_DNS_WORKERS))
        .expect("shared DNS worker count is always non-zero")
}
