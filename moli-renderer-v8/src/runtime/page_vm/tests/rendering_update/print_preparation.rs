use super::*;
use crate::capture_resources::{PrintCaptureAdvance, PrintCaptureJob};
use crate::runtime::{
    RendererCaptureScreenshotReply, RendererCaptureScreenshotRequest, RendererPrintToPdfReply,
    RendererPrintToPdfRequest, RendererScreenshotPurpose,
};
use moli_page_types::LayoutPolicy;

#[test]
fn print_depth_limit_truncates_output_without_weakening_realtime_screen_coverage() {
    run_page_vm_large_stack_async_test("bounded-print-frame-coverage", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        for policy in [LayoutPolicy::OnDemand, LayoutPolicy::Realtime] {
            for depth in [31, 32, 34] {
                let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                    &loader,
                    Url::parse("https://bounded-print.test/").unwrap(),
                );
                page.layout_configuration.policy = policy;
                page.vm_mut().set_layout_policy(policy);
                let count = page
                    .vm_mut()
                    .eval(&format!(
                        r#"(() => {{
                        let d = document;
                        d.body.innerHTML = '<p>Layer 0</p>';
                        for (let i = 1; i <= {depth}; ++i) {{
                            const f = d.createElement('iframe');
                            f.style.cssText = 'width:400px;height:100px;border:0';
                            d.body.appendChild(f);
                            d = f.contentDocument;
                            d.body.innerHTML = '<p>Layer ' + i + '</p>';
                        }}
                        let count = 0;
                        for (d = document; d; ++count) {{
                            const f = d.querySelector('iframe');
                            d = f && f.contentDocument;
                        }}
                        return count;
                    }})()"#,
                    ))
                    .unwrap();
                assert_eq!(count, (depth + 1).to_string());
                let strict_screen = policy.is_realtime() && depth >= 32;
                if strict_screen {
                    let error = page.vm_mut().eval("document.body.offsetWidth").unwrap_err();
                    assert!(error.to_string().contains("embedded frame coverage"));
                }
                let publishes = page.vm().layout_snapshot_cache_observability_for_test().2;
                let mut job = PrintCaptureJob::new(RendererCaptureScreenshotRequest {
                    purpose: RendererScreenshotPurpose::Print {
                        print_background: true,
                    },
                    ..RendererCaptureScreenshotRequest::viewport_png()
                })
                .unwrap();
                match page.advance_print_capture(&mut job).unwrap() {
                    PrintCaptureAdvance::Complete(RendererCaptureScreenshotReply::Captured(
                        image,
                    )) => {
                        assert!(!image.bytes.is_empty());
                    }
                    _ => panic!("{policy:?} print at depth {depth} should produce bounded output"),
                }
                assert!(job.members.len() <= 32);
                assert_eq!(job.members.len(), (depth + 1).min(32));
                page.cancel_print_capture(&mut job);
                // The synchronous API also uses transient output, with no resource job.
                assert!(matches!(
                    page.print_to_pdf(RendererPrintToPdfRequest::default())
                        .unwrap(),
                    RendererPrintToPdfReply::Printed(_)
                ));
                assert_eq!(
                    page.vm().layout_snapshot_cache_observability_for_test().2,
                    publishes,
                    "bounded printing must not publish partial screen coverage"
                );
                if strict_screen {
                    let error = page.vm_mut().eval("document.body.offsetWidth").unwrap_err();
                    assert!(error.to_string().contains("embedded frame coverage"));
                }
            }
        }
    });
}
