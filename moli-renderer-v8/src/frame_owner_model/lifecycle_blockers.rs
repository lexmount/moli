use std::collections::BTreeSet;

use super::load_event_gate::{
    DocumentLoadDelaySettlement, DocumentLoadGate, DocumentLoadGateRelease,
};
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
    state: DocumentLoadDelayState,
}

/// Blocking authority and token identity have different lifetimes. A current
/// Document can invalidate all delays while their resource tasks still exist.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum DocumentLoadDelayState {
    #[default]
    Open,
    Invalidated,
    Retired,
}

impl DocumentLifecycleBlockers {
    pub(super) fn acquire(
        &mut self,
        token: DocumentLoadDelayTokenId,
        reason: DocumentLoadDelayReason,
    ) -> bool {
        if self.state != DocumentLoadDelayState::Open || self.owns_any(token) {
            return false;
        }
        if reason == DocumentLoadDelayReason::ParserDeferredScript {
            self.parser_deferred_scripts.insert(token)
        } else {
            self.window_load.acquire(token, reason)
        }
    }

    pub(super) fn settle(
        &mut self,
        token: DocumentLoadDelayTokenId,
        reason: DocumentLoadDelayReason,
    ) -> DocumentLoadDelaySettlement {
        match self.state {
            DocumentLoadDelayState::Invalidated => {
                return if self.owns_exact_token(token, reason) {
                    DocumentLoadDelaySettlement::Invalidated
                } else {
                    DocumentLoadDelaySettlement::NotOwned
                };
            }
            DocumentLoadDelayState::Retired => return DocumentLoadDelaySettlement::NotOwned,
            DocumentLoadDelayState::Open => {}
        }
        if reason == DocumentLoadDelayReason::ParserDeferredScript {
            if self.parser_deferred_scripts.remove(&token) {
                DocumentLoadDelaySettlement::Released
            } else {
                DocumentLoadDelaySettlement::NotOwned
            }
        } else {
            self.window_load.release(token, reason).settlement()
        }
    }

    pub(super) fn release_window_load(
        &mut self,
        token: DocumentLoadDelayTokenId,
        reason: DocumentLoadDelayReason,
    ) -> DocumentLoadGateRelease {
        debug_assert!(reason.blocks_window_load_directly());
        match self.state {
            DocumentLoadDelayState::Invalidated => {
                return if self.owns_exact_token(token, reason) {
                    DocumentLoadGateRelease::Invalidated
                } else {
                    DocumentLoadGateRelease::NotOwned
                };
            }
            DocumentLoadDelayState::Retired => return DocumentLoadGateRelease::NotOwned,
            DocumentLoadDelayState::Open => {}
        }
        self.window_load.release(token, reason)
    }

    pub(super) fn owns(
        &self,
        token: DocumentLoadDelayTokenId,
        reason: DocumentLoadDelayReason,
    ) -> bool {
        self.state == DocumentLoadDelayState::Open && self.owns_exact_token(token, reason)
    }

    fn owns_exact_token(
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
        if self.state != DocumentLoadDelayState::Open {
            return false;
        }
        if reason == DocumentLoadDelayReason::ParserDeferredScript {
            !self.parser_deferred_scripts.is_empty()
        } else {
            self.window_load.has_reason(reason)
        }
    }

    pub(super) fn blocks_domcontentloaded(&self) -> bool {
        self.state == DocumentLoadDelayState::Open && !self.parser_deferred_scripts.is_empty()
    }

    pub(super) fn blocks_window_load_directly(&self) -> bool {
        self.state == DocumentLoadDelayState::Open && self.window_load.is_blocked()
    }

    pub(super) fn release_all_document_script_delays(&mut self) -> usize {
        if self.state != DocumentLoadDelayState::Open {
            return 0;
        }
        let parser_deferred = self.parser_deferred_scripts.len();
        self.parser_deferred_scripts.clear();
        parser_deferred + self.window_load.release_all_document_script_delays()
    }

    pub(super) fn clear_for_retirement(&mut self) {
        self.state = DocumentLoadDelayState::Retired;
        self.parser_deferred_scripts.clear();
        self.window_load.clear();
    }

