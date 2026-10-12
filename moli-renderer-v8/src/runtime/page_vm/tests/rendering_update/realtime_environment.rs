use super::realtime_opportunity::run_opportunity;
use super::*;
use moli_page_types::LayoutPolicy;

pub(super) fn check_fixture(fixture: &'static str, expected_rows: serde_json::Value) {
    run_page_vm_large_stack_async_test("realtime-environment", move || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime-environment").unwrap(),
        );
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.vm_mut().eval(&format!(
            "globalThis.report=null;({fixture}).then(value=>{{report=JSON.stringify(value)}},error=>{{report=JSON.stringify({{error:String(error)}})}});'ready'",
        )).unwrap();
        // The shared browser fixture has a finite sequence of rAF steps. The
        // owner test advances deadlines explicitly, with no sleeps/captures.
        for _ in 0..64 {
            if page.vm_mut().eval("String(report)").unwrap() != "null" {
                break;
            }
            run_opportunity(&mut page, &loader).await.unwrap();
        }
        let report = page.vm_mut().eval("String(report)").unwrap();
        let report: serde_json::Value = serde_json::from_str(&report).unwrap();
        assert_eq!(report["failures"], serde_json::json!([]), "{report}");
        assert_eq!(report["rows"], expected_rows);
        for _ in 0..3 {
            if page
                .next_rendering_opportunity_deadline_for_test()
                .is_none()
            {
                break;
            }
            run_opportunity(&mut page, &loader).await.unwrap();
        }
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none(),
            "idle page must not keep waking"
        );
        assert_eq!(
            page.vm()
                .layout_pass_observability_for_test()
                .3
                .unwrap()
                .paint_operation_count,
            0
        );
    });
}

#[test]
fn realtime_environment_notifies_child_viewport_and_media_without_capture() {
    check_fixture(
        include_str!("../../../../../tests/fixtures/realtime-viewport-notifications.js"),
        serde_json::json!([
            ["immediate", true],
            ["synchronous", []],
            [
                "events",
                [["window", 480], ["visual", 480], ["media", true]]
            ],
        ]),
    );
}

#[test]
fn realtime_environment_preserves_round_trips_reentrancy_and_document_phase_order() {
    // Chromium 145: native media events expose their live list, even if a
    // later resize/read restores its original match before delivery.
    check_fixture(
        include_str!("../../../../../tests/fixtures/realtime-viewport-notification-order.js"),
        serde_json::json!([
            [
                "coalesced",
                [["window", 520], ["visual", 520], ["media", true]]
            ],
            ["roundtrip-sync", true, false, []],
            [
                "roundtrip-events",
                [["window", 320], ["visual", 320], ["media", false]]
            ],
            [
                "reentrant",
                [
                    ["window", 480],
                    ["reentrant-read", false, 320],
                    ["microtask"],
                    ["visual", 320],
                    ["media", false],
                    ["window", 320],
                    ["visual", 320]
                ]
            ],
            [
                "nested",
                [
                    ["window", 480],
                    ["visual", 480],
                    ["nested-window", 240],
                    ["nested-visual", 240],
                    ["media", true],
                    ["nested-media", true],
                    ["raf"]
                ]
            ],
        ]),
    );
}

#[test]
fn realtime_environment_drives_responsive_frame_without_polling_or_capture() {
    check_fixture(
        include_str!("../../../../../tests/fixtures/realtime-responsive-frame.js"),
        serde_json::json!([
            ["window", 480],
            ["visual", 480],
            ["media", true],
            ["window", 320],
            ["visual", 320],
            ["media", false],
            ["window", 480],
            ["visual", 480],
            ["media", true],
        ]),
    );
}

#[test]
fn realtime_environment_respects_media_listener_lifetime_and_live_event_matches() {
    check_fixture(
        include_str!("../../../../../tests/fixtures/realtime-media-notification-listeners.js"),
        serde_json::json!([
            ["first", 1, [], [], [true]],
            ["second", 1, [false], [], [true, false]],
            ["third", 1, [false, true], [], [true, false], true],
            ["removed", false],
        ]),
    );
}

