use std::collections::BTreeSet;

use super::load_event_gate::{DocumentLoadGate, DocumentLoadGateRelease};
use super::records::{DocumentLoadDelayReason, DocumentLoadDelayTokenId};

/// All phase-specific blockers owned by one exact Document lifecycle.
///
/// The aggregate owns the shared token namespace and retirement boundary, but
/// deliberately keeps parser/DCL blockers separate from direct Window-load
/// blockers so each phase retains an unambiguous readiness transition.
#[derive(Debug, Default)]
pub(super) struct DocumentLifecycleBlockers {
    parser_deferred_scripts: BTreeSet<DocumentLoadDelayTokenId>,
    window_load: DocumentLoadGate,
}

impl DocumentLifecycleBlockers {
    pub(super) fn acquire(
        &mut self,
        token: DocumentLoadDelayTokenId,
        reason: DocumentLoadDelayReason,
    ) -> bool {
        if self.owns_any(token) {
            return false;
        }
        if reason == DocumentLoadDelayReason::ParserDeferredScript {
            self.parser_deferred_scripts.insert(token)
        } else {
            self.window_load.acquire(token, reason)
        }
    }

    pub(super) fn release(
        &mut self,
        token: DocumentLoadDelayTokenId,
        reason: DocumentLoadDelayReason,
    ) -> bool {
        if reason == DocumentLoadDelayReason::ParserDeferredScript {
            self.parser_deferred_scripts.remove(&token)
        } else {
            self.window_load.release(token, reason).released()
        }
    }

    pub(super) fn release_window_load(
        &mut self,
        token: DocumentLoadDelayTokenId,
        reason: DocumentLoadDelayReason,
    ) -> DocumentLoadGateRelease {
        debug_assert!(reason.blocks_window_load_directly());
        self.window_load.release(token, reason)
    }

    pub(super) fn owns(
        &self,
        token: DocumentLoadDelayTokenId,
        reason: DocumentLoadDelayReason,
    ) -> bool {
        if reason == DocumentLoadDelayReason::ParserDeferredScript {
            self.parser_deferred_scripts.contains(&token)
        } else {
            self.window_load.owns(token, reason)
        }
    }

    pub(super) fn owns_any(&self, token: DocumentLoadDelayTokenId) -> bool {
        self.parser_deferred_scripts.contains(&token) || self.window_load.owns_any(token)
    }

    pub(super) fn has_reason(&self, reason: DocumentLoadDelayReason) -> bool {
        if reason == DocumentLoadDelayReason::ParserDeferredScript {
            !self.parser_deferred_scripts.is_empty()
        } else {
            self.window_load.has_reason(reason)
        }
    }

    pub(super) fn blocks_domcontentloaded(&self) -> bool {
        !self.parser_deferred_scripts.is_empty()
    }

    pub(super) fn blocks_window_load_directly(&self) -> bool {
        self.window_load.is_blocked()
    }

    pub(super) fn release_all_document_script_delays(&mut self) -> usize {
        let parser_deferred = self.parser_deferred_scripts.len();
        self.parser_deferred_scripts.clear();
        parser_deferred + self.window_load.release_all_document_script_delays()
    }

    pub(super) fn clear_for_retirement(&mut self) {
        self.parser_deferred_scripts.clear();
        self.window_load.clear();
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.parser_deferred_scripts.len() + self.window_load.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_blockers_share_tokens_without_merging_phase_semantics() {
        let mut blockers = DocumentLifecycleBlockers::default();
        let parser = DocumentLoadDelayTokenId(1);
        let window = DocumentLoadDelayTokenId(2);

        assert!(blockers.acquire(parser, DocumentLoadDelayReason::ParserDeferredScript));
        assert!(blockers.acquire(window, DocumentLoadDelayReason::AsyncModuleScript));
        assert!(!blockers.acquire(parser, DocumentLoadDelayReason::AsyncClassicScript));
        assert!(blockers.blocks_domcontentloaded());
        assert!(blockers.blocks_window_load_directly());

        assert!(blockers.release(parser, DocumentLoadDelayReason::ParserDeferredScript));
        assert!(!blockers.blocks_domcontentloaded());
        assert_eq!(
            blockers.release_window_load(window, DocumentLoadDelayReason::AsyncModuleScript),
            DocumentLoadGateRelease::BecameUnblocked
        );
    }
}
