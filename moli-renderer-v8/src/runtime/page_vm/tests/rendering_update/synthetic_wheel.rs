#![cfg(feature = "devtools")]

use super::*;
use crate::runtime::{
    RendererDevToolsPageCommand, RendererPageCommand, RendererSyntheticWheelGesture,
};
use moli_page_types::LayoutPolicy;

fn surface(width: u32) -> crate::protocol_types::ViewportSurface {
    crate::protocol_types::ViewportSurface {
        inner_width: width,
        inner_height: 400,
        device_pixel_ratio: 1.0,
        ..Default::default()
    }
}

fn wheel(
    page: &mut PageVm,
    gesture: &RendererSyntheticWheelGesture,
    x: f64,
    dx: f64,
    dy: f64,
) -> anyhow::Result<()> {
    page.dispatch_renderer_page_command(RendererPageCommand::DevTools(
        RendererDevToolsPageCommand::DispatchSyntheticWheel {
            x,
            y: 40.0,
            delta_x: dx,
            delta_y: dy,
            cancelable: true,
            intercept_drags: false,
            gesture: gesture.clone(),
        },
    ))?;
    Ok(())
}

fn install(page: &mut PageVm) {
    page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
    page.set_viewport_surface(Some(surface(600))).unwrap();
    page.vm_mut().eval(r#"
        document.body.style.cssText='margin:0;height:2400px';
        document.body.innerHTML='<div id="a" style="width:160px;height:120px;overflow:auto"><div style="width:100px;height:1800px">content</div></div><iframe id="f" style="position:absolute;left:300px;top:0;width:200px;height:180px;border:0"></iframe>';
        window.a=document.getElementById('a'); window.events=[];
        document.addEventListener('wheel',e=>events.push(e.deltaY));
    "#).unwrap();
}

#[test]
fn synthetic_wheel_removed_scroller_stays_invalid_after_reinsertion() {
    run_page_vm_large_stack_async_test("synthetic-wheel-reinsert", || async move {
        for (setup, detach, reinsert, gap) in [
            ("", "a.remove()", "document.body.prepend(a)", false),
            ("", "a.remove()", "document.body.prepend(a)", true),
            (
                "window.wrap=document.createElement('div');a.before(wrap);wrap.append(a)",
                "wrap.remove()",
                "document.body.prepend(wrap)",
                false,
            ),
            ("", "document.body.moveBefore(a,null)", "", false),
        ] {
            let loader =
                crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://example.com/wheel").unwrap(),
            );
            install(&mut page);
            page.vm_mut().eval(setup).unwrap();
            let gesture = RendererSyntheticWheelGesture::default();
            wheel(&mut page, &gesture, 40.0, 0.0, 50.0).unwrap();
            assert_eq!(page.vm_mut().eval("String(a.scrollTop)").unwrap(), "50");
            page.vm_mut().eval(detach).unwrap();
            if gap {
                wheel(&mut page, &gesture, 40.0, 0.0, 25.0).unwrap();
            }
            page.vm_mut().eval(reinsert).unwrap();
            wheel(&mut page, &gesture, 40.0, 0.0, 100.0).unwrap();
            assert_eq!(
                page.vm_mut()
                    .eval("JSON.stringify([a.scrollTop,scrollY])")
                    .unwrap(),
                "[50,0]",
                "{detach}, gap={gap}"
            );
            wheel(&mut page, &gesture.next_sequence(), 40.0, 0.0, 40.0).unwrap();
            assert_eq!(
                page.vm_mut().eval("String(a.scrollTop)").unwrap(),
                "90",
                "new gesture must select a fresh scrollbox"
            );
        }
    });
}

#[test]
fn synthetic_wheel_adopted_scroller_does_not_follow_old_gesture() {
    run_page_vm_large_stack_async_test("synthetic-wheel-adoption", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/wheel").unwrap(),
        );
        install(&mut page);
        let gesture = RendererSyntheticWheelGesture::default();
        wheel(&mut page, &gesture, 40.0, 0.0, 50.0).unwrap();
        page.vm_mut()
            .eval("f.contentDocument.body.append(a)")
            .unwrap();
        wheel(&mut page, &gesture, 40.0, 0.0, 100.0).unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([a.scrollTop,scrollY])")
                .unwrap(),
            "[50,0]"
        );
    });
}

#[test]
fn synthetic_wheel_uses_one_scroller_for_both_axes_and_keeps_viewport_at_boundary() {
    run_page_vm_large_stack_async_test("synthetic-wheel-vector", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/wheel").unwrap(),
        );
        install(&mut page);
        page.vm_mut().eval(r#"
          document.body.innerHTML='<div id=outer style="width:160px;height:150px;overflow-x:auto;overflow-y:hidden"><div id=inner style="width:600px;height:140px;overflow-y:auto;overflow-x:hidden"><div style="height:1800px">text</div></div></div>';
        "#).unwrap();
        wheel(
            &mut page,
            &RendererSyntheticWheelGesture::default(),
            40.0,
            40.0,
            50.0,
        )
        .unwrap();
        assert_eq!(page.vm_mut().eval("JSON.stringify([outer.scrollLeft,outer.scrollTop,inner.scrollLeft,inner.scrollTop,scrollY])").unwrap(),"[0,0,0,50,0]");
        let gesture = RendererSyntheticWheelGesture::default();
        wheel(&mut page, &gesture, 400.0, 0.0, -50.0).unwrap();
        assert_eq!(page.vm_mut().eval("String(scrollY)").unwrap(), "0");
        wheel(&mut page, &gesture, 400.0, 0.0, 100.0).unwrap();
        assert_eq!(page.vm_mut().eval("String(scrollY)").unwrap(), "100");
    });
}

#[test]
fn synthetic_wheel_skips_resized_outside_updates_and_rejects_initial_outside_anchor() {
    run_page_vm_large_stack_async_test("synthetic-wheel-resize", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/wheel").unwrap(),
        );
        install(&mut page);
        let gesture = RendererSyntheticWheelGesture::default();
        wheel(&mut page, &gesture, 500.0, 0.0, 50.0).unwrap();
        page.set_viewport_surface(Some(surface(300))).unwrap();
        wheel(&mut page, &gesture, 500.0, 0.0, 100.0).unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([scrollY,events])")
                .unwrap(),
            "[50,[50]]"
        );
        let error = wheel(
            &mut page,
            &RendererSyntheticWheelGesture::default(),
            500.0,
            0.0,
            100.0,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("Position out of bounds"),
            "{error}"
        );
        let repeat = gesture.next_sequence();
        wheel(&mut page, &repeat, 500.0, 0.0, 75.0).unwrap();
        page.set_viewport_surface(Some(surface(600))).unwrap();
        wheel(&mut page, &repeat, 500.0, 0.0, 25.0).unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([scrollY,events])")
                .unwrap(),
            "[75,[50,25]]"
        );
    });
}

#[test]
fn synthetic_wheel_survives_unrelated_mutations() {
    run_page_vm_large_stack_async_test("synthetic-wheel-stable-box", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/wheel").unwrap(),
        );
        install(&mut page);
        let gesture = RendererSyntheticWheelGesture::default();
        wheel(&mut page, &gesture, 40.0, 0.0, 50.0).unwrap();
        page.vm_mut().eval("const unrelated=document.createElement('div');document.body.append(unrelated);unrelated.remove()").unwrap();
        wheel(&mut page, &gesture, 40.0, 0.0, 25.0).unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([a.scrollTop,scrollY])")
                .unwrap(),
            "[75,0]"
        );
    });
}
