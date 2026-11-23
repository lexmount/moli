use super::*;
use crate::capture_resources::{PrintCaptureAdvance, PrintCaptureJob, PrintCaptureReply};
use crate::runtime::{
    RendererCaptureScreenshotReply, RendererCaptureScreenshotRequest, RendererPrintToPdfReply,
    RendererPrintToPdfRequest, RendererScreenshotPurpose,
};
use moli_page_types::LayoutPolicy;

#[tokio::test(flavor = "current_thread")]
async fn ready_print_resources_need_one_layout_and_never_publish_screen_geometry() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        for policy in [LayoutPolicy::OnDemand, LayoutPolicy::Realtime] {
            let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://print-preparation.test/")?,
            );
            page.layout_configuration.policy = policy;
            page.vm_mut().set_layout_policy(policy);
            page.vm_mut().eval(
                r#"document.head.innerHTML='<style>body{margin:0}p{width:100px}@media print{p{width:200px}}</style>';
                   document.body.innerHTML='<p>Printed text</p>';"#,
            )?;
            let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
            let mut text_passes = 0;
            assert_eq!(
                host.borrow().layout_text_services_observability_for_test(),
                (false, 0)
            );
            for raster in [false, true] {
                if raster {
                    page.vm_mut().eval(
                        "document.querySelector('p').style.backgroundImage='url(https://disabled-print-image.invalid/image.png)'",
                    )?;
                }
                let mut job = if raster {
                    PrintCaptureJob::new(RendererCaptureScreenshotRequest {
                        purpose: RendererScreenshotPurpose::Print {
                            print_background: true,
                        },
                        ..RendererCaptureScreenshotRequest::viewport_png()
                    })?
                } else {
                    PrintCaptureJob::new(RendererPrintToPdfRequest::default())?
                };
                let before = page.vm().layout_pass_observability_for_test().1;
                let publishes = page.vm().layout_snapshot_cache_observability_for_test().2;
                match page.advance_print_capture(&mut job)? {
                    PrintCaptureAdvance::Complete(PrintCaptureReply::Pdf(
                        RendererPrintToPdfReply::Printed(bytes),
                    )) => {
                        let pdf = lopdf::Document::load_mem(&bytes)?;
                        assert!(
                            pdf.extract_text(&pdf.get_pages().keys().copied().collect::<Vec<_>>())?
                                .contains("Printed text")
                        );
                    }
                    PrintCaptureAdvance::Complete(PrintCaptureReply::Raster(
                        RendererCaptureScreenshotReply::Captured(image),
                    )) => assert!(!image.bytes.is_empty()),
                    _ => anyhow::bail!("ready resources must produce the requested output"),
                }
                assert_eq!(
                    page.vm().layout_pass_observability_for_test().1 - before,
                    1,
                    "ready {policy:?} print must consume its first layout"
                );
                assert_eq!(
                    page.vm().layout_snapshot_cache_observability_for_test().2,
                    publishes,
                    "print must remain transient"
                );
                assert!(job.fetches.is_empty());
                if raster {
                    assert!(
                        job.entries.is_empty(),
                        "disabled resources must not consume capture entries"
                    );
                }
                let services = host.borrow().layout_text_services_observability_for_test();
                assert!(services.0, "matching fonts should initialize the live text service");
                assert!(services.1 > text_passes, "matching fonts should reuse that service");
                text_passes = services.1;
                page.cancel_print_capture(&mut job);
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("ready print resources use a single transient pass");
}

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
                for raster in [false, true] {
                    let mut job = if raster {
                        PrintCaptureJob::new(RendererCaptureScreenshotRequest {
                            purpose: RendererScreenshotPurpose::Print {
                                print_background: true,
                            },
                            ..RendererCaptureScreenshotRequest::viewport_png()
                        })
                    } else {
                        PrintCaptureJob::new(RendererPrintToPdfRequest::default())
                    }
                    .unwrap();
                    match page.advance_print_capture(&mut job).unwrap() {
                        PrintCaptureAdvance::Complete(PrintCaptureReply::Pdf(
                            RendererPrintToPdfReply::Printed(bytes),
                        )) => {
                            let pdf = lopdf::Document::load_mem(&bytes).unwrap();
                            assert!(
                                pdf.extract_text(
                                    &pdf.get_pages().keys().copied().collect::<Vec<_>>()
                                )
                                .unwrap()
                                .contains("Layer 0")
                            );
                        }
                        PrintCaptureAdvance::Complete(PrintCaptureReply::Raster(
                            RendererCaptureScreenshotReply::Captured(image),
                        )) => assert!(!image.bytes.is_empty()),
                        _ => panic!(
                            "{policy:?} print at depth {depth} should produce bounded output"
                        ),
                    }
                    assert!(job.members.len() <= 32);
                    assert_eq!(job.members.len(), (depth + 1).min(32));
                    page.cancel_print_capture(&mut job);
                }
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

