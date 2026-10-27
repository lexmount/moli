use std::collections::{BTreeMap, HashMap, HashSet};

#[cfg(test)]
use moli_layout::LayoutWorkingSetStats;
use moli_layout::{LayoutWorkingSet, LayoutWorkingSetLimits};

use crate::{
    document_runtime::DomHandle,
    layout_working_set_budget::{
        AdmissionStamp, BudgetLimits, DocumentBudgetHandle, DocumentSlot, DocumentTicket,
        PageBudgetHandle, PassRetentionPermit, TrimEpoch, TrimStatus, WorkingSetLease,
    },
    native_bridge::WindowDocumentTaskTarget,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NativeLayoutWorkingSetOwner {
    pub(crate) document: DomHandle,
    pub(crate) root: DomHandle,
    pub(crate) target: WindowDocumentTaskTarget,
}

impl NativeLayoutWorkingSetOwner {
    fn budget_slot(self) -> DocumentSlot {
        DocumentSlot {
            target: self.target,
            document: self.document,
            root: self.root,
        }
    }
}

pub(super) struct NativeDocumentWorkingSet {
    owner: NativeLayoutWorkingSetOwner,
    // Keep the existing backend borrowing interface. The lease stays beside
    // this value while its payload moves through retained inputs and World.
    pub(super) working_set: LayoutWorkingSet<DomHandle>,
    lease: Option<WorkingSetLease>,
}

impl Drop for NativeDocumentWorkingSet {
    fn drop(&mut self) {
        // A lease describes real retained state. Never refund before dropping
        // the backend's owned buffers, including error/unwind paths.
        self.working_set.clear();
        drop(self.lease.take());
    }
}

struct NativeWorkingSetTicket {
    owner: NativeLayoutWorkingSetOwner,
    ticket: DocumentTicket,
}

struct NativePageWorkingSetBudget {
    page: PageBudgetHandle,
    limits: BudgetLimits,
    // This registry is not part of the evictable cache or checked-out pass.
    // A denied candidate may own a demand despite owning no working tree.
    tickets: HashMap<DomHandle, NativeWorkingSetTicket>,
    last_applied_trim: TrimEpoch,
}

impl NativePageWorkingSetBudget {
    fn retain_tickets(&mut self, keep: impl Fn(&NativeWorkingSetTicket) -> bool) {
        if self.tickets.is_empty() {
            return;
        }
        // A Page has at most one queued demand. Cancel a retired demand first:
        // dropping another ticket or lease advances shared deadlines and must
        // not trim a peer for a slot already known to be dead. The peek itself
        // deliberately does not advance the ledger.
        if let Some(slot) = self.page.pending_demand_slot_for_retirement()
            && self
                .tickets
                .get(&slot.document)
                .is_some_and(|entry| entry.owner.budget_slot() == slot && !keep(entry))
        {
            self.tickets.remove(&slot.document);
        }
        self.tickets.retain(|_, entry| keep(entry));
    }
}

/// Installing a production Page binding is distinct from owning a handle.
/// Registration failure must not select the standalone harness's local cache.
#[derive(Default)]
enum NativeWorkingSetBudgetBinding {
    #[default]
    Unmanaged,
    ManagedDisabled,
    Managed(NativePageWorkingSetBudget),
}

impl NativeWorkingSetBudgetBinding {
    fn managed(&self) -> Option<&NativePageWorkingSetBudget> {
        match self {
            Self::Managed(budget) => Some(budget),
            Self::Unmanaged | Self::ManagedDisabled => None,
        }
    }

    fn managed_mut(&mut self) -> Option<&mut NativePageWorkingSetBudget> {
        match self {
            Self::Managed(budget) => Some(budget),
            Self::Unmanaged | Self::ManagedDisabled => None,
        }
    }
}

/// Realtime's mutable computation state, separate from published geometry.
/// No instance is installed on a Mock or OnDemand Host. Page registration is
/// owned by the stable renderer Page; this state only borrows its handle.
#[derive(Default)]
pub(crate) struct NativeLayoutWorkingSets {
    documents: HashMap<DomHandle, NativeDocumentWorkingSet>,
    budget: NativeWorkingSetBudgetBinding,
}

impl Drop for NativeLayoutWorkingSets {
    fn drop(&mut self) {
        self.retire_all_documents();
    }
}

impl NativeLayoutWorkingSets {
    /// Always installed by production Page bootstrap, before any layout pass.
    /// None records managed-but-disabled admission, e.g. ID exhaustion. Only
    /// a standalone ScriptVm that never installs a binding uses local caching.
    pub(crate) fn bind_page_budget(
        &mut self,
        page: Option<PageBudgetHandle>,
    ) -> Result<(), &'static str> {
        match (&self.budget, &page) {
            (NativeWorkingSetBudgetBinding::Unmanaged, _) => {}
            (NativeWorkingSetBudgetBinding::ManagedDisabled, None) => return Ok(()),
            (NativeWorkingSetBudgetBinding::Managed(existing), Some(page))
                if existing.page.shares_registration_with(page) =>
            {
                return Ok(());
            }
            _ => return Err("Realtime working state cannot switch its stable Page binding"),
        }
        self.clear();
        self.budget = match page {
            Some(page) => NativeWorkingSetBudgetBinding::Managed(NativePageWorkingSetBudget {
                limits: page.limits(),
                page,
                tickets: HashMap::new(),
                last_applied_trim: TrimEpoch::default(),
            }),
            None => NativeWorkingSetBudgetBinding::ManagedDisabled,
        };
        Ok(())
    }

    /// Budget eviction releases payloads but preserves exact-slot demand.
    pub(crate) fn clear(&mut self) {
        // The cache's high-water container capacity has no lifetime contract.
        // Keep only the independent ticket registry when trimming payloads.
        drop(std::mem::take(&mut self.documents));
    }

    /// Document replacement/Host teardown revoke tickets independently of
    /// payload presence. The stable Page registration remains untouched.
    pub(crate) fn retire_all_documents(&mut self) {
        if let Some(budget) = self.budget.managed_mut() {
            budget.retain_tickets(|_| false);
        }
        self.clear();
    }

    pub(crate) fn retire_owner(&mut self, owner: crate::native_bridge::WindowDocumentOwner) {
        if let Some(budget) = self.budget.managed_mut() {
            budget.retain_tickets(|entry| entry.owner.target.owner() != owner);
        }
        self.documents
            .retain(|_, entry| entry.owner.target.owner() != owner);
    }

    /// Invoked at existing owner-maintenance opportunities and before a real
    /// new layout pass. It neither invalidates geometry nor requests a frame.
    pub(crate) fn maintain_budget(&mut self) -> Option<TrimStatus> {
        let budget = self.budget.managed()?;
        let request = budget.page.maintenance(budget.last_applied_trim)?;
        // No ledger lock is held across buffer destruction. Other transactions
        // may still own leases; acknowledging never pretends those are freed.
        self.clear();
        let budget = self
            .budget
            .managed_mut()
            .expect("maintenance keeps Page binding");
        let status = budget.page.acknowledge_trim(request);
        if status == TrimStatus::Complete {
            budget.last_applied_trim = request.epoch;
        }
        Some(status)
    }

    pub(crate) fn prepare_pass(
        &mut self,
        owners: impl IntoIterator<Item = NativeLayoutWorkingSetOwner>,
    ) -> NativeLayoutWorkingSetPass {
        let owners = owners.into_iter().collect::<Vec<_>>();
        let current = owners
            .iter()
            .map(|owner| (owner.document, *owner))
            .collect::<HashMap<_, _>>();
        // A root replacement is a new exact slot even when Document is the
        // same. Drop dead payloads rather than lending them to a new ticket.
        if let Some(budget) = self.budget.managed_mut() {
            budget.retain_tickets(|entry| current.get(&entry.owner.document) == Some(&entry.owner));
        }
        self.documents
            .retain(|document, entry| current.get(document) == Some(&entry.owner));
        if let Some(budget) = self.budget.managed_mut() {
            for owner in &owners {
                if budget.tickets.contains_key(&owner.document) {
                    continue;
                }
                match budget.page.register_document(owner.budget_slot()) {
                    Ok(ticket) => {
                        budget.tickets.insert(
                            owner.document,
                            NativeWorkingSetTicket {
                                owner: *owner,
                                ticket,
                            },
                        );
                    }
                    Err(error) => {
                        // The geometry backend still runs without a working
                        // set. Exhausted/closed metadata cannot break layout.
                        tracing::debug!(?error, document = ?owner.document,
                            "layout working-set Document admission unavailable");
                    }
                }
            }
        }
        let _ = self.maintain_budget();
        let pending_slot = self
            .budget
            .managed()
            .and_then(|budget| budget.page.current_demand_slot());
        let (page_limit, document_limit) = match &self.budget {
            // Preserve the existing standalone ScriptVm harness contract.
            // Production bootstrap always selects one of the managed states.
            NativeWorkingSetBudgetBinding::Unmanaged => (
                crate::layout_working_set_budget::DEFAULT_PAGE_WORKING_SET_BYTES,
                LayoutWorkingSetLimits::default().max_estimated_retained_bytes,
            ),
            NativeWorkingSetBudgetBinding::ManagedDisabled => (0, 0),
            NativeWorkingSetBudgetBinding::Managed(budget) => {
                (budget.limits.page_bytes, budget.limits.document_bytes)
            }
        };
        let owners = owners
            .into_iter()
            .map(|owner| {
                let budget = match &self.budget {
                    NativeWorkingSetBudgetBinding::Unmanaged => NativePassDocumentBudget::Unmanaged,
                    NativeWorkingSetBudgetBinding::ManagedDisabled => {
                        NativePassDocumentBudget::ManagedDisabled
                    }
                    NativeWorkingSetBudgetBinding::Managed(budget) => budget
                        .tickets
                        .get(&owner.document)
                        .map_or(NativePassDocumentBudget::ManagedDisabled, |entry| {
                            NativePassDocumentBudget::Managed(entry.ticket.handle())
                        }),
                };
                (owner.document, NativePassDocument { owner, budget })
            })
            .collect();
        NativeLayoutWorkingSetPass {
            owners,
            previous: std::mem::take(&mut self.documents),
            completed: BTreeMap::new(),
            admissions: HashMap::new(),
            visited: HashSet::new(),
            pending_slot,
            completed_charge: 0,
            max_estimated_retained_bytes: page_limit,
            document_limit,
        }
    }

    #[cfg(test)]
    pub(crate) fn stats(&self, document: DomHandle) -> Option<LayoutWorkingSetStats> {
        self.documents
            .get(&document)
            .map(|entry| entry.working_set.stats())
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.documents.len()
    }

    #[cfg(test)]
    pub(crate) fn ticket_count(&self) -> usize {
        self.budget
            .managed()
            .map_or(0, |budget| budget.tickets.len())
    }
}

