use super::realtime_opportunity::run_opportunity;
use super::*;
use moli_page_types::LayoutPolicy;

mod geometry;
mod lifecycle;
mod ownership;
mod rewrites;

fn realtime_page(loader: &crate::network::ResourceRequestClient) -> PageVm {
    let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
        loader,
        Url::parse("https://example.com/realtime-resize-observer").unwrap(),
    );
    page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
    page
}

#[test]
fn realtime_ro_initial_delivery_follows_raf_and_becomes_idle_without_capture() {
    run_page_vm_large_stack_async_test("realtime-ro-initial", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval("document.body.innerHTML='<div id=target style=width:100px;height:20px></div>';window.order=[]").unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        let builds = page.vm().layout_pass_observability_for_test().1;
        page.vm_mut().eval(r#"
            window.observer=new ResizeObserver(entries=>order.push('resize:'+entries[0].contentRect.width));
            observer.observe(target);
            requestAnimationFrame(()=>{order.push('raf');Promise.resolve().then(()=>order.push('microtask'))});
        "#).unwrap();
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(order)").unwrap(),
            "[]",
            "Realtime RO delivery is not a host microtask or a synchronous getter"
        );
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("order.join('|')").unwrap(),
            "raf|microtask|resize:100"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
        assert_eq!(
            page.vm()
                .layout_pass_observability_for_test()
                .3
                .unwrap()
                .paint_operation_count,
            0
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
        assert!(page.vm().next_timeout_deadline().is_none());
        page.vm_mut()
            .eval("requestAnimationFrame(()=>order.push('clean-raf'))")
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("order.join('|')").unwrap(),
            "raf|microtask|resize:100|clean-raf"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_frames_admit_cssom_fonts_after_observer_callbacks_without_capture() {
    assert_frame_cssom_font_admission(true);
}

#[test]
fn realtime_frame_cssom_fonts_preserve_the_disabled_resource_policy() {
    assert_frame_cssom_font_admission(false);
}

fn assert_frame_cssom_font_admission(fonts_enabled: bool) {
    run_page_vm_large_stack_async_test("realtime-cssom-font-admission", move || async move {
        use base64::Engine;

        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        loader.set_optional_resource_fetch_mask(if fonts_enabled {
            crate::protocol_types::OptionalResourceFetchMask::FONT
        } else {
            crate::protocol_types::OptionalResourceFetchMask::NONE
        });
        let mut page = realtime_page(&loader);
        let font = base64::engine::general_purpose::STANDARD.encode(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../moli-layout/tests/fixtures/moli-ahem.woff2"
        )));
        let markup = "<!doctype html><style>@font-face{font-family:ObserverFont;src:local(MissingObserverFont)}</style><span id=target style='display:inline-block;font:20px ObserverFont,serif'>iiii</span>";
        page.vm_mut().eval(&format!(r#"
            document.head.innerHTML='<style>@font-face{{font-family:ObserverFont;src:local(MissingObserverFont)}}</style>';
            document.body.innerHTML="<span id=target style='display:inline-block;font:20px ObserverFont,serif'>iiii</span><iframe id=frame></iframe>";
            const child=frame.contentDocument;
            child.open();child.write({markup});child.close();
            window.samples=[[],[]];window.observers=[];
            [document,child].forEach((doc,index)=>{{
                const ro=new doc.defaultView.ResizeObserver(entries=>{{
                    samples[index].push(entries[0].contentRect.width);
                    if(samples[index].length===1){{
                        doc.styleSheets[0].cssRules[0].style.src='url(data:font/woff2;base64,{font})';
                    }}
                }});
                observers.push(ro);ro.observe(doc.getElementById('target'));
            }});
        "#,markup=serde_json::to_string(markup).unwrap())).unwrap();
        // Advance bounded native frame opportunities, without screenshots,
        // geometry queries or a manual font-reconciliation helper.
        for _ in 0..4 {
            if page
                .next_rendering_opportunity_deadline_for_test()
                .is_none()
            {
                break;
            }
            run_opportunity(&mut page, &loader).await.unwrap();
        }
        let samples: Vec<Vec<f64>> =
            serde_json::from_str(&page.vm_mut().eval("JSON.stringify(samples)").unwrap()).unwrap();
        for sample in &samples {
            assert_eq!(
                sample.len(),
                if fonts_enabled { 2 } else { 1 },
                "font policy {fonts_enabled}: {samples:?}"
            );
            assert_ne!(sample[0], 48.0);
            if fonts_enabled {
                assert_eq!(sample[1], 48.0);
            }
        }
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
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
