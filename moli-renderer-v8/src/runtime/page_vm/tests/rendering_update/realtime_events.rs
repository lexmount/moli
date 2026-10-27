use super::realtime_environment::check_fixture;
use super::realtime_opportunity::run_opportunity;
use super::*;
use moli_page_types::LayoutPolicy;

#[test]
fn realtime_scroll_phase_orders_documents_and_snapshots_reentrant_batches() {
    // Chromium 145 matches the first and third rows. Its unfiltered later
    // animation-event drain also dispatches reentrant scroll before rAF.
    // Blink 157's stable EventTimingMatchingHTML path explicitly excludes
    // scroll there; like the HTML phase model, we defer the new batch.
    check_fixture(
        include_str!("../../../../../tests/fixtures/realtime-scroll-phases.js"),
        serde_json::json!([
            [
                "phases",
                [
                    ["resize", 480],
                    ["visual", 480],
                    ["scroll", 20],
                    ["scrollend", 20],
                    ["child-scroll", 15],
                    ["child-scrollend", 15],
                    ["media", true],
                    ["raf"]
                ]
            ],
            [
                "reentrant",
                [
                    ["scroll", 30],
                    ["microtask"],
                    ["scrollend", 40],
                    ["raf"],
                    ["scroll", 40],
                    ["scrollend", 40]
                ]
            ],
            [
                "element",
                [["element-scroll", 20], ["element-scrollend", 20], ["raf"]]
            ]
        ]),
    );
}

#[test]
fn realtime_element_scroll_drives_dom_and_further_scroll_without_polling() {
    check_fixture(
        include_str!("../../../../../tests/fixtures/realtime-scroll-driven-pane.js"),
        serde_json::json!([
            ["scroll", 40, false, false, true],
            ["scrollend", 80, false, false, true],
            ["scroll", 80, false, false, true],
            ["scrollend", 120, false, false, true],
            ["scroll", 120, false, false, true],
            ["scrollend", 120, false, false, true],
        ]),
    );
}

#[test]
fn realtime_autofocus_and_animation_scan_share_the_ordered_frame() {
    run_page_vm_large_stack_async_test("realtime-pre-animation", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime-events").unwrap(),
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
            document.body.style.height='3000px';
            document.body.innerHTML='<input id=entry autofocus><div id=moving></div>';
            document.head.appendChild(document.createElement('style')).textContent=
              '@keyframes move {from {left:0px} to {left:10px}} #moving {position:relative;animation:move 1s linear}';
            globalThis.events=[];
            document.addEventListener('DOMContentLoaded',()=>{
              events.push('dcl');Promise.resolve().then(()=>events.push('dcl-microtask'));
            });
            entry.addEventListener('focus',()=>{
              events.push('focus');Promise.resolve().then(()=>events.push('focus-microtask'));
            });
            addEventListener('resize',()=>events.push('resize'));
            document.addEventListener('scroll',()=>{
              events.push('scroll');Promise.resolve().then(()=>events.push('scroll-microtask'));
            });
            document.addEventListener('scrollend',()=>events.push('scrollend'));
            matchMedia('(min-width:900px)').onchange=()=>events.push('media');
            moving.addEventListener('animationstart',()=>events.push('animation'));
            scrollTo(0,20);
            requestAnimationFrame(()=>events.push('raf'));
        "#).unwrap();
        dispatch_main_document_domcontentloaded_for_rendering_test(&mut page)
            .await
            .unwrap();
        page.vm_mut()
            .set_viewport_surface(Some(viewport(960)))
            .unwrap();
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "dcl|dcl-microtask"
        );
        assert!(
            page.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::RenderingUpdate
            )
            .is_none(),
            "no immediate legacy event task may bypass the deadline"
        );
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "dcl|dcl-microtask|focus|focus-microtask|resize|scroll|scroll-microtask|scrollend|media|animation|raf"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        assert!(
            page.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::RenderingUpdate
            )
            .is_none()
        );
    });
}

#[test]
fn realtime_scroll_survives_explicit_document_open_before_and_during_delivery() {
    run_page_vm_large_stack_async_test("realtime-scroll-document-open", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime-scroll-open").unwrap(),
        );
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.vm_mut()
            .eval("document.body.style.height='3000px'")
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        page.vm_mut().eval(r#"
            globalThis.events=[];
            scrollTo(0,100);
            document.open();document.write('<!doctype html><body style="height:3000px"></body>');document.close();
            document.addEventListener('scroll',()=>events.push('scroll:'+scrollY));
            document.addEventListener('scrollend',()=>events.push('scrollend:'+scrollY));
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "scroll:100|scrollend:100"
        );
        page.vm_mut().eval(r#"
            events=[];
            document.addEventListener('scroll',()=>{
              document.open();document.write('<!doctype html><body style="height:3000px"></body>');document.close();
              document.addEventListener('scrollend',()=>events.push('current-end:'+scrollY));
              Promise.resolve().then(()=>events.push('microtask'));
            },{once:true});
            scrollTo(0,200);
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "scroll:200|microtask|current-end:200"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_scroll_skips_detached_child_and_keeps_later_child_phase_runnable() {
    run_page_vm_large_stack_async_test("realtime-scroll-child-owner", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/realtime-scroll-child").unwrap(),
        );
        page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
        page.vm_mut()
            .eval(
                r#"
            document.body.style.height='3000px';
            for (const id of ['retired','current']) {
              const f=document.createElement('iframe');f.id=id;document.body.append(f);
              f.contentDocument.body.style.height='2000px';
            }
            globalThis.events=[];
            globalThis.retiredFrame=document.getElementById('retired');
            globalThis.currentFrame=document.getElementById('current');
            retiredFrame.contentDocument.addEventListener('scroll',()=>events.push('retired'));
            currentFrame.contentDocument.addEventListener('scroll',()=>events.push('current'));
            document.addEventListener('scroll',()=>{
              events.push('root');
              retiredFrame.remove();
              currentFrame.contentWindow.scrollTo(0,40);
            });
        "#,
            )
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        page.vm_mut()
            .eval("retiredFrame.contentWindow.scrollTo(0,20);scrollTo(0,30)")
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("events.join('|')").unwrap(),
            "root|current"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}