#[derive(Clone)]
enum NativePassDocumentBudget {
    Unmanaged,
    ManagedDisabled,
    Managed(DocumentBudgetHandle),
}

struct NativePassDocument {
    owner: NativeLayoutWorkingSetOwner,
    budget: NativePassDocumentBudget,
}

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
struct CandidateRank {
    // The existing exact FIFO slot gets first refusal. Remaining candidates
    // follow actual root-before-child take order, never HashMap iteration.
    ordinary_slot: bool,
    visit: usize,
}

enum NativePassAdmissionBudget {
    Unmanaged,
    Managed {
        handle: DocumentBudgetHandle,
        stamp: AdmissionStamp,
    },
}

struct NativePassAdmission {
    budget: NativePassAdmissionBudget,
    rank: CandidateRank,
}

struct NativeWorkingSetCandidate {
    entry: NativeDocumentWorkingSet,
    admission: NativePassAdmission,
}

/// Checked-out payloads remain charged by their original leases. New output
/// is temporary pass storage until CurrentScreen publication succeeds.
pub(crate) struct NativeLayoutWorkingSetPass {
    owners: HashMap<DomHandle, NativePassDocument>,
    previous: HashMap<DomHandle, NativeDocumentWorkingSet>,
    completed: BTreeMap<CandidateRank, NativeWorkingSetCandidate>,
    admissions: HashMap<DomHandle, NativePassAdmission>,
    visited: HashSet<DomHandle>,
    pending_slot: Option<DocumentSlot>,
    completed_charge: usize,
    max_estimated_retained_bytes: usize,
    document_limit: usize,
}