fn install_many_capture_fonts(page: &mut PageVm, print_only: bool) -> anyhow::Result<String> {
    use base64::Engine;
    let data = base64::engine::general_purpose::STANDARD.encode(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../moli-layout/tests/fixtures/moli-ahem.woff2"
    )));
    page.vm_mut().eval(&format!(r#"(() => {{
        const rules=Array.from({{length:65}},(_,i)=>
            `@font-face{{font-family:Cached${{i}};src:url(data:font/woff2;base64,{data})}}`).join('');
        document.head.innerHTML='<style>'+({print_only}?'@media print{{'+rules+'}}':rules)+'</style>';
        document.body.innerHTML='<p style="font:16px Cached0">Cached text</p>';
        const fonts=[...document.fonts];
        if(!{print_only})for(const face of fonts)face.load();
        return JSON.stringify([fonts.length,fonts.every(face=>face.status==='loaded')]);
    }})()"#))
}

#[tokio::test(flavor = "current_thread")]
async fn print_cached_fonts_do_not_consume_resource_admission() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        for policy in [LayoutPolicy::OnDemand, LayoutPolicy::Realtime] {
            let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://cached-print-fonts.test/")?,
            );
            page.layout_configuration.policy = policy;
            page.vm_mut().set_layout_policy(policy);
            assert_eq!(install_many_capture_fonts(&mut page, false)?, "[65,true]");
            for raster in [false, true] {
                let mut job = if raster {
                    PrintCaptureJob::new(RendererCaptureScreenshotRequest {
                        purpose: RendererScreenshotPurpose::Print {
                            print_background: true,
                        },
                        ..RendererCaptureScreenshotRequest::viewport_png()
                    })?
                } else {
                    PrintCaptureJob::new(RendererPrintToPdfRequest::default())?
                };
                let before = page.vm().layout_pass_observability_for_test().1;
                let publishes = page.vm().layout_snapshot_cache_observability_for_test().2;
                match page.advance_print_capture(&mut job)? {
                    PrintCaptureAdvance::Complete(PrintCaptureReply::Pdf(
                        RendererPrintToPdfReply::Printed(bytes),
                    )) => assert!(bytes.starts_with(b"%PDF-")),
                    PrintCaptureAdvance::Complete(PrintCaptureReply::Raster(
                        RendererCaptureScreenshotReply::Captured(image),
                    )) => assert!(!image.bytes.is_empty()),
                    _ => anyhow::bail!("cached fonts must not require another resource wave"),
                }
                assert_eq!(page.vm().layout_pass_observability_for_test().1 - before, 1);
                assert_eq!(
                    page.vm().layout_snapshot_cache_observability_for_test().2,
                    publishes
                );
                assert!(
                    job.entries.is_empty(),
                    "ready fonts cannot consume preparation entries"
                );
                assert!(job.fetches.is_empty());
                assert_eq!(page.vm_mut().eval("document.fonts.size")?, "65");
                page.cancel_print_capture(&mut job);
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("ready print fonts are not new preparation work");
}

