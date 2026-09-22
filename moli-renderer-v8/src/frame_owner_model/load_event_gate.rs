use std::collections::BTreeMap;

use super::records::{DocumentLoadDelayReason, DocumentLoadDelayTokenId};

/// Document-owned ledger for work that directly delays the Window `load`
/// transition.
///
/// This is state, not a task source: it stores no executable payload, performs
/// no wakeup, and knows nothing about script scheduling. Producers acquire an
/// exact token before publishing work; the terminal owner releases that same
/// token and uses the returned transition to reconsider lifecycle.
#[derive(Debug, Default)]
pub(super) struct DocumentLoadGate {
    active: BTreeMap<DocumentLoadDelayTokenId, DocumentLoadDelayReason>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DocumentLoadGateRelease {
    NotOwned,
    StillBlocked,
    BecameUnblocked,
}

impl DocumentLoadGateRelease {
    pub(super) const fn released(self) -> bool {
        !matches!(self, Self::NotOwned)
    }
}

impl DocumentLoadGate {
    pub(super) fn acquire(
        &mut self,
        token: DocumentLoadDelayTokenId,
        reason: DocumentLoadDelayReason,
    ) -> bool {
        if !reason.blocks_window_load_directly() || self.active.contains_key(&token) {
            return false;
        }
        self.active.insert(token, reason);
        true
    }

    pub(super) fn release(
        &mut self,
        token: DocumentLoadDelayTokenId,
        expected_reason: DocumentLoadDelayReason,
    ) -> DocumentLoadGateRelease {
        if self.active.get(&token) != Some(&expected_reason) {
            return DocumentLoadGateRelease::NotOwned;
        }
        self.active.remove(&token);
        if self.active.is_empty() {
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
        self.active.get(&token) == Some(&reason)
    }

    pub(super) fn owns_any(&self, token: DocumentLoadDelayTokenId) -> bool {
        self.active.contains_key(&token)
    }

    pub(super) fn has_reason(&self, reason: DocumentLoadDelayReason) -> bool {
        self.active.values().any(|candidate| *candidate == reason)
    }

    pub(super) fn is_blocked(&self) -> bool {
        !self.active.is_empty()
    }

    pub(super) fn clear(&mut self) {
        self.active.clear();
    }

    pub(super) fn release_all_document_script_delays(&mut self) -> usize {
        let before = self.active.len();
        self.active.retain(|_, reason| !reason.is_document_script());
        before - self.active.len()
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.active.len()
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
