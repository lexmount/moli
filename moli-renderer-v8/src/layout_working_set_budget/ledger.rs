use std::{
    collections::{HashMap, VecDeque},
    time::Instant,
};

use super::{
    AdmissionStamp, BudgetConfigError, BudgetLimits, BudgetSnapshot, BudgetTiming, DemandStatus,
    DocumentSlot, PassRetentionPermit, PressureChange, RegistrationError, RendererPageToken,
    RetentionDenial, RetentionDenied, TrimCause, TrimEpoch, TrimRequest, TrimStatus,
    ValidatedDemand,
};

#[cfg(test)]
use super::{DemandSnapshot, HandoffSnapshot, PageBudgetSnapshot};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct PageKey {
    token: RendererPageToken,
    registration: u64,
}

struct PageAccount {
    registration: u64,
    open: bool,
    charged_bytes: usize,
    documents: HashMap<u64, DocumentSlot>,
    demand: Option<Demand>,
    resident_since: Option<Instant>,
    protected_until: Option<Instant>,
    last_use: Option<Instant>,
    trim_epoch: TrimEpoch,
    trim: Option<TrimRequest>,
}

#[derive(Clone, Copy)]
struct Demand {
    document: u64,
    slot: DocumentSlot,
    bytes: usize,
    #[cfg(test)]
    enqueued_at: Instant,
    last_demand_at: Instant,
    claim_deadline: Option<Instant>,
}

#[derive(Clone, Copy)]
struct LeaseAccount {
    page: PageKey,
    document: u64,
    bytes: usize,
}

#[derive(Clone, Copy)]
enum Handoff {
    WaitingRelease {
        head: PageKey,
    },
    ReadyToClaim {
        head: PageKey,
        bytes: usize,
        // Expiry uses Demand::claim_deadline. This copy is test diagnostics only.
        #[cfg(test)]
        deadline: Instant,
    },
}

impl Handoff {
    const fn head(self) -> PageKey {
        match self {
            Self::WaitingRelease { head } | Self::ReadyToClaim { head, .. } => head,
        }
    }
}

pub(super) struct Ledger {
    limits: BudgetLimits,
    timing: BudgetTiming,
    pressure_target: usize,
    pressure_epoch: u64,
    charged_bytes: usize,
    pages: HashMap<RendererPageToken, PageAccount>,
    leases: HashMap<u64, LeaseAccount>,
    fifo: VecDeque<PageKey>,
    handoff: Option<Handoff>,
    next_id: u64,
    exhausted: bool,
    trim_requests: usize,
    last_observed_now: Instant,
}

impl Ledger {
    pub(super) const fn limits(&self) -> BudgetLimits {
        self.limits
    }

    pub(super) fn new(limits: BudgetLimits, timing: BudgetTiming, now: Instant) -> Self {
        Self {
            limits,
            timing,
            pressure_target: limits.browser_bytes,
            pressure_epoch: 0,
            charged_bytes: 0,
            pages: HashMap::new(),
            leases: HashMap::new(),
            fifo: VecDeque::new(),
            handoff: None,
            next_id: 1,
            exhausted: false,
            trim_requests: 0,
            last_observed_now: now,
        }
    }

    pub(super) fn observe_time(&mut self, sampled: Instant) -> Instant {
        self.last_observed_now = self.last_observed_now.max(sampled);
        self.last_observed_now
    }

    fn allocate_id(&mut self) -> Option<u64> {
        if self.exhausted {
            return None;
        }
        let next = self.next_id.checked_add(1);
        let Some(next) = next else {
            self.exhausted = true;
            return None;
        };
        let id = self.next_id;
        self.next_id = next;
        Some(id)
    }

    fn page(&self, key: PageKey) -> Option<&PageAccount> {
        self.pages
            .get(&key.token)
            .filter(|page| page.registration == key.registration)
    }

    fn page_mut(&mut self, key: PageKey) -> Option<&mut PageAccount> {
        self.pages
            .get_mut(&key.token)
            .filter(|page| page.registration == key.registration)
    }

    fn document_is_live(&self, page: PageKey, document: u64) -> bool {
        self.page(page)
            .is_some_and(|page| page.open && page.documents.contains_key(&document))
    }

