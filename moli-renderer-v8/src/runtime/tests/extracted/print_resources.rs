use super::*;
use moli_page_types::LayoutPolicy;

mod lifecycle;

#[derive(Clone, Copy, Debug)]
enum PrintOnlyResource {
    Font,
    Image,
}

impl PrintOnlyResource {
    fn resource(self) -> (&'static str, &'static str, Vec<u8>) {
        match self {
            Self::Font => (
                "/print-only.woff2",
                "font/woff2",
                include_bytes!("../../../../../moli-layout/tests/fixtures/moli-ahem.woff2")
                    .to_vec(),
            ),
            Self::Image => (
                "/print-only.png",
                "image/png",
                moli_image::encode_png(
                    &moli_image::RgbaImage::try_new(1, 1, vec![17, 85, 221, 255])
                        .expect("one valid RGBA pixel"),
                )
                .expect("fixture image encodes")
                .bytes,
            ),
        }
    }

    fn mask(self) -> crate::protocol_types::OptionalResourceFetchMask {
        match self {
            Self::Font => crate::protocol_types::OptionalResourceFetchMask::FONT,
            Self::Image => crate::protocol_types::OptionalResourceFetchMask::IMAGE,
        }
    }

    fn css(self, resource_url: &str) -> String {
        let resource_rule = match self {
            Self::Font => format!(
                "@font-face{{font-family:PrintOnly;src:url({resource_url:?}) format('woff2')}}\
                 #asset{{display:inline-block;font:20px/20px PrintOnly,serif;color:black;white-space:nowrap}}"
            ),
            Self::Image => format!(
                "#asset{{display:block;width:40px;height:20px;background:rgb(255,0,255) url({resource_url:?})}}"
            ),
        };
        format!("@media print{{#screen{{width:80px;background:lime}}{resource_rule}}}")
    }
}

fn screen_cast_request(
    known_visual_state: Option<RendererVisualStateToken>,
) -> RendererCaptureScreencastFrameRequest {
    RendererCaptureScreencastFrameRequest {
        base_background_color: [255; 4],
        format: RendererScreenshotFormat::Png,
        quality: 100,
        optimize_for_speed: false,
        max_width: None,
        max_height: None,
        known_visual_state,
    }
}

async fn evaluate_print_fixture(
    page: &RendererPageHandle,
    expression: String,
    await_promise: bool,
) -> anyhow::Result<serde_json::Value> {
    let (reply, _) = page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression,
            await_promise,
        })
        .await?;
    renderer_json_value(reply).ok_or_else(|| anyhow::anyhow!("evaluation has no owned value"))
}

async fn screen_state(page: &RendererPageHandle) -> anyhow::Result<serde_json::Value> {
    let value = evaluate_print_fixture(
        page,
        r#"JSON.stringify({
            width:document.getElementById('screen').offsetWidth,
            viewport:[innerWidth,innerHeight],
            print:matchMedia('print').matches,
            fontsSame:document.fonts===window.screenFonts,
            fontCount:document.fonts.size,
            events:window.printEvents
        })"#
        .to_owned(),
        false,
    )
    .await?;
    serde_json::from_str(
        value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("screen state must be a JSON string"))?,
    )
    .map_err(Into::into)
}

async fn assert_first_print_resource(resource: PrintOnlyResource) {
    assert_first_print_resource_with_policy(resource, LayoutPolicy::Realtime).await;
}

