use super::realtime_opportunity::run_opportunity;
use super::*;
use moli_page_types::LayoutPolicy;

mod lifecycle;
mod ownership;

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
