use super::*;

fn job() -> PrintCaptureJob {
    PrintCaptureJob::new(RendererCaptureScreenshotRequest {
        purpose: RendererScreenshotPurpose::Print {
            print_background: true,
        },
        ..RendererCaptureScreenshotRequest::viewport_png()
    })
    .unwrap()
}

#[test]
fn encoded_entry_is_charged_once_and_cumulative_excess_fails_the_capture() {
    let job = job();
    let endpoint = job.endpoint();
    endpoint
        .accept_encoded(
            0,
            CaptureResourceKind::Image,
            MAX_CAPTURE_IMAGE_ENCODED_BYTES,
        )
        .unwrap();
    endpoint
        .accept_encoded(
            0,
            CaptureResourceKind::Image,
            MAX_CAPTURE_IMAGE_ENCODED_BYTES,
        )
        .unwrap();
    // Repeating one accepted terminal must not consume a second entry's room.
    endpoint
        .accept_encoded(
            1,
            CaptureResourceKind::Image,
            MAX_CAPTURE_IMAGE_ENCODED_BYTES,
        )
        .unwrap();
    assert!(
        endpoint
            .accept_encoded(2, CaptureResourceKind::Font, 1)
            .is_err()
    );
    assert!(job.take_completions().is_err());
}

#[test]
fn a_single_oversized_resource_is_a_capture_error() {
    for (kind, limit) in [
        (CaptureResourceKind::Font, MAX_CAPTURE_FONT_ENCODED_BYTES),
        (CaptureResourceKind::Image, MAX_CAPTURE_IMAGE_ENCODED_BYTES),
    ] {
        let job = job();
        let endpoint = job.endpoint();
        assert!(endpoint.accept_encoded(0, kind, limit + 1).is_err());
        // The owner sees a job-level failure, not an ordinary missing resource.
        assert!(job.take_completions().is_err());
    }
}

#[test]
fn weak_producers_do_not_keep_cancelled_captures_or_queued_bytes_alive() {
    for explicitly_revoke in [false, true] {
        let mut job = job();
        let endpoint = job.endpoint();
        let producer = endpoint.clone();
        let mailbox = Arc::downgrade(&job.mailbox);
        let bytes: Arc<[u8]> = vec![1, 2, 3].into();
        let queued_bytes = Arc::downgrade(&bytes);
        assert!(endpoint.deliver(0, CaptureResourceResult::FontBytes(bytes)));
        assert!(queued_bytes.upgrade().is_some());

        if explicitly_revoke {
            job.revoke();
            assert!(queued_bytes.upgrade().is_none());
            assert!(!endpoint.is_active());
            assert!(
                endpoint
                    .accept_encoded(1, CaptureResourceKind::Font, 1)
                    .is_err()
            );
            let late: Arc<[u8]> = vec![4, 5].into();
            let late_bytes = Arc::downgrade(&late);
            assert!(!producer.deliver(1, CaptureResourceResult::FontBytes(late)));
            assert!(late_bytes.upgrade().is_none());
        }

        drop(job);
        assert!(queued_bytes.upgrade().is_none());
        assert!(mailbox.upgrade().is_none());
        assert!(!endpoint.is_active());
        assert!(!producer.is_active());
        assert!(
            producer
                .accept_encoded(2, CaptureResourceKind::Font, 1)
                .is_err()
        );
        assert!(!producer.deliver(2, CaptureResourceResult::Failed("late".to_owned())));
    }
}