#[test]
fn realtime_environment_cache_reset_and_document_open_do_not_fabricate_resize() {
    run_page_vm_large_stack_async_test("realtime-environment-reset", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/environment-reset").unwrap(),
        );
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.vm_mut().eval(r#"
            document.body.innerHTML='<iframe id=frame style="width:320px;height:100px;border:0"></iframe>';
            window.child=frame.contentWindow;
            window.events=[];
            child.addEventListener('resize',()=>events.push('window'));
            window.query=child.matchMedia('(min-width:400px)');
            query.onchange=e=>events.push('media:'+e.matches);
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        page.vm_mut().eval("events.length=0").unwrap();
        let builds = page.vm().layout_pass_observability_for_test().1;
        let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
        host.borrow().reset_document_layout_state();
        assert_eq!(
            page.vm_mut().eval("String(child.innerWidth)").unwrap(),
            "320"
        );
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds + 1);
        assert_eq!(page.vm_mut().eval("JSON.stringify(events)").unwrap(), "[]");
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );

        // Chromium keeps the same Window viewport across document.open().
        // New Window listeners must not receive a synthetic initial resize.
        page.vm_mut().eval(r#"
            child.document.open();child.document.write('<!doctype html><body></body>');child.document.close();
            child.addEventListener('resize',()=>events.push('new-window'));
            child.innerWidth;
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(page.vm_mut().eval("JSON.stringify(events)").unwrap(), "[]");
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );

        page.vm_mut()
            .eval("frame.style.width='480px';child.innerWidth")
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(events)").unwrap(),
            "[\"new-window\",\"media:true\"]"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_environment_emulation_uses_frames_while_activity_can_run_hidden() {
    run_page_vm_large_stack_async_test("realtime-environment-emulation", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/environment-emulation").unwrap(),
        );
        let viewport = |width| crate::protocol_types::ViewportSurface {
            inner_width: width,
            inner_height: 600,
            ..Default::default()
        };
        page.vm_mut()
            .set_viewport_surface_for_bootstrap(Some(viewport(800)));
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        run_opportunity(&mut page, &loader).await.unwrap();
        page.vm_mut().eval(r#"
            globalThis.events=[];
            addEventListener('resize',()=>events.push('window:'+innerWidth));
            visualViewport.addEventListener('resize',()=>events.push('visual:'+visualViewport.width));
            matchMedia('(min-width:900px)').onchange=e=>events.push('width:'+e.matches);
            matchMedia('(prefers-color-scheme:dark)').onchange=e=>events.push('dark:'+e.matches);
            screen.orientation.addEventListener('change',()=>events.push('orientation:'+screen.orientation.angle));
            addEventListener('blur',()=>events.push('blur'));
            document.addEventListener('visibilitychange',()=>events.push('visibility:'+document.hidden));
            requestAnimationFrame(()=>events.push('raf'));
        "#).unwrap();
        page.vm_mut()
            .set_viewport_surface(Some(viewport(1024)))
            .unwrap();
        page.vm_mut()
            .set_viewport_surface(Some(viewport(960)))
            .unwrap();
        page.vm_mut()
            .set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides {
                color_scheme: Some("dark".to_owned()),
                ..Default::default()
            });
        assert_eq!(page.vm_mut().eval("JSON.stringify(events)").unwrap(), "[]");
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(events)").unwrap(),
            "[\"window:960\",\"visual:960\",\"width:true\",\"dark:true\",\"raf\"]"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        assert!(
            page.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::RenderingUpdate
            )
            .is_none(),
            "Realtime emulation must not also leave a legacy environment task"
        );
        page.vm_mut().eval("events.length=0").unwrap();
        let mut rotated = viewport(960);
        rotated.screen_orientation = moli_page_types::ScreenOrientation {
            kind: "landscape-primary",
            angle: 90,
        };
        page.vm_mut().set_viewport_surface(Some(rotated)).unwrap();
        let task = page
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
            .unwrap();
        assert_eq!(
            task.rendering_update_owner_and_kind().unwrap().1,
            RendererPageRenderingUpdateTaskKind::EnvironmentChange
        );
        page.run_claimed_selected_page_task_for_test(task, &loader)
            .await
            .unwrap();
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(events)").unwrap(),
            "[\"orientation:90\"]"
        );
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(events)").unwrap(),
            "[\"orientation:90\"]",
            "frame phases must not repeat the platform orientation event"
        );
        page.vm_mut().eval("events.length=0").unwrap();
        page.vm_mut()
            .set_document_activity(moli_page_types::DocumentActivity {
                visible: false,
                focused: false,
            })
            .unwrap();
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        let task = page
            .claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::RenderingUpdate)
            .unwrap();
        assert_eq!(
            task.rendering_update_owner_and_kind().unwrap().1,
            RendererPageRenderingUpdateTaskKind::EnvironmentChange
        );
        page.run_claimed_selected_page_task_for_test(task, &loader)
            .await
            .unwrap();
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(events)").unwrap(),
            "[\"blur\",\"visibility:true\"]"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}