    pub(super) fn register_page(
        &mut self,
        token: RendererPageToken,
        now: Instant,
    ) -> Result<PageKey, RegistrationError> {
        self.advance(now);
        if self.pages.contains_key(&token) {
            return Err(RegistrationError::DuplicatePage);
        }
        let registration = self.allocate_id().ok_or(RegistrationError::Exhausted)?;
        self.pages.insert(
            token,
            PageAccount {
                registration,
                open: true,
                charged_bytes: 0,
                documents: HashMap::new(),
                demand: None,
                resident_since: None,
                protected_until: None,
                last_use: None,
                trim_epoch: TrimEpoch::default(),
                trim: None,
            },
        );
        Ok(PageKey {
            token,
            registration,
        })
    }

    pub(super) fn register_document(
        &mut self,
        key: PageKey,
        slot: DocumentSlot,
        now: Instant,
    ) -> Result<u64, RegistrationError> {
        self.advance(now);
        let page = self
            .page(key)
            .filter(|page| page.open)
            .ok_or(RegistrationError::Closed)?;
        if page.documents.values().any(|existing| *existing == slot) {
            return Err(RegistrationError::DuplicateSlot);
        }
        let id = self.allocate_id().ok_or(RegistrationError::Exhausted)?;
        self.page_mut(key)
            .expect("validated Page")
            .documents
            .insert(id, slot);
        Ok(id)
    }

    pub(super) fn close_page(&mut self, key: PageKey, now: Instant) {
        if let Some(page) = self.page_mut(key) {
            page.open = false;
            page.documents.clear();
        }
        self.cancel_demand(key);
        self.remove_closed_empty_page(key);
        // No charge changes here. A checked-out pass may still own a lease.
        self.advance(now);
    }

    pub(super) fn cancel_document(&mut self, key: PageKey, document: u64, now: Instant) {
        let cancel_demand = self.page_mut(key).is_some_and(|page| {
            page.documents.remove(&document);
            page.demand
                .is_some_and(|demand| demand.document == document)
        });
        if cancel_demand {
            self.cancel_demand(key);
        }
        self.advance(now);
    }

    fn remove_closed_empty_page(&mut self, key: PageKey) {
        if self
            .page(key)
            .is_some_and(|page| !page.open && page.charged_bytes == 0)
        {
            self.pages.remove(&key.token);
        }
    }

    pub(super) fn begin_pass(
        &mut self,
        key: PageKey,
        document: u64,
        now: Instant,
    ) -> PassRetentionPermit {
        self.advance(now);
        if self.pressure_target == 0 || !self.document_is_live(key, document) {
            return PassRetentionPermit::Disabled;
        }
        let pressure_epoch = self.pressure_epoch;
        let page = self.page_mut(key).expect("live document Page");
        page.last_use = Some(now);
        PassRetentionPermit::Enabled(AdmissionStamp {
            page: key,
            document,
            trim_epoch: page.trim_epoch,
            pressure_epoch,
        })
    }

    fn denied(
        &self,
        reason: RetentionDenial,
        key: PageKey,
        document: u64,
        bytes: usize,
    ) -> RetentionDenied {
        let can_wait = matches!(
            reason,
            RetentionDenial::Capacity | RetentionDenial::Queued | RetentionDenial::PendingTrim
        ) && self.document_is_live(key, document)
            && self.candidate_fits_absolute_limits(bytes)
            && bytes <= self.pressure_target;
        RetentionDenied {
            reason,
            demand: can_wait.then_some(ValidatedDemand {
                page: key,
                document,
                bytes,
                pressure_epoch: self.pressure_epoch,
            }),
        }
    }

    fn candidate_fits_absolute_limits(&self, bytes: usize) -> bool {
        bytes > 0
            && bytes <= self.limits.document_bytes
            && bytes <= self.limits.page_bytes
            && bytes <= self.limits.browser_bytes
    }

