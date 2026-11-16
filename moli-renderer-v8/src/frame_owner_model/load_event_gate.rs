use std::collections::BTreeMap;

use super::records::{DocumentLoadDelayReason, DocumentLoadDelayTokenId};

/// Document-owned ledger for work that directly delays the Window `load`
/// transition.
///
/// The enclosing lifecycle owns whether these identities still have blocking
/// authority; invalidation retains them only for authenticated late cleanup.
///
/// This is state, not a task source: it stores no executable payload, performs
/// no wakeup, and knows nothing about script scheduling. Producers acquire an
/// exact token before publishing work; the terminal owner releases that same
/// token and uses the returned transition to reconsider lifecycle.
#[derive(Debug, Default)]
pub(super) struct DocumentLoadGate {
    entries: BTreeMap<DocumentLoadDelayTokenId, DocumentLoadDelayReason>,
}

/// Settlement of an exact Document-owned load delay, independent of the
/// resource that acquired it and of the phase it used to block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DocumentLoadDelaySettlement {
    /// The owner, token or reason does not match, or an active delay was
    /// already released.
    #[default]
    NotOwned,
    /// Admission occurred after the Document completed and acquired no delay.
    NoDelayRequired,
    /// This completion removed an active delay.
    Released,
    /// The Document already invalidated this exact delay. Later completions
    /// are idempotent no-ops and cannot unblock or restart lifecycle work.
    Invalidated,
}

impl DocumentLoadDelaySettlement {
    pub(crate) const fn settled(self) -> bool {
        !matches!(self, Self::NotOwned)
    }

    pub(crate) const fn released(self) -> bool {
        matches!(self, Self::Released)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DocumentLoadGateRelease {
    NotOwned,
    Invalidated,
    StillBlocked,
    BecameUnblocked,
}

impl DocumentLoadGateRelease {
    pub(super) const fn settlement(self) -> DocumentLoadDelaySettlement {
        match self {
            Self::NotOwned => DocumentLoadDelaySettlement::NotOwned,
            Self::Invalidated => DocumentLoadDelaySettlement::Invalidated,
            Self::StillBlocked | Self::BecameUnblocked => DocumentLoadDelaySettlement::Released,
        }
    }
}

impl DocumentLoadGate {
    pub(super) fn acquire(
        &mut self,
        token: DocumentLoadDelayTokenId,
        reason: DocumentLoadDelayReason,
    ) -> bool {
        if !reason.blocks_window_load_directly() || self.entries.contains_key(&token) {
            return false;
        }
        self.entries.insert(token, reason);
        true
    }

    pub(super) fn release(
        &mut self,
        token: DocumentLoadDelayTokenId,
        expected_reason: DocumentLoadDelayReason,
    ) -> DocumentLoadGateRelease {
        if self.entries.get(&token) != Some(&expected_reason) {
            return DocumentLoadGateRelease::NotOwned;
        }
        self.entries.remove(&token);
        if self.entries.is_empty() {
            DocumentLoadGateRelease::BecameUnblocked
        } else {
            DocumentLoadGateRelease::StillBlocked
        }
    }

    pub(super) fn owns(
        &self,
        token: DocumentLoadDelayTokenId,
        reason: DocumentLoadDelayReason,
    ) -> bool {
        self.entries.get(&token) == Some(&reason)
    }

    pub(super) fn owns_any(&self, token: DocumentLoadDelayTokenId) -> bool {
        self.entries.contains_key(&token)
    }

    pub(super) fn has_reason(&self, reason: DocumentLoadDelayReason) -> bool {
        self.entries.values().any(|candidate| *candidate == reason)
    }

    pub(super) fn is_blocked(&self) -> bool {
        !self.entries.is_empty()
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(super) fn release_all_document_script_delays(&mut self) -> usize {
        let before = self.entries.len();
        self.entries
            .retain(|_, reason| !reason.is_document_script());
        before - self.entries.len()
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_reports_only_the_last_exact_token_as_unblocking() {
        let mut gate = DocumentLoadGate::default();
        let first = DocumentLoadDelayTokenId(1);
        let second = DocumentLoadDelayTokenId(2);

        assert!(gate.acquire(first, DocumentLoadDelayReason::AsyncClassicScript));
        assert!(gate.acquire(second, DocumentLoadDelayReason::AsyncModuleScript));
        assert_eq!(
            gate.release(first, DocumentLoadDelayReason::AsyncClassicScript),
            DocumentLoadGateRelease::StillBlocked
        );
        assert_eq!(
            gate.release(first, DocumentLoadDelayReason::AsyncClassicScript),
            DocumentLoadGateRelease::NotOwned
        );
        assert_eq!(
            gate.release(second, DocumentLoadDelayReason::AsyncModuleScript),
            DocumentLoadGateRelease::BecameUnblocked
        );
    }

    #[test]
    fn parser_deferred_tokens_are_not_window_load_gate_entries() {
        let mut gate = DocumentLoadGate::default();
        assert!(!gate.acquire(
            DocumentLoadDelayTokenId(1),
            DocumentLoadDelayReason::ParserDeferredScript,
        ));
        assert!(!gate.is_blocked());
    }
}
