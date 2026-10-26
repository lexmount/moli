use super::*;
use moli_fetch::FetchCancelHandle;

#[test]
fn native_commit_visibility_requires_the_admitted_fact_and_current_attachment() {
    use moli_core::page::{RendererAgentAttachmentId, RendererMainDocumentCommit};
    let mut slot = TargetPageSlot::default();
    let token = slot.start_document_navigation("target".into(), "loader".into());
    let attachment = RendererAgentAttachmentId::allocate();
    let fact = std::sync::Arc::new(RendererMainDocumentCommit {
        frame_id: token.target_id.clone(),
        loader_id: token.loader_id.clone(),
        url: "https://example.test/committed".into(),
        unreachable_url: None,
        security_origin: "https://example.test".into(),
        secure_context_type: "Secure".into(),
        document_referrer: None,
        timestamp: 1.0,
        session_history_position: None,
        browsing_context_group: None,
    });
    assert!(slot.register_renderer_document_commit(&token, attachment, fact.clone()));
    assert_eq!(slot.frame_tree_document_loader_id(), None);
    assert_eq!(
        slot.observe_prepared_renderer_document_commit(
            Some(RendererAgentAttachmentId::allocate()),
            &fact,
        ),
        Some(false)
    );
    let mut different_fact = fact.as_ref().clone();
    different_fact.timestamp += 1.0;
    assert_eq!(
        slot.observe_prepared_renderer_document_commit(Some(attachment), &different_fact),
        Some(false)
    );
    assert_eq!(slot.frame_tree_document_loader_id(), None);
    assert_eq!(
        slot.observe_prepared_renderer_document_commit(Some(attachment), &fact),
        Some(true)
    );
    assert_eq!(slot.frame_tree_document_loader_id(), Some("loader"));
    assert!(
        slot.accepts_pending_document_navigation_event(&token),
        "native visibility must not finish the still-unadopted navigation"
    );

    let replacement = slot.start_document_navigation("target".into(), "loader".into());
    assert!(!slot.register_renderer_document_commit(&token, attachment, fact.clone()));
    assert_eq!(
        slot.observe_prepared_renderer_document_commit(Some(attachment), &fact),
        None
    );
    assert_eq!(slot.frame_tree_document_loader_id(), None);
    assert!(slot.accepts_pending_document_navigation_event(&replacement));
}

#[test]
fn background_phase_tracks_result_receipt_independently_of_document_commit() {
    for commit_first in [false, true] {
        let mut slot = TargetPageSlot::default();
        let token = slot.start_document_navigation("target".into(), "loader".into());
        assert_eq!(
            slot.pending_navigation_request
                .as_ref()
                .unwrap()
                .background_work,
            BackgroundWorkState::NotStarted
        );
        assert!(!slot.settle_background_navigation_completion(&token));
        slot.claim_background_navigation_completion(&token, None)
            .unwrap();
        if commit_first {
            assert!(slot.commit_pending_document_navigation_if_matches(&token));
        }
        let request = slot.pending_navigation_request.as_ref().unwrap();
        assert_eq!(request.background_work, BackgroundWorkState::Running);
        assert_eq!(request.committed, commit_first);
        assert!(slot.settle_background_navigation_completion(&token));
        assert!(!slot.settle_background_navigation_completion(&token));
        if !commit_first {
            let request = slot.pending_navigation_request.as_ref().unwrap();
            assert_eq!(request.background_work, BackgroundWorkState::ResultReceived);
            assert!(!request.committed);
            assert!(request.cancellation_handles.is_empty());
            assert!(slot.commit_pending_document_navigation_if_matches(&token));
        }
        assert!(slot.pending_navigation_request.is_none());
    }
}

#[test]
fn claim_requires_the_exact_live_request() {
    let mut slot = TargetPageSlot::default();
    let old = slot.start_document_navigation("target".into(), "loader".into());
    let current = slot.start_document_navigation("target".into(), "loader".into());
    for rejected in [
        old,
        DocumentNavigationToken {
            target_id: "other target".into(),
            ..current.clone()
        },
        DocumentNavigationToken {
            loader_id: "other loader".into(),
            ..current.clone()
        },
    ] {
        assert_eq!(
            slot.claim_background_navigation_completion(&rejected, None)
                .unwrap_err(),
            BackgroundNavigationClaimError::Stale
        );
        assert!(!slot.has_inflight_background_navigation());
    }
    let cancellation = slot
        .claim_background_navigation_completion(&current, None)
        .unwrap();
    slot.clear_document_navigation_state();
    assert!(cancellation.is_cancelled());
    assert_eq!(
        slot.claim_background_navigation_completion(&current, None)
            .unwrap_err(),
        BackgroundNavigationClaimError::Stale
    );
}

#[test]
fn duplicate_claim_does_not_take_the_rejected_transports_cancellation() {
    let mut slot = TargetPageSlot::default();
    let token = slot.start_document_navigation("target".into(), "loader".into());
    let response = FetchCancelHandle::new();
    let duplicate = FetchCancelHandle::new();
    let cancellation = slot
        .claim_background_navigation_completion(&token, Some(response.clone()))
        .unwrap();
    assert_eq!(
        slot.claim_background_navigation_completion(&token, Some(duplicate.clone()))
            .unwrap_err(),
        BackgroundNavigationClaimError::AlreadyClaimed
    );
    assert!(!cancellation.is_cancelled());
    assert!(!response.is_cancelled());
    slot.clear_document_navigation_state();
    assert!(cancellation.is_cancelled());
    assert!(response.is_cancelled());
    assert!(
        !duplicate.is_cancelled(),
        "the rejected response remains caller-owned"
    );
}

#[test]
fn committed_request_is_stale_even_while_background_completion_is_pending() {
    for background in [false, true] {
        let mut slot = TargetPageSlot::default();
        let token = slot.start_document_navigation("target".into(), "loader".into());
        let cancellation = slot
            .document_navigation_cancellation_handle(&token)
            .unwrap();
        if background {
            slot.claim_background_navigation_completion(&token, None)
                .unwrap();
        }
        assert!(slot.commit_pending_document_navigation_if_matches(&token));
        assert_eq!(
            slot.claim_background_navigation_completion(&token, None)
                .unwrap_err(),
            BackgroundNavigationClaimError::Stale
        );
        assert_eq!(slot.has_inflight_background_navigation(), background);
        assert_eq!(
            slot.settle_background_navigation_completion(&token),
            background
        );
        assert!(!slot.has_inflight_background_navigation());
        assert!(!cancellation.is_cancelled());
    }
}

#[test]
fn settled_request_cannot_be_claimed_again_before_its_commit_is_drained() {
    let mut slot = TargetPageSlot::default();
    let token = slot.start_document_navigation("target".into(), "loader".into());
    let cancellation = slot
        .claim_background_navigation_completion(&token, None)
        .unwrap();
    assert!(slot.settle_background_navigation_completion(&token));
    assert_eq!(
        slot.claim_background_navigation_completion(&token, None)
            .unwrap_err(),
        BackgroundNavigationClaimError::Stale
    );
    assert!(slot.commit_pending_document_navigation_if_matches(&token));
    assert!(!cancellation.is_cancelled());
}
