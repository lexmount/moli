use super::*;
use crate::capture_resources::{CaptureResourceResult, PrintCaptureJob};

fn print_request() -> RendererCaptureScreenshotRequest {
    RendererCaptureScreenshotRequest {
        purpose: RendererScreenshotPurpose::Print {
            print_background: true,
        },
        ..RendererCaptureScreenshotRequest::viewport_png()
    }
}

fn page_view(vm_creation_id: u64, view_generation: u64) -> RendererPageView {
    let url = Url::parse("https://print-admission.test/").unwrap();
    RendererPageView {
        page_id: PageId::new_for_testing(1),
        vm_creation_id,
        view_generation,
        page_state: Arc::new(RendererPageState {
            snapshot_revision: RendererPageStateRevision::new(vm_creation_id, 1),
            requested_url: url.clone(),
            navigation_initiator_url: None,
            navigation_redirected: false,
            navigation_redirect_count: 0,
            navigation_redirect_chain: Vec::new(),
            final_url: url,
            document_title: String::new(),
            document_activity: Default::default(),
            status: 200,
            headers: Vec::new(),
            script_execution: Arc::new(ScriptExecutionReport::default()),
            idle_override: None,
            service_worker_client_id: 0,
            document_isolate_identity: 0,
            dedicated_worker_running_worker_isolate_count: 0,
            performance_metric_snapshot: RendererPerformanceMetricSnapshot::default(),
        }),
    }
}

#[test]
fn print_admission_survives_replacement_view_until_its_job_is_dropped() {
    let initial = page_view(11, 0);
    let (cancel_tx, _cancel_rx) = renderer_page_context_cancel_channel();
    let slot = RendererPageSlotHandle::new(
        std::sync::Weak::new(),
        RendererPageEntry::active(
            initial.page_id,
            initial.vm_creation_id,
            initial.view_generation,
            0,
            initial.page_state,
        ),
        cancel_tx,
        Default::default(),
    );
    let same_slot = slot.clone();
    let first = slot.begin_print_capture(print_request()).unwrap();
    let old_endpoint = first.endpoint();
    let old_mailbox = Arc::downgrade(&first.mailbox);
    slot.validate_print_capture(&first).unwrap();

    // Exercise the real stable-slot refresh with another PageVm's identity.
    // This metadata test does not simulate the navigation/network pipeline.
    slot.refresh(page_view(12, 1)).unwrap();
    assert_eq!(same_slot.entry().vm_creation_id(), 12);
    same_slot.validate_print_capture(&first).unwrap();
    assert!(same_slot.begin_print_capture(print_request()).is_err());

    let unadmitted = PrintCaptureJob::new(print_request()).unwrap();
    assert!(same_slot.validate_print_capture(&unadmitted).is_err());
    drop(unadmitted);
    same_slot.validate_print_capture(&first).unwrap();
    assert!(same_slot.begin_print_capture(print_request()).is_err());

    drop(first);
    assert!(old_mailbox.upgrade().is_none());
    assert!(!old_endpoint.is_active());
    let second = same_slot.begin_print_capture(print_request()).unwrap();
    slot.validate_print_capture(&second).unwrap();
    assert!(!old_endpoint.deliver(
        0,
        CaptureResourceResult::Failed("late old producer".to_owned()),
    ));
    slot.validate_print_capture(&second).unwrap();

    slot.remove();
    assert!(same_slot.validate_print_capture(&second).is_err());
    drop(second);
    assert!(same_slot.begin_print_capture(print_request()).is_err());
}
