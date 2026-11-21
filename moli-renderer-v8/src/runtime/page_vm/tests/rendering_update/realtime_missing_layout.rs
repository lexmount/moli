use super::realtime_opportunity::run_opportunity;
use super::*;
use moli_page_types::LayoutPolicy;

fn realtime_page(loader: &crate::network::ResourceRequestClient) -> PageVm {
    let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
        loader,
        Url::parse("https://missing-layout.test/").unwrap(),
    );
    page.layout_configuration.policy = LayoutPolicy::Realtime;
    page.vm_mut().set_layout_policy(LayoutPolicy::Realtime);
    page
}

fn install_child(page: &mut PageVm) {
    page.vm_mut().eval(
        "document.body.innerHTML='<div id=parentTarget style=width:40px;height:20px></div><iframe id=frame style=width:120px;height:80px;border:0></iframe>'",
    ).unwrap();
    materialize_child_realm_through_page_turn_for_test(page, "frame").unwrap();
    page.vm_mut().eval(
        "const child=frame.contentDocument;child.body.style.margin='0';child.body.innerHTML='<div id=target style=width:100px;height:20px></div>';window.childTarget=child.getElementById('target')",
    ).unwrap();
}

#[test]
fn realtime_ro_missing_frame_layout_reports_zero_and_preserves_parent_frames() {
    run_page_vm_large_stack_async_test("realtime-ro-missing-layout", || async move {
        for hidden in [true, false] {
            let loader =
                crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let mut page = realtime_page(&loader);
            install_child(&mut page);
            page.vm_mut().eval(&format!(r#"
                window.ticks=0;window.childSizes=[];window.parentSizes=[];
                window.observer=new ResizeObserver(entries=>{{
                    for(const entry of entries){{
                        (entry.target===childTarget?childSizes:parentSizes).push(entry.contentRect.width);
                    }}
                }});
                observer.observe(childTarget);observer.observe(parentTarget);
                requestAnimationFrame(function tick(){{
                    ticks++;
                    if(ticks===2){{if({hidden})frame.style.display='none';else frame.remove()}}
                    if(ticks===3)parentTarget.style.width='60px';
                    if(ticks<4)requestAnimationFrame(tick);
                }});
            "#)).unwrap();
            for _ in 0..4 {
                run_opportunity(&mut page, &loader).await.unwrap();
            }
            assert_eq!(
                page.vm_mut()
                    .eval("JSON.stringify([ticks,childSizes,parentSizes])")
                    .unwrap(),
                "[4,[100,0],[40,60]]",
                "hidden={hidden}"
            );
            if hidden {
                page.vm_mut().eval("frame.style.display='block'").unwrap();
                run_opportunity(&mut page, &loader).await.unwrap();
                assert_eq!(
                    page.vm_mut().eval("JSON.stringify(childSizes)").unwrap(),
                    "[100,0,100]"
                );
            }
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

#[test]
fn realtime_io_missing_frame_layout_reports_nonintersection_and_preserves_frames() {
    run_page_vm_large_stack_async_test("realtime-io-missing-layout", || async move {
        for hidden in [true, false] {
            let loader =
                crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let mut page = realtime_page(&loader);
            install_child(&mut page);
            page.vm_mut().eval(&format!(r#"
                window.ticks=0;window.childEntries=[];window.parentEntries=[];
                window.observer=new IntersectionObserver(entries=>{{
                    for(const entry of entries){{
                        (entry.target===childTarget?childEntries:parentEntries).push(entry.isIntersecting);
                    }}
                }});
                observer.observe(childTarget);observer.observe(parentTarget);
                requestAnimationFrame(function tick(){{
                    ticks++;
                    if(ticks===2){{if({hidden})frame.style.display='none';else frame.remove()}}
                    if(ticks<4)requestAnimationFrame(tick);
                }});
            "#)).unwrap();
            for _ in 0..4 {
                run_opportunity(&mut page, &loader).await.unwrap();
                page.run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::IntersectionObserver,
                    &loader,
                )
                .await
                .unwrap();
            }
            assert_eq!(
                page.vm_mut()
                    .eval("JSON.stringify([ticks,childEntries,parentEntries])")
                    .unwrap(),
                "[4,[true,false],[true]]",
                "hidden={hidden}"
            );
            if hidden {
                page.vm_mut().eval("frame.style.display='block'").unwrap();
                run_opportunity(&mut page, &loader).await.unwrap();
                assert!(
                    page.run_exact_selected_page_task_for_test(
                        PageSelectedTaskTestSelector::IntersectionObserver,
                        &loader,
                    )
                    .await
                    .unwrap()
                );
                assert_eq!(
                    page.vm_mut().eval("JSON.stringify(childEntries)").unwrap(),
                    "[true,false,true]"
                );
            }
        }
    });
}

#[test]
fn realtime_io_inert_document_does_not_block_connected_targets_or_animation() {
    run_page_vm_large_stack_async_test("realtime-io-inert-document", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut()
            .eval(
                r#"
            document.body.innerHTML='<div id=target style="width:40px;height:20px"></div>';
            window.other=document.implementation.createHTMLDocument('');
            window.entries=[];window.ticks=0;
            window.observer=new IntersectionObserver(es=>entries.push(...es.map(e=>[
                e.target===target,e.isIntersecting,e.boundingClientRect.width
            ])));
            observer.observe(target);observer.observe(other.body);
            requestAnimationFrame(function tick(){if(++ticks<3)requestAnimationFrame(tick)});
        "#,
            )
            .unwrap();
        for _ in 0..3 {
            run_opportunity(&mut page, &loader).await.unwrap();
            page.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::IntersectionObserver,
                &loader,
            )
            .await
            .unwrap();
        }
        assert_eq!(
            page.vm_mut()
                .eval("JSON.stringify([ticks,entries])")
                .unwrap(),
            "[3,[[true,true,40],[false,false,0]]]"
        );
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}