    /// End all outstanding delays while their Document remains current.
    /// Resource tasks may still finish, but no longer own a gate transition.
    pub(super) fn invalidate_all(&mut self) {
        if self.state != DocumentLoadDelayState::Retired {
            self.state = DocumentLoadDelayState::Invalidated;
        }
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        if self.state == DocumentLoadDelayState::Open {
            self.parser_deferred_scripts.len() + self.window_load.len()
        } else {
            0
        }
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

        assert_eq!(
            blockers.settle(parser, DocumentLoadDelayReason::ParserDeferredScript),
            DocumentLoadDelaySettlement::Released
        );
        assert!(!blockers.blocks_domcontentloaded());
        assert_eq!(
            blockers.release_window_load(window, DocumentLoadDelayReason::AsyncModuleScript),
            DocumentLoadGateRelease::BecameUnblocked
        );
    }

    #[test]
    fn invalidation_settles_every_resource_kind_without_a_new_gate_transition() {
        let reasons = [
            DocumentLoadDelayReason::AsyncClassicScript,
            DocumentLoadDelayReason::BlockingStylesheet,
            DocumentLoadDelayReason::StylesheetSubresource,
            DocumentLoadDelayReason::ParserDeferredScript,
            DocumentLoadDelayReason::AsyncModuleScript,
            DocumentLoadDelayReason::Image,
            DocumentLoadDelayReason::Media,
            DocumentLoadDelayReason::StyleLoadEvent,
            DocumentLoadDelayReason::Navigation,
        ];
        let mut blockers = DocumentLifecycleBlockers::default();
        for (index, reason) in reasons.iter().copied().enumerate() {
            assert!(blockers.acquire(DocumentLoadDelayTokenId(index as u64), reason));
        }
        assert!(blockers.blocks_domcontentloaded());
        assert!(blockers.blocks_window_load_directly());

        blockers.invalidate_all();
        blockers.invalidate_all();
        assert!(!blockers.blocks_domcontentloaded());
        assert!(!blockers.blocks_window_load_directly());
        assert_eq!(blockers.len(), 0);
        for (index, reason) in reasons.iter().copied().enumerate() {
            let token = DocumentLoadDelayTokenId(index as u64);
            assert!(!blockers.owns(token, reason));
            assert!(!blockers.acquire(token, reason));
            for _ in 0..2 {
                assert_eq!(
                    blockers.settle(token, reason),
                    DocumentLoadDelaySettlement::Invalidated
                );
            }
            if reason.blocks_window_load_directly() {
                assert_eq!(
                    blockers.release_window_load(token, reason),
                    DocumentLoadGateRelease::Invalidated
                );
            }
        }
        assert!(!blockers.blocks_domcontentloaded());
        assert!(!blockers.blocks_window_load_directly());

        blockers.clear_for_retirement();
        for (index, reason) in reasons.iter().copied().enumerate() {
            assert!(!blockers.owns_any(DocumentLoadDelayTokenId(index as u64)));
            assert_eq!(
                blockers.settle(DocumentLoadDelayTokenId(index as u64), reason),
                DocumentLoadDelaySettlement::NotOwned
            );
        }
    }

    #[test]
    fn invalidation_does_not_authorize_unknown_wrong_reason_or_already_released_tokens() {
        let mut blockers = DocumentLifecycleBlockers::default();
        let released = DocumentLoadDelayTokenId(1);
        let outstanding = DocumentLoadDelayTokenId(2);
        assert!(blockers.acquire(released, DocumentLoadDelayReason::StyleLoadEvent));
        assert!(blockers.acquire(outstanding, DocumentLoadDelayReason::Image));
        assert_eq!(
            blockers.settle(released, DocumentLoadDelayReason::StyleLoadEvent),
            DocumentLoadDelaySettlement::Released
        );
        blockers.invalidate_all();
        for token in [released, outstanding, DocumentLoadDelayTokenId(3)] {
            assert_eq!(
                blockers.settle(token, DocumentLoadDelayReason::StyleLoadEvent),
                DocumentLoadDelaySettlement::NotOwned
            );
        }
        assert_eq!(
            blockers.release_window_load(outstanding, DocumentLoadDelayReason::StyleLoadEvent),
            DocumentLoadGateRelease::NotOwned
        );
        assert_eq!(
            blockers.settle(outstanding, DocumentLoadDelayReason::Image),
            DocumentLoadDelaySettlement::Invalidated
        );
    }
}