    pub(super) fn try_commit_retained(
        &mut self,
        key: PageKey,
        document: u64,
        stamp: AdmissionStamp,
        bytes: usize,
        old_lease: Option<u64>,
        now: Instant,
    ) -> Result<u64, RetentionDenied> {
        // Expiration precedes admission, including admission by the former
        // Ready owner. A pass stamp never conveys a future allocation grant.
        self.advance(now);
        if self.exhausted {
            return Err(self.denied(RetentionDenial::Exhausted, key, document, bytes));
        }
        if !self.document_is_live(key, document) {
            return Err(self.denied(RetentionDenial::Closed, key, document, bytes));
        }
        let page = self.page(key).expect("live Page");
        if stamp.page != key
            || stamp.document != document
            || stamp.pressure_epoch != self.pressure_epoch
            || stamp.trim_epoch != page.trim_epoch
        {
            return Err(self.denied(RetentionDenial::StalePass, key, document, bytes));
        }
        if !self.candidate_fits_absolute_limits(bytes) {
            return Err(self.denied(RetentionDenial::AbsoluteLimit, key, document, bytes));
        }
        if bytes > self.pressure_target {
            return Err(self.denied(RetentionDenial::Pressure, key, document, bytes));
        }
        let old_bytes = match old_lease {
            Some(id) => match self.leases.get(&id) {
                Some(lease) if lease.page == key && lease.document == document => lease.bytes,
                _ => return Err(self.denied(RetentionDenial::InvalidLease, key, document, bytes)),
            },
            None => {
                // A Document slot owns one movable lease. Creating another
                // lease instead of resizing the live one would bypass the
                // per-Document bound. Only new admission needs this search;
                // ordinary resize uses the lease id above.
                if self
                    .leases
                    .values()
                    .any(|lease| lease.page == key && lease.document == document)
                {
                    return Err(self.denied(RetentionDenial::InvalidLease, key, document, bytes));
                }
                0
            }
        };
        // An already active exact-slot demand may change size without moving
        // to the tail or renewing an already-started claim deadline.
        if let Some(demand) = self.page_mut(key).and_then(|page| page.demand.as_mut())
            && demand.document == document
        {
            demand.bytes = bytes;
            demand.last_demand_at = now;
            self.advance(now);
        }
        let page = self.page(key).expect("live Page remains registered");
        if page.trim.is_some() {
            return Err(self.denied(RetentionDenial::PendingTrim, key, document, bytes));
        }
        if page
            .demand
            .is_some_and(|demand| demand.document != document)
        {
            return Err(self.denied(RetentionDenial::Queued, key, document, bytes));
        }
        let total = self
            .charged_bytes
            .checked_sub(old_bytes)
            .and_then(|value| value.checked_add(bytes));
        let page_total = page
            .charged_bytes
            .checked_sub(old_bytes)
            .and_then(|value| value.checked_add(bytes));
        let (Some(total), Some(page_total)) = (total, page_total) else {
            return Err(self.denied(RetentionDenial::Capacity, key, document, bytes));
        };
        if total > self.pressure_target
            || total > self.limits.browser_bytes
            || page_total > self.limits.page_bytes
        {
            return Err(self.denied(RetentionDenial::Capacity, key, document, bytes));
        }
        if bytes > old_bytes {
            let can_grow = match self.handoff {
                Some(Handoff::WaitingRelease { head }) => head == key,
                Some(Handoff::ReadyToClaim {
                    head,
                    bytes: reserved,
                    ..
                }) if head != key => total
                    .checked_add(reserved)
                    .is_some_and(|sum| sum <= self.pressure_target),
                Some(Handoff::ReadyToClaim { .. }) | None => true,
            };
            if !can_grow {
                return Err(self.denied(RetentionDenial::Queued, key, document, bytes));
            }
        }
        let protected_until = if page.resident_since.is_none() {
            Some(
                now.checked_add(self.timing.protection)
                    .ok_or_else(|| self.denied(RetentionDenial::Exhausted, key, document, bytes))?,
            )
        } else {
            page.protected_until
        };
        let id = match old_lease {
            Some(id) => id,
            None => self
                .allocate_id()
                .ok_or_else(|| self.denied(RetentionDenial::Exhausted, key, document, bytes))?,
        };
        self.charged_bytes = total;
        self.leases.insert(
            id,
            LeaseAccount {
                page: key,
                document,
                bytes,
            },
        );
        let page = self.page_mut(key).expect("live Page");
        page.charged_bytes = page_total;
        if page.resident_since.is_none() {
            page.resident_since = Some(now);
        }
        page.protected_until = protected_until;
        page.last_use = Some(now);
        if page
            .demand
            .is_some_and(|demand| demand.document == document)
        {
            self.cancel_demand(key);
        }
        self.advance(now);
        Ok(id)
    }

