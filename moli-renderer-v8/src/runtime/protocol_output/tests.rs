use super::item::RendererOutputResolutionError;
use super::*;
use crate::runtime::{
    PageId, RendererDocumentLifecycleEvent, RendererDocumentLifecycleEventKind,
    RendererDocumentToken, RendererFrameToken, RendererLifecycleEpoch,
    RendererLifecycleStartReason, RendererRuntimeInspectorMessage,
    RendererRuntimeInspectorMessageBatch,
};
use moli_page_types::{DevToolsSessionKey, RendererDevToolsAgentToken};
use serde_json::json;

fn lifecycle_event(sequence: u64) -> RendererDocumentLifecycleEvent {
    let page_id = PageId::new_for_testing(7);
    RendererDocumentLifecycleEvent {
        frame: RendererFrameToken { page_id },
        document: RendererDocumentToken::new_for_testing(page_id, 3),
        epoch: RendererLifecycleEpoch(2),
        sequence,
        timestamp_micros: sequence,
        kind: RendererDocumentLifecycleEventKind::Started {
            reason: RendererLifecycleStartReason::InitialDocument,
        },
    }
}

fn lifecycle_record(sequence: u64) -> PendingRendererOutputRecord {
    PendingRendererOutputRecord::observation(
        None,
        RendererProtocolObservation::DocumentLifecycle(lifecycle_event(sequence)),
    )
}

#[test]
fn default_context_record_requires_an_origin_bound_at_context_creation() {
    let unresolved_message = RendererRuntimeInspectorMessage::from_v8_inspector_message(json!({
        "method": "Runtime.executionContextCreated",
        "params": {
            "context": {
                "id": 41,
                "uniqueId": "realm-41",
                "origin": "",
                "name": "",
                "auxData": {
                    "isDefault": true,
                    "type": "default"
                }
            }
        }
    }));
    let unresolved_record = PendingRendererOutputRecord::observation(
        None,
        RendererProtocolObservation::RuntimeInspector(RendererRuntimeInspectorMessageBatch::new(
            RendererDevToolsAgentToken::allocate(),
            DevToolsSessionKey::Primary,
            vec![unresolved_message],
        )),
    );

    let error = unresolved_record
        .resolve()
        .expect_err("an unresolved origin must not enter a publication");
    assert_eq!(error, RendererOutputResolutionError::RuntimeInspector);

    let resolved_message = RendererRuntimeInspectorMessage::from_v8_inspector_message(json!({
        "method": "Runtime.executionContextCreated",
        "params": {
            "context": {
                "id": 41,
                "uniqueId": "realm-41",
                "origin": "https://example.test",
                "name": "",
                "auxData": {
                    "isDefault": true,
                    "type": "default"
                }
            }
        }
    }));
    let resolved_record = PendingRendererOutputRecord::observation(
        None,
        RendererProtocolObservation::RuntimeInspector(RendererRuntimeInspectorMessageBatch::new(
            RendererDevToolsAgentToken::allocate(),
            DevToolsSessionKey::Primary,
            vec![resolved_message],
        )),
    );
    assert!(
        resolved_record.resolve().is_ok(),
        "a creation-time origin must cross into the resolved publication type"
    );
}

#[test]
fn journal_preserves_append_order_and_only_sequences_non_empty_publications() {
    let page_id = PageId::new_for_testing(7);
    let journal = RendererTurnOutputJournal::new(
        RendererOutputStreamIdentity::new_page_for_protocol_test(page_id),
    );

    assert!(journal.settle().is_none());
    journal.append(lifecycle_record(11));
    journal.append(lifecycle_record(12));
    let first = journal.settle().expect("non-empty turn should publish");
    assert_eq!(first.cursor().sequence(), 1);
    let sequences = first
        .records()
        .iter()
        .map(|record| match record.item() {
            RendererOutputItem::Observation(RendererProtocolObservation::DocumentLifecycle(
                event,
            )) => event.sequence,
            item => panic!("unexpected test output: {item:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(sequences, vec![11, 12]);

    assert!(journal.settle().is_none());
    journal.append(lifecycle_record(13));
    assert_eq!(
        journal
            .settle()
            .expect("second non-empty turn should publish")
            .cursor()
            .sequence(),
        2
    );
}

#[test]
fn pending_resolution_releases_journal_before_later_producer_append() {
    let journal = RendererTurnOutputJournal::new(
        RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(8)),
    );
    journal.append(lifecycle_record(1));
    let pending = journal
        .take_pending_for_resolution()
        .expect("first record should reserve a publication sequence");

    journal.append(lifecycle_record(2));
    assert_eq!(pending.finish().cursor().sequence(), 1);
    assert_eq!(
        journal
            .settle()
            .expect("reentrant producer output must remain in the journal")
            .cursor()
            .sequence(),
        2
    );
}

#[test]
fn retirement_publishes_the_final_batch_before_its_frozen_close_boundary() {
    let page_id = PageId::new_for_testing(9);
    let (transport, mut receiver) = super::renderer_output_transport_channel();
    let journal = RendererTurnOutputJournal::new_with_transport(
        RendererOutputStreamIdentity::new_page_for_protocol_test(page_id),
        transport,
    );
    let stream = journal.stream();
    journal.append(lifecycle_record(1));
    journal.retire(RendererOutputStreamCloseReason::ResidenceRetired);

    assert_eq!(
        receiver.try_recv().expect("stream open"),
        RendererOutputTransportMessage::StreamControl(RendererOutputStreamControl::Opened {
            stream
        })
    );
    let RendererOutputTransportMessage::Publication(publication) =
        receiver.try_recv().expect("final publication")
    else {
        panic!("retirement must publish its final concrete batch before closing");
    };
    assert_eq!(publication.cursor().sequence(), 1);
    assert_eq!(
        receiver.try_recv().expect("stream close"),
        RendererOutputTransportMessage::StreamControl(RendererOutputStreamControl::Closed {
            stream,
            last_published_sequence: std::num::NonZeroU64::new(publication.cursor().sequence()),
            reason: RendererOutputStreamCloseReason::ResidenceRetired,
        })
    );
    assert!(receiver.try_recv().is_err());
}
