use super::*;
use crate::runtime::{
    RendererCaptureScreenshotReply, RendererCaptureScreenshotRequest, RendererPrintToPdfReply,
    RendererPrintToPdfRequest, RendererScreenshotPurpose,
};
use moli_page_types::LayoutPolicy;

#[tokio::test(flavor = "current_thread")]
async fn native_pdf_extracts_only_bullets_for_transformed_password_values() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/password-print.html")?,
        );
        page.vm_mut().eval(
            r#"
document.head.innerHTML = '<style>@media print { input { text-transform: uppercase; } }</style>';
document.body.innerHTML = '<input type=password value="Aß😀e\u0301Z">';
'installed'
"#,
        )?;
        let reply = page.print_to_pdf(RendererPrintToPdfRequest::default())?;
        let RendererPrintToPdfReply::Printed(bytes) = reply else {
            anyhow::bail!("password PDF failed: {reply:?}");
        };
        let pdf = lopdf::Document::load_mem(&bytes)?;
        assert_eq!(pdf.extract_text(&[1])?.trim(), "••••••");
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("password PDF fixture");
}

#[tokio::test(flavor = "current_thread")]
async fn native_pdf_requests_only_css_images_used_by_the_capture() {
    run_page_vm_async_test(async move {
        let image = "data:image/webp;base64,UklGRhwAAABXRUJQVlA4TA8AAAAvAUAAAAcQ/Y/+ByKi/wEA";
        for policy in [LayoutPolicy::Lazy, LayoutPolicy::Realtime] {
            for (print_background, uses_mask) in [(false, false), (true, false), (false, true)] {
                let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
                loader.set_optional_resource_fetch_mask(
                    crate::protocol_types::OptionalResourceFetchMask::IMAGE,
                );
                let mut page = test_page_vm_with_loader_and_document_url(
                    &loader,
                    Vec::new(),
                    Url::parse("https://print-css-image-admission.test/")?,
                );
                page.layout_configuration.policy = policy;
                page.vm_mut().set_layout_policy(policy);

                let mask = if uses_mask {
                    format!("mask-image: url('{image}');")
                } else {
                    String::new()
                };
                let css = format!(
                    "@media screen {{ p {{ background-image: url('/screen.png') }} }} \
                     @media print {{ p {{ background-image: url('{image}'); {mask} }} }}"
                );
                page.vm_mut().eval(&format!(
                    "document.head.innerHTML='<style></style>';\
                     document.querySelector('style').textContent={};\
                     document.body.innerHTML='<p>Print CSS images</p>';",
                    serde_json::to_string(&css)?,
                ))?;
                assert!(
                    page.vm()
                        .css_image_resource_observability_for_test()
                        .4
                        .is_empty()
                );

                let expected = if print_background || uses_mask {
                    vec![image.to_owned()]
                } else {
                    Vec::new()
                };
                let request = RendererPrintToPdfRequest {
                    print_background,
                    ..Default::default()
                };
                assert!(matches!(
                    page.print_to_pdf(request)?,
                    RendererPrintToPdfReply::Printed(_)
                ));
                assert_eq!(
                    page.vm().css_image_resource_observability_for_test().4,
                    expected,
                    "{policy:?}, print_background={print_background}, uses_mask={uses_mask}"
                );
            }
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("PDF capture admits its background and mask resources according to their use");
}

#[tokio::test(flavor = "current_thread")]
async fn native_pdf_embeds_a_loaded_font_under_its_print_declaration() {
    run_page_vm_async_test(async move {
        use base64::Engine;

        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        loader.set_optional_resource_fetch_mask(crate::protocol_types::OptionalResourceFetchMask::FONT);
        let data = base64::engine::general_purpose::STANDARD.encode(include_bytes!(
            "../../../../../../moli-layout/tests/fixtures/moli-ahem.woff2"
        ));
        for policy in [LayoutPolicy::Lazy, LayoutPolicy::Realtime] {
            let mut page = test_page_vm_with_loader_and_document_url(
                &loader,
                Vec::new(),
                Url::parse("https://cached-print-sources.test/")?,
            );
            page.layout_configuration.policy = policy;
            page.vm_mut().set_layout_policy(policy);
            let loaded = page.vm_mut().eval(&format!(r#"
                document.head.innerHTML = '<style>' +
                    '@media screen {{ @font-face {{ font-family: ScreenOnly; src: url(data:font/woff2;base64,{data}) }} }}' +
                    '@media print {{ @font-face {{ font-family: PrintOnly; src: url(data:font/woff2;base64,{data}) }}' +
                    'p {{ font-family: PrintOnly }} }}' + '</style>';
                document.body.innerHTML = '<p style="font-size:16px">Cached text</p>';
                window.screenFonts = document.fonts;
                window.screenFace = [...screenFonts].find(face => face.family === 'ScreenOnly');
                screenFace.load();
                JSON.stringify([screenFonts.size, screenFace.status]);
            "#))?;
            assert_eq!(loaded, r#"[1,"loaded"]"#);
            let fonts = page.vm_mut().document_web_font_counts_for_test();
            let publishes = page.vm().layout_snapshot_cache_observability_for_test().2;

            let reply = page.print_to_pdf(RendererPrintToPdfRequest::default())?;
            let RendererPrintToPdfReply::Printed(bytes) = reply else {
                anyhow::bail!("expected PDF for {policy:?}, got {reply:?}");
            };
            let pdf = lopdf::Document::load_mem(&bytes)?;
            assert!(pdf.extract_text(&[1])?.contains("Cached text"));
            assert!(
                pdf.objects.values().any(|object| {
                    object.as_dict().is_ok_and(|dict| {
                        dict.get(b"BaseFont")
                            .and_then(lopdf::Object::as_name)
                            .is_ok_and(|name| String::from_utf8_lossy(name).contains("MoliAhem"))
                    })
                }),
                "{policy:?} must embed the font loaded through the screen declaration"
            );
            assert_eq!(page.vm().layout_snapshot_cache_observability_for_test().2, publishes);
            assert_eq!(page.vm_mut().document_web_font_counts_for_test(), fonts);
            assert_eq!(
                page.vm_mut().eval(
                    "JSON.stringify([document.fonts === screenFonts, screenFonts.size, \
                     screenFonts.has(screenFace), screenFace.status, matchMedia('print').matches])"
                )?,
                r#"[true,1,true,"loaded",false]"#
            );
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("printing should reuse the loaded font without changing the screen FontFaceSet");
}

#[test]
fn print_truncates_deep_frames_without_publishing_partial_screen_layout() {
    run_page_vm_large_stack_async_test("bounded-print-frame-coverage", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        for policy in [LayoutPolicy::Lazy, LayoutPolicy::Realtime] {
            for depth in [31, 32] {
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
                            const frame = d.createElement('iframe');
                            frame.style.cssText = 'width:400px;height:100px;border:0';
                            d.body.appendChild(frame);
                            d = frame.contentDocument;
                            d.body.innerHTML = '<p>Layer ' + i + '</p>';
                        }}
                        let count = 0;
                        for (d = document; d; ++count) {{
                            const frame = d.querySelector('iframe');
                            d = frame && frame.contentDocument;
                        }}
                        return count;
                    }})()"#,
                    ))
                    .unwrap();
                assert_eq!(count, (depth + 1).to_string());
                let strict_screen_exceeds_depth = policy.is_realtime() && depth == 32;
                let screen_width = page.vm_mut().eval("document.body.offsetWidth");
                if strict_screen_exceeds_depth {
                    assert!(
                        screen_width
                            .unwrap_err()
                            .to_string()
                            .contains("embedded frame coverage")
                    );
                } else {
                    assert!(screen_width.unwrap().parse::<u32>().unwrap() > 0);
                }
                let publishes = page.vm().layout_snapshot_cache_observability_for_test().2;

                let reply = page
                    .print_to_pdf(RendererPrintToPdfRequest::default())
                    .unwrap();
                let RendererPrintToPdfReply::Printed(bytes) = reply else {
                    panic!("PDF output for {policy:?} at depth {depth} failed with {reply:?}");
                };
                let pdf = lopdf::Document::load_mem(&bytes).unwrap();
                assert!(pdf.extract_text(&[1]).unwrap().contains("Layer 0"));

                let reply = page
                    .capture_screenshot(RendererCaptureScreenshotRequest {
                        purpose: RendererScreenshotPurpose::Print {
                            print_background: true,
                        },
                        ..RendererCaptureScreenshotRequest::viewport_png()
                    })
                    .unwrap();
                assert!(
                    matches!(reply, RendererCaptureScreenshotReply::Captured(image) if !image.bytes.is_empty())
                );
                assert_eq!(
                    page.vm().layout_snapshot_cache_observability_for_test().2,
                    publishes
                );
                if strict_screen_exceeds_depth {
                    let error = page.vm_mut().eval("document.body.offsetWidth").unwrap_err();
                    assert!(error.to_string().contains("embedded frame coverage"));
                }
            }
        }
    });
}

#[tokio::test(flavor = "current_thread")]
async fn native_pdf_prints_dense_link_columns_without_losing_content_or_annotations() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/dense-links.html")?,
        );
        page_vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 300,
            inner_height: 300,
            ..Default::default()
        }))?;
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body { margin:0 }
a { position:absolute; top:20px; width:0.0625px; height:10px }
p { position:absolute; top:60px; margin:0 }
</style>`;
document.body.innerHTML = '<p>Dense link content</p>' +
    Array.from({length:1600}, (_, i) =>
        `<a href="https://example.org/dense" style="left:${i / 8}px"></a>`).join('');
'installed'
"#,
        )?;
        let reply = page_vm.print_to_pdf(RendererPrintToPdfRequest {
            options: moli_paint::pdf::PdfOptions {
                width: 300.0,
                height: 300.0,
                margin_left: 0.0,
                margin_right: 0.0,
                margin_top: 0.0,
                margin_bottom: 0.0,
                ..Default::default()
            },
            ..Default::default()
        })?;
        let RendererPrintToPdfReply::Printed(bytes) = reply else {
            anyhow::bail!("dense link PDF failed: {reply:?}");
        };
        let document = lopdf::Document::load_mem(&bytes)?;
        let mut links = 0;
        for object in document.objects.values() {
            let Ok(dict) = object.as_dict() else { continue };
            if dict.get(b"Subtype").and_then(|o| o.as_name()).ok() != Some(b"Link") {
                continue;
            }
            links += 1;
            assert_eq!(
                dict.get(b"A")?.as_dict()?.get(b"URI")?.as_str()?,
                b"https://example.org/dense"
            );
        }
        assert_eq!(links, 1_600);
        let text = document
            .extract_text(&[1])?
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(text.contains("Dense link content"), "{text:?}");
        assert_eq!(
            page_vm.vm_mut().eval("matchMedia('print').matches")?,
            "false"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("dense link pages should retain their PDF content and annotations");
}

#[tokio::test(flavor = "current_thread")]
async fn native_pdf_resolves_links_base_urls_anchors_and_embedded_frames() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/doc.html")?,
        );
        page_vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 300,
            inner_height: 300,
            ..Default::default()
        }))?;
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<base href="https://example.org/base/"><style>
body { margin:0 } a { display:block; width:100px; height:20px }
#wrapped { position:absolute; top:120px; line-height:20px }
#wrapped a { display:inline }
iframe { position:absolute; left:120px; top:20px; width:120px; height:80px; border:0 }
#destination { position:absolute; top:400px; height:20px }
</style>`;
document.body.innerHTML = `<a href="../relative?q=1&amp;x=2">Relative</a>
<a href="mailto:hello@example.org">Mail</a>
<a href="https://example.com/doc.html#dest%20name">Internal</a>
<a href="javascript:void(0)">Script</a>
<a href="https://hidden.invalid/" style="opacity:0">Hidden</a>
<a href="https://display.invalid/" style="display:none">None</a>
<div id=wrapped><a href="https://wrapped.example/link">First<br>Second</a></div>
<div id="destination"><span id="dest name">Destination</span></div>
<iframe id=frame></iframe>`;
frame.contentDocument.body.innerHTML = '<a href="https://child.example/link">Child</a>';
'installed'
"#,
        )?;
        let request = RendererPrintToPdfRequest {
            options: moli_paint::pdf::PdfOptions {
                width: 300.0,
                height: 300.0,
                margin_left: 0.0,
                margin_right: 0.0,
                margin_top: 0.0,
                margin_bottom: 0.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let RendererPrintToPdfReply::Printed(bytes) = page_vm.print_to_pdf(request)? else {
            anyhow::bail!("print failed");
        };
        let doc = lopdf::Document::load_mem(&bytes)?;
        let mut urls = std::collections::BTreeSet::new();
        let mut destinations = Vec::new();
        let mut wrapped_regions = Vec::new();
        for object in doc.objects.values() {
            let Ok(dict) = object.as_dict() else { continue };
            if dict.get(b"Subtype").and_then(|o| o.as_name()).ok() != Some(b"Link") {
                continue;
            }
            if let Ok(action) = dict.get(b"A") {
                let action = action.as_dict()?;
                let url = String::from_utf8(action.get(b"URI")?.as_str()?.to_vec())?;
                if url == "https://wrapped.example/link" {
                    wrapped_regions.push(dict.get(b"Rect")?.as_array()?.clone());
                }
                urls.insert(url);
            } else {
                let (_, dest) = doc.dereference(dict.get(b"Dest")?)?;
                destinations.push(dest.as_array()?[0].as_reference()?);
            }
        }
        assert_eq!(
            urls,
            [
                "https://example.org/relative?q=1&x=2",
                "mailto:hello@example.org",
                "https://child.example/link",
                "https://wrapped.example/link"
            ]
            .map(str::to_owned)
            .into()
        );
        assert!(!destinations.is_empty());
        assert!(destinations.iter().all(|id| *id == doc.get_pages()[&2]));
        assert_eq!(
            wrapped_regions.len(),
            2,
            "one link annotation per wrapped line"
        );
        assert_ne!(wrapped_regions[0][1], wrapped_regions[1][1]);
        assert_eq!(
            page_vm.vm_mut().eval("matchMedia('print').matches")?,
            "false"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("print links preserve URL and document ownership");
}

#[tokio::test(flavor = "current_thread")]
async fn native_pdf_restores_media_on_success_and_failure_and_keeps_iframe_text() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader, Vec::new(), Url::parse("https://example.com/native-print.html")?,
        );
        page_vm.vm_mut().eval(r#"
document.head.innerHTML = `<style>
.print { display:none } @media print { .screen { display:none } .print { display:block } }
</style>`;
document.body.innerHTML = '<p class=screen>Screen parent</p><p class=print>Print parent</p><iframe id=child></iframe>';
child.contentDocument.body.innerHTML = '<p>Iframe text</p>';
'installed'
"#)?;
        let extract = |reply| -> anyhow::Result<String> {
            let RendererPrintToPdfReply::Printed(bytes) = reply else { anyhow::bail!("expected PDF, got {reply:?}"); };
            let doc = lopdf::Document::load_mem(&bytes)?;
            Ok(doc.extract_text(&doc.get_pages().keys().copied().collect::<Vec<_>>())?
                .split_whitespace().collect::<Vec<_>>().join(" "))
        };
        let cold = page_vm.vm().layout_snapshot_cache_observability_for_test().2;
        assert_eq!(extract(page_vm.print_to_pdf(Default::default())?)?, "Print parent Iframe text");
        assert_eq!(page_vm.vm().layout_snapshot_cache_observability_for_test().2, cold);
        assert_eq!(page_vm.vm_mut().eval("matchMedia('print').matches")?, "false");

        page_vm.capture_screenshot(RendererCaptureScreenshotRequest::viewport_png())?;
        let warm = page_vm.vm().layout_snapshot_cache_observability_for_test().2;
        let mut invalid = RendererPrintToPdfRequest::default();
        invalid.options.width = 0.0;
        assert!(matches!(page_vm.print_to_pdf(invalid)?, RendererPrintToPdfReply::Failed(_)));
        assert_eq!(page_vm.vm().layout_snapshot_cache_observability_for_test().2, warm);
        assert_eq!(page_vm.vm_mut().eval("matchMedia('print').matches")?, "false");

        let mut media = page_vm.document_settings.emulated_media.clone();
        media.media = Some("screen".to_owned());
        page_vm.set_emulated_media(&media)?;
        assert_eq!(extract(page_vm.print_to_pdf(Default::default())?)?, "Screen parent Iframe text");
        assert_eq!(page_vm.document_settings.emulated_media, media);
        assert_eq!(page_vm.vm().layout_snapshot_cache_observability_for_test().2, warm);
        Ok::<_, anyhow::Error>(())
    }).await.expect("native printing preserves media and interactive state");
}