    pub(super) fn activate_demand(
        &mut self,
        key: PageKey,
        document: u64,
        demand: ValidatedDemand,
        now: Instant,
    ) -> DemandStatus {
        self.advance(now);
        if self.exhausted
            || demand.page != key
            || demand.document != document
            || demand.pressure_epoch != self.pressure_epoch
            || !self.document_is_live(key, document)
            || !self.candidate_fits_absolute_limits(demand.bytes)
            || demand.bytes > self.pressure_target
        {
            return DemandStatus::Rejected;
        }
        let page = self.page_mut(key).expect("live Page");
        let status = if let Some(existing) = page.demand.as_mut() {
            if existing.document != document {
                return DemandStatus::AlreadyWaiting;
            }
            existing.bytes = demand.bytes;
            existing.last_demand_at = now;
            DemandStatus::Updated
        } else {
            page.demand = Some(Demand {
                document,
                slot: *page.documents.get(&document).expect("live Document"),
                bytes: demand.bytes,
                #[cfg(test)]
                enqueued_at: now,
                last_demand_at: now,
                claim_deadline: None,
            });
            self.fifo.push_back(key);
            DemandStatus::Enqueued
        };
        self.advance(now);
        status
    }

    pub(super) fn release_lease(&mut self, id: u64, now: Instant) {
        let lease = self
            .leases
            .remove(&id)
            .expect("a live lease is released exactly once");
        self.charged_bytes = self
            .charged_bytes
            .checked_sub(lease.bytes)
            .expect("lease total is charged");
        let page = self
            .page_mut(lease.page)
            .expect("charged Page account remains alive");
        page.charged_bytes = page
            .charged_bytes
            .checked_sub(lease.bytes)
            .expect("Page lease is charged");
        if page.charged_bytes == 0 {
            page.resident_since = None;
            page.protected_until = None;
            page.trim = None;
        }
        self.remove_closed_empty_page(lease.page);
        // Refund first, then reconsider victims. Selecting victims before this
        // real release could evict an unrelated Page for a shortfall just freed.
        self.advance(now);
    }

    fn cancel_demand(&mut self, key: PageKey) {
        if let Some(page) = self.page_mut(key) {
            page.demand = None;
        }
        self.fifo.retain(|entry| *entry != key);
        if self.handoff.is_some_and(|handoff| handoff.head() == key) {
            self.handoff = None;
        }
    }

    fn request_trim(&mut self, key: PageKey, cause: TrimCause) {
        let Some(page) = self.page(key) else {
            return;
        };
        if !page.open || page.charged_bytes == 0 || page.trim.is_some() {
            return;
        }
        // Normal identities never consume MAX. Once they are exhausted, no
        // new retention can succeed, but existing payloads must still be
        // reclaimable (including immediate zero-pressure reclamation).
        let epoch = self.allocate_id().unwrap_or(u64::MAX);
        let request = TrimRequest {
            epoch: TrimEpoch(epoch),
            cause,
        };
        let page = self.page_mut(key).expect("trim Page remains live");
        page.trim_epoch = request.epoch;
        page.trim = Some(request);
        self.trim_requests = self.trim_requests.saturating_add(1);
    }

    fn projected_charge_after_trims(&self) -> usize {
        self.pages
            .values()
            .fold(self.charged_bytes, |remaining, page| {
                if page.trim.is_some() {
                    remaining.saturating_sub(page.charged_bytes)
                } else {
                    remaining
                }
            })
    }