impl NativeLayoutWorkingSetPass {
    #[cfg(test)]
    pub(crate) fn set_retention_limit_for_test(&mut self, max_estimated_retained_bytes: usize) {
        self.max_estimated_retained_bytes = max_estimated_retained_bytes;
    }

    pub(super) fn take_document(
        &mut self,
        document: DomHandle,
        root: DomHandle,
    ) -> Option<NativeDocumentWorkingSet> {
        let input = self.owners.get(&document)?;
        if input.owner.root != root || !self.visited.insert(document) {
            return None;
        }
        let owner = input.owner;
        let budget = input.budget.clone();
        let previous = self.previous.remove(&document);
        let budget = match budget {
            NativePassDocumentBudget::Unmanaged => NativePassAdmissionBudget::Unmanaged,
            NativePassDocumentBudget::ManagedDisabled => {
                drop(previous);
                return None;
            }
            NativePassDocumentBudget::Managed(handle) => {
                debug_assert_eq!(handle.slot(), owner.budget_slot());
                let PassRetentionPermit::Enabled(stamp) = handle.begin_pass() else {
                    // In particular pressure=0 uses the existing one-shot backend,
                    // without constructing and then immediately discarding a cache.
                    drop(previous);
                    return None;
                };
                NativePassAdmissionBudget::Managed { handle, stamp }
            }
        };
        self.admissions.insert(
            document,
            NativePassAdmission {
                budget,
                rank: CandidateRank {
                    ordinary_slot: self.pending_slot != Some(owner.budget_slot()),
                    visit: self.visited.len() - 1,
                },
            },
        );
        Some(previous.unwrap_or_else(|| NativeDocumentWorkingSet {
            owner,
            working_set: LayoutWorkingSet::new(LayoutWorkingSetLimits {
                max_estimated_retained_bytes: self.document_limit,
                ..LayoutWorkingSetLimits::default()
            }),
            lease: None,
        }))
    }