async fn assert_first_print_resource_with_policy(
    resource: PrintOnlyResource,
    policy: LayoutPolicy,
) {
    let runtime = JsRuntime::initialize();
    runtime
        .renderer_owner_handle()
        .configure_layout_policy(policy)
        .expect("configure layout policy before Page creation");
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mask = resource.mask();
    loader.set_optional_resource_fetch_mask(mask);
    assert_eq!(loader.optional_resource_fetch_mask(), mask);

    let (path, content_type, bytes) = resource.resource();
    let (base_url, mut request_seen, release_response, server) =
        spawn_owner_wake_gated_binary_server_with_content_type(path, bytes, content_type).await;
    let mut release_response = Some(release_response);
    // No resource rule is installed until this resource-free page has loaded.
    // A mistaken screen admission therefore cannot deadlock page creation.
    let html = concat!(
        "<!doctype html><style>",
        "html,body{margin:0;background:white;scrollbar-width:none}",
        "#screen{position:absolute;left:0;top:64px;width:40px;height:16px;background:red}",
        "#asset{display:none;position:absolute;left:8px;top:8px}",
        "</style><div id=screen></div><span id=asset>__PRINT_TEXT__</span>"
    )
    .replace(
        "__PRINT_TEXT__",
        match resource {
            PrintOnlyResource::Font => "iiii",
            PrintOnlyResource::Image => "",
        },
    );
    let mut page = create_test_html_page(
        &runtime,
        &loader,
        url::Url::parse(&format!("{base_url}/page.html")).unwrap(),
        &html,
    )
    .await;
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        page.run_async_command(RendererPageCommand::SetViewportSurface(Some(
            crate::protocol_types::ViewportSurface {
                inner_width: 128,
                inner_height: 96,
                outer_width: 128,
                outer_height: 96,
                screen_width: 128,
                screen_height: 96,
                screen_avail_width: 128,
                screen_avail_height: 96,
                device_pixel_ratio: 1.0,
                ..Default::default()
            },
        )))
        .await?;
        if policy == LayoutPolicy::OnDemand {
            // Legacy observers sample an existing screen tree. Establish that
            // sample before registering them, while this page still has no
            // resource URL. Realtime keeps its original lifecycle-only setup.
            let (reply, _) = page.run_async_command(
                RendererPageCommand::CaptureScreencastFrame(screen_cast_request(None)),
            ).await?;
            anyhow::ensure!(matches!(
                reply,
                RendererPageReply::CaptureScreencastFrame(
                    RendererCaptureScreencastFrameReply::Captured(_)
                )
            ), "OnDemand must establish its initial screen sample");
        }
        let css = serde_json::to_string(&resource.css(&format!("{base_url}{path}")))?;
        let setup = format!(
            r#"(() => {{
                const sheet=new CSSStyleSheet();sheet.replaceSync({css});
                document.adoptedStyleSheets=[sheet];
                window.printEvents=[];
                window.screenFonts=document.fonts;
                for(const type of ['loading','loadingdone','loadingerror'])
                    screenFonts.addEventListener(type,()=>printEvents.push('fonts:'+type));
                matchMedia('print').addEventListener('change',()=>printEvents.push('media'));
                let sawRO=false,sawIO=false;
                const ready=new Promise(resolve=>{{
                    function done(){{if(sawRO&&sawIO)resolve(true)}}
                    window.printRO=new ResizeObserver(es=>{{
                        for(const e of es)printEvents.push('ro:'+e.contentRect.width);
                        sawRO=true;done();
                    }});
                    window.printIO=new IntersectionObserver(es=>{{
                        for(const e of es)printEvents.push('io:'+e.boundingClientRect.width);
                        sawIO=true;done();
                    }});
                    printRO.observe(document.getElementById('screen'));
                    printIO.observe(document.getElementById('screen'));
                }});
                return ready;
            }})()"#
        );
        anyhow::ensure!(
            evaluate_print_fixture(&page, setup, true).await? == serde_json::json!(true),
            "the real owner must deliver the initial screen RO and IO callbacks"
        );
        let (screen_reply, _) = page
            .run_async_command(RendererPageCommand::CaptureScreencastFrame(
                screen_cast_request(None),
            ))
            .await?;
        let RendererPageReply::CaptureScreencastFrame(
            RendererCaptureScreencastFrameReply::Captured(screen),
        ) = screen_reply
        else {
            anyhow::bail!("expected the initial SCREEN frame");
        };
        anyhow::ensure!(
            decoded_png_pixel(&screen.image.bytes, 20, 70) == [255, 0, 0, 255],
            "screen uses its own background"
        );
        anyhow::ensure!(
            decoded_png_pixel(&screen.image.bytes, 12, 12) == [255; 4],
            "the print asset is absent from the screen frame"
        );
        // Consume only ordinary pre-test owner work. No print resource is
        // requested and this does not force the later print to become ready.
        evaluate_print_fixture(
            &page,
            "new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>setTimeout(()=>resolve(true),0))))".to_owned(),
            true,
        )
        .await?;
        let before_state = screen_state(&page).await?;
        anyhow::ensure!(before_state["width"] == 40);
        anyhow::ensure!(before_state["viewport"] == serde_json::json!([128, 96]));
        anyhow::ensure!(before_state["print"] == false);
        anyhow::ensure!(before_state["fontsSame"] == true);
        anyhow::ensure!(before_state["fontCount"] == 0);
        let events = before_state["events"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("observer events must be an array"))?;
        anyhow::ensure!(events.len() == 2, "unexpected initial screen state: {before_state}");
        anyhow::ensure!(events.contains(&serde_json::json!("ro:40")));
        anyhow::ensure!(events.contains(&serde_json::json!("io:40")));
        let (pending, _) = page
            .run_async_command(RendererPageCommand::PendingSubresourceRequestCount)
            .await?;
        anyhow::ensure!(
            matches!(pending, RendererPageReply::Usize(0)),
            "screen must not admit an in-flight resource request"
        );
        anyhow::ensure!(
            matches!(request_seen.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
            "screen must not reach the print-only resource server"
        );
        let testing = RendererPageTestingHandle::new_for_testing(&page);
        let before = testing.layout_working_set_observation_async().await?;
        anyhow::ensure!(before.screen.3.is_some(), "screen has a canonical frozen tree");

        let capture = async {
            let (reply, _) = page
                .run_async_command(RendererPageCommand::CaptureScreenshot(
                    RendererCaptureScreenshotRequest {
                        purpose: RendererScreenshotPurpose::Print {
                            print_background: true,
                        },
                        ..RendererCaptureScreenshotRequest::viewport_png()
                    },
                ))
                .await?;
            let RendererPageReply::CaptureScreenshot(RendererCaptureScreenshotReply::Captured(image)) = reply else {
                anyhow::bail!("first print must produce an owned image");
            };
            Ok::<_, anyhow::Error>(image)
        };
        let resource_response = async {
            tokio::time::timeout(Duration::from_secs(5), &mut request_seen)
                .await
                .map_err(|_| anyhow::anyhow!("first print never requested its enabled {resource:?} resource"))??;
            release_response
                .take()
                .expect("single resource response")
                .send(())
                .map_err(|_| anyhow::anyhow!("resource server disappeared before release"))?;
            Ok::<_, anyhow::Error>(())
        };
        let (first, requested) = tokio::join!(capture, resource_response);
        // The stored first Print reply is never replaced by a later capture or
        // repaired by document.fonts.load()/ready or a geometry command.
        requested?;
        let first = first?;
        anyhow::ensure!((first.width, first.height) == (128, 96));
        anyhow::ensure!(
            decoded_png_pixel(&first.bytes, 60, 70) == [0, 255, 0, 255],
            "the first Print reply must already use print media"
        );
        match resource {
            PrintOnlyResource::Font => {
                // The project fixture is 1000 upem, a 600x800 rectangular
                // glyph, 800 ascent/-200 descent. Four i's at 20px advance
                // exactly 48px; samples stay well inside the glyph rectangle.
                for (x, y) in [(10, 10), (30, 10), (50, 10), (50, 20)] {
                    anyhow::ensure!(
                        decoded_png_pixel(&first.bytes, x, y) == [0, 0, 0, 255],
                        "first print must use the downloaded rectangular font at ({x},{y})"
                    );
                }
                anyhow::ensure!(decoded_png_pixel(&first.bytes, 58, 12) == [255; 4]);
            }
            PrintOnlyResource::Image => {
                for (x, y) in [(12, 12), (42, 22)] {
                    anyhow::ensure!(
                        decoded_png_pixel(&first.bytes, x, y) == [17, 85, 221, 255],
                        "first print must contain the downloaded image at ({x},{y})"
                    );
                }
            }
        }
        // Native metadata is sampled BEFORE any post-print geometry getter or
        // screen capture. A getter cannot hide canonical publication here.
        let after = testing.layout_working_set_observation_async().await?;
        anyhow::ensure!(after.vm_creation_id == before.vm_creation_id);
        anyhow::ensure!(after.document == before.document);
        anyhow::ensure!(after.screen.2 == before.screen.2, "print must not publish screen geometry");
        anyhow::ensure!(after.screen.3 == before.screen.3, "print must not replace the canonical tree");
        anyhow::ensure!(after.inputs == before.inputs, "print-only resources must not invalidate screen inputs");
        let (known_reply, _) = page
            .run_async_command(RendererPageCommand::CaptureScreencastFrame(
                screen_cast_request(Some(screen.visual_state)),
            ))
            .await?;
        anyhow::ensure!(
            matches!(known_reply, RendererPageReply::CaptureScreencastFrame(RendererCaptureScreencastFrameReply::Unchanged)),
            "print-only resource completion must preserve the screen visual token"
        );
        // Allow ordinary subsequent opportunities to expose any erroneous
        // resource-driven RO/IO notifications. These callbacks cannot alter
        // the already stored and validated first Print bytes.
        evaluate_print_fixture(
            &page,
            "new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>setTimeout(()=>resolve(true),0))))".to_owned(),
            true,
        )
        .await?;
        anyhow::ensure!(screen_state(&page).await? == before_state, "screen geometry, viewport, media, fonts, RO and IO stay unchanged");
        anyhow::ensure!(loader.optional_resource_fetch_mask() == mask);
        Ok::<_, anyhow::Error>(())
    })
    .await;

    // Run cleanup even when a positive assertion fails. The gate is bounded;
    // neither a missing request nor an old synchronous print leaves a server.
    // The separate 2s cleanup bound terminates close; it is not a Print or
    // resource-readiness latency requirement.
    let close = tokio::time::timeout(Duration::from_secs(2), page.close_async()).await;
    server.abort();
    let server = server.await;
    assert!(
        close.as_ref().is_ok_and(|result| result.is_ok()),
        "Page close failed: {close:?}; original case result: {result:?}"
    );
    assert!(
        server.is_ok() || server.as_ref().is_err_and(|error| error.is_cancelled()),
        "resource server failed: {server:?}; original case result: {result:?}"
    );
    result
        .expect("print-only resource case must terminate")
        .unwrap_or_else(|error| panic!("{resource:?}: {error:#}"));
}

#[tokio::test(flavor = "multi_thread")]
async fn first_print_loads_adopted_print_only_font_without_mutating_screen() {
    assert_first_print_resource(PrintOnlyResource::Font).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn first_print_loads_print_only_css_image_without_mutating_screen() {
    assert_first_print_resource(PrintOnlyResource::Image).await;
}