    fn victims(&self, now: Instant, force: bool, exclude: Option<PageKey>) -> Vec<PageKey> {
        let mut candidates = self
            .pages
            .iter()
            .filter_map(|(token, page)| {
                let key = PageKey {
                    token: *token,
                    registration: page.registration,
                };
                if !page.open
                    || page.charged_bytes == 0
                    || page.trim.is_some()
                    || exclude == Some(key)
                    || (!force && page.protected_until.is_some_and(|until| until > now))
                {
                    return None;
                }
                let idle = page.last_use.is_none_or(|last| {
                    now.saturating_duration_since(last) >= self.timing.idle_after
                });
                Some((
                    !idle,
                    page.resident_since.unwrap_or(now),
                    page.registration,
                    key,
                ))
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|(active, since, registration, _)| (*active, *since, *registration));
        candidates.into_iter().map(|(_, _, _, key)| key).collect()
    }

    /// Time is observed only at real demand, refund, configuration and existing
    /// owner-maintenance boundaries. No work or timer is created by this pool.
    pub(super) fn advance(&mut self, now: Instant) {
        if self.fifo.is_empty()
            && self.handoff.is_none()
            && self.charged_bytes <= self.pressure_target
        {
            return;
        }
        let expired = self
            .fifo
            .iter()
            .copied()
            .filter(|key| {
                self.page(*key).is_none_or(|page| {
                    !page.open
                        || page.demand.is_none_or(|demand| {
                            !page.documents.contains_key(&demand.document)
                                || demand.bytes > self.pressure_target
                                || now.saturating_duration_since(demand.last_demand_at)
                                    >= self.timing.demand_ttl
                                || demand
                                    .claim_deadline
                                    .is_some_and(|deadline| deadline <= now)
                        })
                })
            })
            .collect::<Vec<_>>();
        for key in expired {
            self.cancel_demand(key);
        }

        // Positive pressure respects an existing fixed protection interval;
        // zero pressure overrides it. Neither requires a competing waiter.
        if self.charged_bytes > self.pressure_target {
            for victim in self.victims(now, self.pressure_target == 0, None) {
                if self.projected_charge_after_trims() <= self.pressure_target {
                    break;
                }
                self.request_trim(victim, TrimCause::Pressure);
            }
        }
        let Some(head) = self.fifo.front().copied() else {
            self.handoff = None;
            return;
        };
        let demand = self
            .page(head)
            .and_then(|page| page.demand)
            .expect("FIFO names active demand");
        let page = self.page(head).expect("demand Page");
        let head_charged = page.charged_bytes;
        let head_has_trim = page.trim.is_some();
        let browser_fits = self
            .charged_bytes
            .checked_add(demand.bytes)
            .is_some_and(|sum| sum <= self.pressure_target);
        let page_fits = head_charged
            .checked_add(demand.bytes)
            .is_some_and(|sum| sum <= self.limits.page_bytes);
        if browser_fits && page_fits && !head_has_trim {
            let deadline = match demand.claim_deadline {
                Some(deadline) => deadline,
                None => match now.checked_add(self.timing.claim_ttl) {
                    Some(deadline) => deadline,
                    None => {
                        self.cancel_demand(head);
                        return;
                    }
                },
            };
            self.page_mut(head)
                .expect("demand Page")
                .demand
                .as_mut()
                .expect("demand")
                .claim_deadline = Some(deadline);
            self.handoff = Some(Handoff::ReadyToClaim {
                head,
                bytes: demand.bytes,
                #[cfg(test)]
                deadline,
            });
            return;
        }
        self.handoff = Some(Handoff::WaitingRelease { head });
        // The requester may need to relinquish a different local slot. This
        // trim keeps its demand and is not protected against its own request.
        if head_charged > 0 && (!browser_fits || !page_fits) {
            self.request_trim(head, TrimCause::SelfTrim);
        }
        for victim in self.victims(now, false, Some(head)) {
            if self
                .projected_charge_after_trims()
                .checked_add(demand.bytes)
                .is_some_and(|sum| sum <= self.pressure_target)
            {
                break;
            }
            self.request_trim(victim, TrimCause::Fairness);
        }
    }

    pub(super) fn set_pressure_target(
        &mut self,
        bytes: usize,
        now: Instant,
    ) -> Result<PressureChange, BudgetConfigError> {
        if bytes > self.limits.browser_bytes {
            return Err(BudgetConfigError(
                "pressure target exceeds absolute Browser limit",
            ));
        }
        let changed = bytes != self.pressure_target;
        if changed {
            let Some(epoch) = self.pressure_epoch.checked_add(1) else {
                self.exhausted = true;
                return Err(BudgetConfigError("pressure epoch exhausted"));
            };
            self.pressure_epoch = epoch;
            self.pressure_target = bytes;
            // A pressure change invalidates all in-flight stamps. Zero also
            // retires demand, rather than letting every failing frame requeue.
            if bytes == 0 {
                for page in self.pages.values_mut() {
                    page.demand = None;
                }
                self.fifo.clear();
                self.handoff = None;
            }
        }
        self.advance(now);
        Ok(PressureChange {
            changed,
            epoch: self.pressure_epoch,
            target_bytes: bytes,
        })
    }

    pub(super) fn maintenance(
        &mut self,
        key: PageKey,
        last_applied: TrimEpoch,
        now: Instant,
    ) -> Option<TrimRequest> {
        self.advance(now);
        self.page(key)
            .filter(|page| page.open)?
            .trim
            .filter(|request| request.epoch > last_applied)
    }

    pub(super) fn acknowledge_trim(
        &mut self,
        key: PageKey,
        request: TrimRequest,
        now: Instant,
    ) -> TrimStatus {
        self.advance(now);
        let Some(page) = self.page(key) else {
            return TrimStatus::Stale;
        };
        if page.trim_epoch != request.epoch {
            return TrimStatus::Stale;
        }
        if page.charged_bytes == 0 {
            TrimStatus::Complete
        } else {
            TrimStatus::PendingRelease
        }
    }

    pub(super) fn pending_demand_slot(&self, key: PageKey) -> Option<DocumentSlot> {
        self.page(key)?.demand.map(|demand| demand.slot)
    }

    pub(super) fn current_demand_slot(
        &mut self,
        key: PageKey,
        now: Instant,
    ) -> Option<DocumentSlot> {
        self.advance(now);
        self.page(key)?.demand.map(|demand| demand.slot)
    }

    pub(super) fn snapshot(&self) -> BudgetSnapshot {
        #[cfg(test)]
        let mut pages = self
            .pages
            .iter()
            .map(|(token, page)| PageBudgetSnapshot {
                token: *token,
                registration: page.registration,
                open: page.open,
                charged_bytes: page.charged_bytes,
                documents: page.documents.len(),
                demand: page.demand.map(|demand| DemandSnapshot {
                    slot: demand.slot,
                    bytes: demand.bytes,
                    enqueued_at: demand.enqueued_at,
                    last_demand_at: demand.last_demand_at,
                    claim_deadline: demand.claim_deadline,
                }),
                trim: page.trim,
                resident_since: page.resident_since,
                protected_until: page.protected_until,
            })
            .collect::<Vec<_>>();
        #[cfg(test)]
        pages.sort_by_key(|page| page.registration);
        BudgetSnapshot {
            charged_bytes: self.charged_bytes,
            pressure_target: self.pressure_target,
            pressure_epoch: self.pressure_epoch,
            open_pages: self.pages.values().filter(|page| page.open).count(),
            live_leases: self.leases.len(),
            queued_pages: self.fifo.len(),
            ready_bytes: match self.handoff {
                Some(Handoff::ReadyToClaim { bytes, .. }) => bytes,
                _ => 0,
            },
            trim_requests: self.trim_requests,
            #[cfg(test)]
            pages,
            #[cfg(test)]
            fifo: self.fifo.iter().map(|key| key.token).collect(),
            #[cfg(test)]
            handoff: self.handoff.map(|handoff| match handoff {
                Handoff::WaitingRelease { head } => {
                    HandoffSnapshot::WaitingRelease { head: head.token }
                }
                Handoff::ReadyToClaim {
                    head,
                    bytes,
                    deadline,
                } => HandoffSnapshot::ReadyToClaim {
                    head: head.token,
                    reserved_bytes: bytes,
                    deadline,
                },
            }),
            #[cfg(test)]
            last_observed_now: self.last_observed_now,
        }
    }

    #[cfg(test)]
    pub(super) fn set_next_id_for_test(&mut self, next: u64) {
        self.next_id = next;
    }
}