    pub(super) fn complete_document(&mut self, entry: NativeDocumentWorkingSet) {
        let Some(admission) = self.admissions.remove(&entry.owner.document) else {
            // A successfully retained entry always came through one actual
            // take and its exact-slot permit. Missing metadata fails closed.
            return;
        };
        let stats = entry.working_set.stats();
        let bytes = stats.estimated_retained_bytes;
        if stats.retained_boxes == 0 || bytes == 0 || bytes > self.max_estimated_retained_bytes {
            return;
        }
        // Bounded candidate spool, independently of Browser admission. Child
        // recursion completes first; a later completed root/head can displace
        // lower-ranked children. Eviction does not create a Browser demand.
        let available_for_previous = self.max_estimated_retained_bytes - bytes;
        let higher_priority_charge = self
            .completed
            .range(..admission.rank)
            .map(|(_, candidate)| candidate.entry.working_set.stats().estimated_retained_bytes)
            .sum::<usize>();
        if higher_priority_charge > available_for_previous {
            return;
        }
        while self.completed_charge > available_for_previous {
            let Some((rank, _)) = self.completed.last_key_value() else {
                return;
            };
            if *rank < admission.rank {
                return;
            }
            let (_, victim) = self
                .completed
                .pop_last()
                .expect("selected candidate exists");
            self.completed_charge -= victim.entry.working_set.stats().estimated_retained_bytes;
            drop(victim);
        }
        self.completed_charge += bytes;
        let previous = self.completed.insert(
            admission.rank,
            NativeWorkingSetCandidate { entry, admission },
        );
        assert!(previous.is_none(), "each visit has a unique spool rank");
    }

    /// Called only after successful CurrentScreen owner/source validation and
    /// geometry publication. Dropping an uncommitted pass never activates a
    /// new demand or refunds a still-owned payload.
    pub(crate) fn finish(mut self, destination: &mut NativeLayoutWorkingSets) {
        // Hidden/rootless/unvisited old payloads are already known dead for
        // this publication. Refund their real releases before growing any
        // completed entry, rather than producing an artificial shortfall.
        if let Some(budget) = destination.budget.managed_mut() {
            budget.retain_tickets(|entry| self.visited.contains(&entry.owner.document));
        }
        self.previous.clear();
        assert!(
            destination.documents.is_empty(),
            "pass owns the checked-out cache"
        );
        for candidate in self.completed.into_values() {
            let NativeWorkingSetCandidate {
                mut entry,
                admission,
            } = candidate;
            match admission.budget {
                NativePassAdmissionBudget::Unmanaged => {
                    debug_assert!(entry.lease.is_none());
                    // Binding is installed before production passes. Also fail
                    // closed if a native harness binds while one is checked out.
                    if matches!(
                        &destination.budget,
                        NativeWorkingSetBudgetBinding::Unmanaged
                    ) {
                        destination.documents.insert(entry.owner.document, entry);
                    }
                }
                NativePassAdmissionBudget::Managed { handle, stamp } => {
                    let bytes = entry.working_set.stats().estimated_retained_bytes;
                    let previous_charge = entry.lease.as_ref().map_or(0, WorkingSetLease::bytes);
                    match handle.try_commit_retained(stamp, bytes, &mut entry.lease) {
                        Ok(()) => {
                            tracing::trace!(document = ?entry.owner.document, bytes, previous_charge,
                                "retained layout working set within shared budget");
                            destination.documents.insert(entry.owner.document, entry);
                        }
                        Err(denied) => {
                            tracing::debug!(document = ?entry.owner.document, bytes, previous_charge,
                                reason = ?denied.reason,
                                "layout working-set retention denied");
                            // The same synchronous finish owns the rejection token;
                            // never retain/replay it across passes or publish failure.
                            drop(entry);
                            if let Some(demand) = denied.demand {
                                handle.activate_demand(demand);
                            }
                        }
                    }
                }
            }
        }
    }
}
