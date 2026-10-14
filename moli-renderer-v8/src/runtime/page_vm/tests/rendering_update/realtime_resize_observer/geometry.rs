use super::*;

fn observe_axis_boxes(page: &mut PageVm, size: &str) {
    page.vm_mut().eval(&format!(r#"
        document.body.innerHTML='<div id=target style="display:block;{size};padding:0;border:2px solid;box-sizing:content-box;writing-mode:vertical-lr"></div>';
        window.samples={{content:[],border:[]}};
        window.record=e=>[
            e.contentRect.width,e.contentRect.height,
            e.contentBoxSize[0].inlineSize,e.contentBoxSize[0].blockSize,
            e.borderBoxSize[0].inlineSize,e.borderBoxSize[0].blockSize
        ];
        window.contentObserver=new ResizeObserver(entries=>samples.content.push(...entries.map(record)));
        window.borderObserver=new ResizeObserver(entries=>samples.border.push(...entries.map(record)));
        contentObserver.observe(target);
        borderObserver.observe(target,{{box:'border-box'}});
    "#)).unwrap();
}

#[test]
fn realtime_ro_physical_dimensions_notify_when_only_logical_axes_change() {
    run_page_vm_large_stack_async_test("realtime-ro-physical-axis-change", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        observe_axis_boxes(&mut page, "width:50px;height:100px");
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("JSON.stringify([samples.content.map(x=>x.slice(0,2)),samples.border.map(x=>x.slice(0,2))])").unwrap(),
            "[[[50,100]],[[50,100]]]"
        );

        page.vm_mut()
            .eval("target.style.writingMode='horizontal-tb'")
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([samples.content.length,samples.border.length])")
                .unwrap(),
            "[2,2]",
            "changing only logical axes is a size change for both observed boxes"
        );
        let expected = r#"{"content":[[50,100,100,50,104,54],[50,100,50,100,54,104]],"border":[[50,100,100,50,104,54],[50,100,50,100,54,104]]}"#;
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(samples)").unwrap(),
            expected
        );
        let builds = page.vm().layout_pass_observability_for_test().1;
        page.vm_mut().eval("requestAnimationFrame(()=>{})").unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(samples)").unwrap(),
            expected
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
    });
}

#[test]
fn realtime_ro_logical_dimensions_do_not_notify_when_only_physical_axes_change() {
    run_page_vm_large_stack_async_test("realtime-ro-logical-axis-change", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        observe_axis_boxes(&mut page, "inline-size:100px;block-size:50px");
        run_opportunity(&mut page, &loader).await.unwrap();
        let expected = r#"{"content":[[50,100,100,50,104,54]],"border":[[50,100,100,50,104,54]]}"#;
        assert_eq!(
            page.vm_mut().eval("JSON.stringify([samples.content.map(x=>x.slice(0,2)),samples.border.map(x=>x.slice(0,2))])").unwrap(),
            "[[[50,100]],[[50,100]]]",
            "the ordinary backend must first resolve these logical CSS dimensions"
        );

        page.vm_mut()
            .eval("target.style.writingMode='horizontal-tb'")
            .unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([samples.content.length,samples.border.length])")
                .unwrap(),
            "[1,1]",
            "unchanged logical sizes must not produce a second notification"
        );
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(samples)").unwrap(),
            expected
        );
        let builds = page.vm().layout_pass_observability_for_test().1;
        // The observer decision above must use the frame's already-current
        // layout. This getter proves the physical box swapped without fixing it.
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([target.offsetWidth,target.offsetHeight])")
                .unwrap(),
            "[104,54]"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
        page.vm_mut().eval("requestAnimationFrame(()=>{})").unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(samples)").unwrap(),
            expected
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
    });
}

#[test]
fn realtime_ro_uses_the_axes_frozen_with_each_published_sample() {
    run_page_vm_large_stack_async_test("realtime-ro-published-axes", || async move {
        for force in [false, true] {
            let loader =
                crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let mut page = realtime_page(&loader);
            page.vm_mut().eval(&format!(r#"
                document.body.innerHTML='<div id=a style="width:100px;height:20px;padding:0;border:0"></div><div id=b style="width:50px;height:100px;padding:0;border:0;writing-mode:vertical-lr"></div>';
                window.order=[];window.errors=[];
                window.onerror=message=>{{errors.push(message);return true}};
                window.first=new ResizeObserver(()=>{{order.push('first');b.style.writingMode='horizontal-tb';if({force})void b.offsetWidth}});
                window.second=new ResizeObserver(entries=>order.push(entries.map(e=>[
                    e.contentRect.width,e.contentRect.height,
                    e.contentBoxSize[0].inlineSize,e.contentBoxSize[0].blockSize,
                    e.borderBoxSize[0].inlineSize,e.borderBoxSize[0].blockSize
                ])));
                first.observe(a);second.observe(b);
            "#)).unwrap();
            run_opportunity(&mut page, &loader).await.unwrap();
            assert_eq!(
                page.vm_mut()
                    .eval("JSON.stringify([order,errors])")
                    .unwrap(),
                if force {
                    r#"[["first",[[50,100,50,100,50,100]]],[]]"#
                } else {
                    r#"[["first",[[50,100,100,50,100,50]]],["ResizeObserver loop completed with undelivered notifications."]]"#
                }
            );
            if !force {
                run_opportunity(&mut page, &loader).await.unwrap();
                assert_eq!(
                    page.vm_mut().eval("JSON.stringify(order)").unwrap(),
                    r#"["first",[[50,100,100,50,100,50]],[[50,100,50,100,50,100]]]"#
                );
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
        }
    });
}