#[tokio::test(flavor = "current_thread")]
async fn print_reuses_loaded_font_sources_without_new_preparation_entries() {
    run_page_vm_async_test(async move {
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.encode(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-ahem.woff2"
        )));
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        for policy in [LayoutPolicy::OnDemand, LayoutPolicy::Realtime] {
            let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://cached-print-sources.test/")?,
            );
            page.layout_configuration.policy = policy;
            page.vm_mut().set_layout_policy(policy);
            assert_eq!(page.vm_mut().eval(&format!(r#"(() => {{
                const rules=prefix=>Array.from({{length:65}},(_,i)=>
                    `@font-face{{font-family:${{prefix}}${{i}};src:url(data:font/woff2;base64,{data})}}`).join('');
                document.head.innerHTML='<style>@media screen{{'+rules('Screen')+
                    '}}@media print{{'+rules('Print')+'p{{font-family:Print0}}}}</style>';
                document.body.innerHTML='<p style="font-size:16px">Cached text</p>';
                window.screenFonts=document.fonts;
                window.screenFaces=[...screenFonts];
                for(const face of screenFaces)face.load();
                return JSON.stringify([screenFaces.length,screenFaces.every(face=>face.status==='loaded')]);
            }})()"#))?, "[65,true]");
            let fonts = page.vm_mut().document_web_font_counts_for_test();
            let publishes = page.vm().layout_snapshot_cache_observability_for_test().2;
            for raster in [false, true] {
                let mut job = if raster {
                    PrintCaptureJob::new(RendererCaptureScreenshotRequest {
                        purpose: RendererScreenshotPurpose::Print {
                            print_background: true,
                        },
                        ..RendererCaptureScreenshotRequest::viewport_png()
                    })?
                } else {
                    PrintCaptureJob::new(RendererPrintToPdfRequest::default())?
                };
                let before = page.vm().layout_pass_observability_for_test().1;
                match page.advance_print_capture(&mut job)? {
                    PrintCaptureAdvance::Complete(PrintCaptureReply::Pdf(
                        RendererPrintToPdfReply::Printed(bytes),
                    )) => {
                        let pdf = lopdf::Document::load_mem(&bytes)?;
                        assert!(pdf.extract_text(&[1])?.contains("Cached text"));
                    }
                    PrintCaptureAdvance::Complete(PrintCaptureReply::Raster(
                        RendererCaptureScreenshotReply::Captured(image),
                    )) => assert!(!image.bytes.is_empty()),
                    _ => anyhow::bail!("cached sources must satisfy new print declaration identities"),
                }
                assert_eq!(page.vm().layout_pass_observability_for_test().1 - before, 1);
                assert!(job.entries.is_empty(), "cached sources are not new preparation work");
                assert!(job.fetches.is_empty());
                assert_eq!(job.overlay.retained_font_bytes(), 0, "shared decoded data must not be charged as new bytes");
                assert_eq!(page.vm().layout_snapshot_cache_observability_for_test().2, publishes);
                assert_eq!(page.vm_mut().document_web_font_counts_for_test(), fonts);
                assert_eq!(page.vm_mut().eval(
                    "JSON.stringify([document.fonts===screenFonts,document.fonts.size,screenFaces.every(face=>screenFonts.has(face)&&face.status==='loaded'),matchMedia('print').matches])"
                )?, "[true,65,true,false]");
                page.cancel_print_capture(&mut job);
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("cached sources survive temporary media declaration selection");
}

#[tokio::test(flavor = "current_thread")]
async fn print_new_fonts_still_respect_preparation_entry_limit() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        for policy in [LayoutPolicy::OnDemand, LayoutPolicy::Realtime] {
            let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://new-print-fonts.test/")?,
            );
            page.layout_configuration.policy = policy;
            page.vm_mut().set_layout_policy(policy);
            assert_eq!(install_many_capture_fonts(&mut page, true)?, "[0,true]");
            for raster in [false, true] {
                let mut job = if raster {
                    PrintCaptureJob::new(RendererCaptureScreenshotRequest {
                        purpose: RendererScreenshotPurpose::Print {
                            print_background: true,
                        },
                        ..RendererCaptureScreenshotRequest::viewport_png()
                    })?
                } else {
                    PrintCaptureJob::new(RendererPrintToPdfRequest::default())?
                };
                let error = page
                    .advance_print_capture(&mut job)
                    .err()
                    .expect("65 new fonts must retain the capture preparation limit");
                assert!(
                    error
                        .to_string()
                        .contains("print resource manifest entry limit exceeded")
                );
                assert_eq!(
                    job.entries.len(),
                    crate::capture_resources::MAX_CAPTURE_RESOURCE_ENTRIES
                );
                assert!(
                    job.fetches.is_empty(),
                    "failed admission must not start partial downloads"
                );
                page.cancel_print_capture(&mut job);
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("new print resource work remains bounded");
}