#[cfg(feature = "devtools")]
#[tokio::test(flavor = "current_thread")]
async fn print_to_pdf_preserves_published_geometry_frame_viewports_and_mouse_targets() {
    run_page_vm_async_test(async move {
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let mut page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse("https://example.com/print-layout.html")?,
        );
        page_vm.set_viewport_surface(Some(crate::protocol_types::ViewportSurface {
            inner_width: 320,
            inner_height: 100,
            ..Default::default()
        }))?;
        page_vm.vm_mut().eval(
            r#"
document.head.innerHTML = `<style>
html,body { margin:0; background:white }
#target,iframe { position:absolute; left:0; top:0; width:40px; height:40px; border:0 }
#target { background:red }
iframe { top:50px }
.changed #target,.changed iframe { width:60px }
.print-label { display:none }
@media print {
  #target,iframe { left:160px; width:80px !important }
  #target { background:lime }
  .print-label { display:block }
}
</style>`;
document.body.innerHTML = '<div id=target><span class=print-label>Target</span></div><iframe id=frame></iframe>';
const child = frame.contentDocument;
child.documentElement.style.cssText = 'margin:0';
child.body.style.cssText = 'margin:0;background:blue;height:40px';
child.head.innerHTML = '<style>.print-label{display:none}@media print{.print-label{display:block}}</style>';
child.body.innerHTML = '<span class=print-label>Frame</span>';
globalThis.clicks = 0;
target.onclick = () => clicks++;
'installed'
"#,
        )?;
        page_vm.vm_mut().sync_live_document_style_sources();
        page_vm.capture_screenshot(RendererCaptureScreenshotRequest::viewport_png())?;
        let geometry = "JSON.stringify([target.getBoundingClientRect().width,frame.getBoundingClientRect().width,frame.contentDocument.body.getBoundingClientRect().width,frame.contentWindow.innerWidth])";
        assert_eq!(page_vm.vm_mut().eval(geometry)?, "[40,40,40,40]");
        let before = page_vm.vm().layout_pass_observability_for_test();
        let published_before = page_vm.vm().layout_snapshot_cache_observability_for_test().2;

        // Restoring the screen media after printing must not rebuild the live
        // DOM either: geometry continues to describe the earlier screenshot.
        page_vm.vm_mut().eval("document.body.classList.add('changed')")?;
        let RendererPrintToPdfReply::Printed(bytes) = page_vm.print_to_pdf(
            RendererPrintToPdfRequest {
                print_background: true,
                ..Default::default()
            },
        )? else {
            panic!("printing should produce a PDF");
        };
        let document = lopdf::Document::load_mem(&bytes)?;
        assert_eq!(
            document.extract_text(&document.get_pages().keys().copied().collect::<Vec<_>>())?
                .split_whitespace().collect::<Vec<_>>().join(" "),
            "Target Frame",
            "PDF should contain print-only content and the embedded frame"
        );
        assert_eq!(page_vm.vm_mut().eval("matchMedia('print').matches")?, "false");
        assert_eq!(page_vm.vm_mut().eval(geometry)?, "[40,40,40,40]");
        let after = page_vm.vm().layout_pass_observability_for_test();
        assert_eq!(after.1, before.1 + 1, "printing must perform exactly one layout");
        assert_eq!(after.3, before.3, "geometry keeps its published pass metrics");
        assert_eq!(page_vm.vm().layout_snapshot_cache_observability_for_test().2, published_before);

        for x in [20.0, 180.0] {
            for (event, buttons) in [("mousedown", 1), ("mouseup", 0)] {
                page_vm.vm_mut().dispatch_mouse_event_at_point(
                    x, 20.0, event, 0, Some(buttons), 0.0, 0.0,
                )?;
            }
        }
        assert_eq!(page_vm.vm_mut().eval("String(clicks)")?, "1");
        assert_eq!(page_vm.vm().layout_pass_observability_for_test().1, after.1);

        page_vm.capture_screenshot(RendererCaptureScreenshotRequest::viewport_png())?;
        assert_eq!(page_vm.vm_mut().eval(geometry)?, "[60,60,60,60]");
        assert_eq!(page_vm.vm().layout_snapshot_cache_observability_for_test().2, published_before + 1);
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("print should preserve the interactive layout");
}
