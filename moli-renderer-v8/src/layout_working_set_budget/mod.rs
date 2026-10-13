//! Shared accounting for optional layout computation storage.
//!
//! This module owns identities, charges and admission order only. Payloads stay
//! on their renderer lane. A lease must be dropped *after* its payload; closing
//! a registration revokes admission without pretending that storage was freed.

use std::{
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::Mutex;

use crate::{
    document_runtime::DomHandle, native_bridge::WindowDocumentTaskTarget,
    runtime::RendererPageToken,
};

mod ledger;
#[cfg(test)]
mod tests;

use ledger::{Ledger, PageKey};

pub(crate) const DEFAULT_PAGE_WORKING_SET_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BudgetLimits {
    pub browser_bytes: usize,
    pub page_bytes: usize,
    pub document_bytes: usize,
}

impl Default for BudgetLimits {
    fn default() -> Self {
        // Experimental admission charges, not allocator or process-memory caps.
        // Keep the existing document/Page bounds and allow four full Pages at
        // the Browser root; these values remain subject to release measurement.
        Self {
            browser_bytes: 4 * DEFAULT_PAGE_WORKING_SET_BYTES,
            page_bytes: DEFAULT_PAGE_WORKING_SET_BYTES,
            document_bytes: moli_layout::DEFAULT_DOCUMENT_WORKING_SET_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BudgetTiming {
    pub protection: Duration,
    pub idle_after: Duration,
    pub demand_ttl: Duration,
    pub claim_ttl: Duration,
}

impl Default for BudgetTiming {
    fn default() -> Self {
        Self {
            protection: Duration::from_secs(5),
            idle_after: Duration::from_secs(5),
            demand_ttl: Duration::from_secs(30),
            claim_ttl: Duration::from_secs(5),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BudgetConfigError(&'static str);

impl fmt::Display for BudgetConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for BudgetConfigError {}

pub(crate) trait BudgetClock: fmt::Debug + Send + Sync {
    fn now(&self) -> Instant;
}

#[derive(Debug)]
struct MonotonicClock;

impl BudgetClock for MonotonicClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

struct BudgetInner {
    state: Mutex<Ledger>,
    clock: Arc<dyn BudgetClock>,
}

/// Browser-root handle. Clones refer to one accounting pool.
#[derive(Clone)]
pub struct LayoutWorkingSetBudget(Arc<BudgetInner>);

impl Default for LayoutWorkingSetBudget {
    fn default() -> Self {
        Self::new(BudgetLimits::default(), BudgetTiming::default())
            .expect("default layout working-set budget must be valid")
    }
}

impl fmt::Debug for LayoutWorkingSetBudget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("LayoutWorkingSetBudget")
            .field(&self.snapshot())
            .finish()
    }
}

impl LayoutWorkingSetBudget {
    pub fn new(limits: BudgetLimits, timing: BudgetTiming) -> Result<Self, BudgetConfigError> {
        Self::with_clock(limits, timing, Arc::new(MonotonicClock))
    }

    pub(crate) fn with_clock(
        limits: BudgetLimits,
        timing: BudgetTiming,
        clock: Arc<dyn BudgetClock>,
    ) -> Result<Self, BudgetConfigError> {
        let now = clock.now();
        if limits.browser_bytes == 0 || limits.page_bytes == 0 || limits.document_bytes == 0 {
            return Err(BudgetConfigError("absolute limits must be positive"));
        }
        if timing.demand_ttl.is_zero() || timing.claim_ttl.is_zero() {
            return Err(BudgetConfigError(
                "demand and claim lifetimes must be positive",
            ));
        }
        if [
            timing.protection,
            timing.idle_after,
            timing.demand_ttl,
            timing.claim_ttl,
        ]
        .into_iter()
        .any(|duration| now.checked_add(duration).is_none())
        {
            return Err(BudgetConfigError("budget deadline is not representable"));
        }
        Ok(Self(Arc::new(BudgetInner {
            state: Mutex::new(Ledger::new(limits, timing, now)),
            clock,
        })))
    }

    fn with_state<T>(&self, action: impl FnOnce(&mut Ledger, Instant) -> T) -> T {
        // Clock implementations are not called under the ledger lock. Threads
        // can acquire that lock out of sampling order, so Ledger clamps time.
        let sampled = self.0.clock.now();
        let mut state = self.0.state.lock();
        let now = state.observe_time(sampled);
        action(&mut state, now)
    }

    pub fn set_pressure_target(&self, bytes: usize) -> Result<PressureChange, BudgetConfigError> {
        self.with_state(|state, now| state.set_pressure_target(bytes, now))
    }

    /// Reads owned accounting metadata without advancing time or evicting.
    pub fn snapshot(&self) -> BudgetSnapshot {
        self.0.state.lock().snapshot()
    }

    /// Reads the immutable admission bounds without advancing the ledger.
    pub fn limits(&self) -> BudgetLimits {
        self.0.state.lock().limits()
    }

    pub fn shares_state_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub(crate) fn register_page(
        &self,
        page: RendererPageToken,
    ) -> Result<PageRegistration, RegistrationError> {
        let key = self.with_state(|state, now| state.register_page(page, now))?;
        Ok(PageRegistration {
            handle: PageBudgetHandle {
                pool: self.clone(),
                key,
            },
            closed: false,
        })
    }

    #[cfg(test)]
    fn set_next_id_for_test(&self, next: u64) {
        self.0.state.lock().set_next_id_for_test(next);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PressureChange {
    pub changed: bool,
    pub epoch: u64,
    pub target_bytes: usize,
}

#[derive(Clone, Debug)]
pub struct BudgetSnapshot {
    pub charged_bytes: usize,
    pub pressure_target: usize,
    pub pressure_epoch: u64,
    pub open_pages: usize,
    pub live_leases: usize,
    pub queued_pages: usize,
    pub ready_bytes: usize,
    /// Cumulative requests, not the number of currently pending trims.
    pub trim_requests: usize,
    pub(crate) pages: Vec<PageBudgetSnapshot>,
    pub(crate) fifo: Vec<RendererPageToken>,
    pub(crate) handoff: Option<HandoffSnapshot>,
    pub(crate) last_observed_now: Instant,
}

#[derive(Clone, Debug)]
pub(crate) struct PageBudgetSnapshot {
    pub(crate) token: RendererPageToken,
    pub(crate) registration: u64,
    pub(crate) open: bool,
    pub(crate) charged_bytes: usize,
    pub(crate) documents: usize,
    pub(crate) demand: Option<DemandSnapshot>,
    pub(crate) trim: Option<TrimRequest>,
    pub(crate) resident_since: Option<Instant>,
    pub(crate) protected_until: Option<Instant>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DemandSnapshot {
    pub(crate) slot: DocumentSlot,
    pub(crate) bytes: usize,
    pub(crate) enqueued_at: Instant,
    pub(crate) last_demand_at: Instant,
    pub(crate) claim_deadline: Option<Instant>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HandoffSnapshot {
    WaitingRelease {
        head: RendererPageToken,
    },
    ReadyToClaim {
        head: RendererPageToken,
        reserved_bytes: usize,
        deadline: Instant,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RegistrationError {
    Closed,
    DuplicatePage,
    DuplicateSlot,
    Exhausted,
}

/// The stable Page slot is the sole owner. Navigation only clones its handle.
#[derive(Debug)]
pub(crate) struct PageRegistration {
    handle: PageBudgetHandle,
    closed: bool,
}

impl PageRegistration {
    pub(crate) fn handle(&self) -> PageBudgetHandle {
        self.handle.clone()
    }

    pub(crate) fn close(&mut self) {
        if !self.closed {
            self.closed = true;
            self.handle
                .pool
                .with_state(|state, now| state.close_page(self.handle.key, now));
        }
    }
}

impl Drop for PageRegistration {
    fn drop(&mut self) {
        self.close();
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PageBudgetHandle {
    pool: LayoutWorkingSetBudget,
    key: PageKey,
}

impl PageBudgetHandle {
    pub(crate) fn limits(&self) -> BudgetLimits {
        self.pool.limits()
    }

    pub(crate) fn shares_registration_with(&self, other: &Self) -> bool {
        self.key == other.key && self.pool.shares_state_with(&other.pool)
    }

    pub(crate) fn register_document(
        &self,
        slot: DocumentSlot,
    ) -> Result<DocumentTicket, RegistrationError> {
        let ticket = self
            .pool
            .with_state(|state, now| state.register_document(self.key, slot, now))?;
        Ok(DocumentTicket {
            handle: DocumentBudgetHandle {
                page: self.clone(),
                slot,
                ticket,
            },
            canceled: false,
        })
    }

    pub(crate) fn maintenance(&self, last_applied: TrimEpoch) -> Option<TrimRequest> {
        // The ledger advances deadlines before consulting the epoch.
        self.pool
            .with_state(|state, now| state.maintenance(self.key, last_applied, now))
    }

    pub(crate) fn acknowledge_trim(&self, request: TrimRequest) -> TrimStatus {
        self.pool
            .with_state(|state, now| state.acknowledge_trim(self.key, request, now))
    }

    pub(crate) fn current_demand_slot(&self) -> Option<DocumentSlot> {
        self.pool
            .with_state(|state, now| state.current_demand_slot(self.key, now))
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct DocumentSlot {
    pub(crate) target: WindowDocumentTaskTarget,
    pub(crate) document: DomHandle,
    pub(crate) root: DomHandle,
}

/// Separate from cache ownership: a self-trim must leave this guard alive.
#[derive(Debug)]
pub(crate) struct DocumentTicket {
    handle: DocumentBudgetHandle,
    canceled: bool,
}

impl DocumentTicket {
    pub(crate) fn handle(&self) -> DocumentBudgetHandle {
        self.handle.clone()
    }

    pub(crate) fn cancel(&mut self) {
        if !self.canceled {
            self.canceled = true;
            self.handle.page.pool.with_state(|state, now| {
                state.cancel_document(self.handle.page.key, self.handle.ticket, now);
            });
        }
    }
}

impl Drop for DocumentTicket {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DocumentBudgetHandle {
    page: PageBudgetHandle,
    slot: DocumentSlot,
    ticket: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AdmissionStamp {
    page: PageKey,
    document: u64,
    trim_epoch: TrimEpoch,
    pressure_epoch: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PassRetentionPermit {
    Enabled(AdmissionStamp),
    Disabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RetentionDenial {
    Closed,
    StalePass,
    InvalidLease,
    AbsoluteLimit,
    Pressure,
    PendingTrim,
    Capacity,
    Queued,
    Exhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RetentionDenied {
    pub(crate) reason: RetentionDenial,
    pub(crate) demand: Option<ValidatedDemand>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ValidatedDemand {
    page: PageKey,
    document: u64,
    bytes: usize,
    pressure_epoch: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DemandStatus {
    Enqueued,
    Updated,
    AlreadyWaiting,
    Rejected,
}

impl DocumentBudgetHandle {
    pub(crate) fn begin_pass(&self) -> PassRetentionPermit {
        self.page
            .pool
            .with_state(|state, now| state.begin_pass(self.page.key, self.ticket, now))
    }

    /// Called only after CurrentScreen owner/input validation. A failed
    /// admission leaves the supplied lease unchanged so the caller can release
    /// its payload first. Shrinking assumes old buffers have already dropped.
    pub(crate) fn try_commit_retained(
        &self,
        stamp: AdmissionStamp,
        new_bytes: usize,
        lease: &mut Option<WorkingSetLease>,
    ) -> Result<(), RetentionDenied> {
        if lease
            .as_ref()
            .is_some_and(|lease| !lease.pool.shares_state_with(&self.page.pool))
        {
            return Err(RetentionDenied {
                reason: RetentionDenial::InvalidLease,
                demand: None,
            });
        }
        let old_id = lease.as_ref().map(|lease| lease.id);
        let id = self.page.pool.with_state(|state, now| {
            state.try_commit_retained(self.page.key, self.ticket, stamp, new_bytes, old_id, now)
        })?;
        if let Some(lease) = lease {
            lease.bytes = new_bytes;
        } else {
            *lease = Some(WorkingSetLease {
                pool: self.page.pool.clone(),
                id,
                bytes: new_bytes,
            });
        }
        Ok(())
    }

    /// The caller has dropped the rejected payload and its old lease, and its
    /// CurrentScreen publication has succeeded. This never owns cache data.
    pub(crate) fn activate_demand(&self, demand: ValidatedDemand) -> DemandStatus {
        self.page
            .pool
            .with_state(|state, now| state.activate_demand(self.page.key, self.ticket, demand, now))
    }

    pub(crate) const fn slot(&self) -> DocumentSlot {
        self.slot
    }
}

#[derive(Debug)]
pub(crate) struct WorkingSetLease {
    pool: LayoutWorkingSetBudget,
    id: u64,
    bytes: usize,
}

impl WorkingSetLease {
    pub(crate) const fn bytes(&self) -> usize {
        self.bytes
    }
}

impl Drop for WorkingSetLease {
    fn drop(&mut self) {
        self.pool
            .with_state(|state, now| state.release_lease(self.id, now));
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct TrimEpoch(pub(crate) u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TrimCause {
    Fairness,
    SelfTrim,
    Pressure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TrimRequest {
    pub(crate) epoch: TrimEpoch,
    pub(crate) cause: TrimCause,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TrimStatus {
    Complete,
    PendingRelease,
    Stale,
}
