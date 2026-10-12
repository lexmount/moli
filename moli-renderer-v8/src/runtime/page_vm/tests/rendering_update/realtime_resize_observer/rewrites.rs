use super::*;

#[test]
fn realtime_ro_broadcast_after_document_open_distinguishes_removed_boxes_and_new_layout() {
    run_page_vm_large_stack_async_test("realtime-ro-broadcast-rewrite", || async move {
        for force in [false, true] {
            let loader =
                crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let mut page = realtime_page(&loader);
            page.vm_mut().eval(&format!(r#"
                document.body.innerHTML='<div id=a style="width:100px;height:20px"></div><div id=b style="width:100px;height:20px"></div>';
                window.savedA=a;window.savedB=b;window.log=[];window.rewritten=false;
                window.first=new ResizeObserver(entries=>{{
                    log.push(['first',entries[0].contentRect.width]);
                    if(!rewritten){{
                        rewritten=true;
                        document.open();document.write('<!doctype html><body></body>');document.close();
                        document.body.append(savedB);savedB.style.width='200px';
                        if({force})void savedB.offsetWidth;
                    }}
                }});
                window.second=new ResizeObserver(entries=>log.push(['second',entries[0].contentRect.width]));
                first.observe(savedA);second.observe(savedB);
            "#)).unwrap();
            run_opportunity(&mut page, &loader).await.unwrap();
            assert_eq!(
                page.vm_mut().eval("JSON.stringify(log)").unwrap(),
                if force {
                    r#"[["first",100],["second",200]]"#
                } else {
                    r#"[["first",100],["second",0]]"#
                }
            );
            let builds = page.vm().layout_pass_observability_for_test().1;
            assert_eq!(
                builds, 2,
                "one initial build and one rewritten screen build"
            );
            run_opportunity(&mut page, &loader).await.unwrap();
            assert_eq!(
                page.vm_mut().eval("JSON.stringify(log)").unwrap(),
                if force {
                    r#"[["first",100],["second",200],["first",0]]"#
                } else {
                    r#"[["first",100],["second",0],["first",0],["second",200]]"#
                }
            );
            assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );
        }
    });
}

#[test]
fn realtime_ro_broadcast_distinguishes_removal_from_state_preserving_moves() {
    run_page_vm_large_stack_async_test("realtime-ro-broadcast-moves", || async move {
        for movement in [
            "style",
            "remove-append",
            "append",
            "ancestor",
            "shadow",
            "moveBefore",
        ] {
            for force in [false, true] {
                let loader =
                    crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
                let mut page = realtime_page(&loader);
                page.vm_mut().eval(&format!(r#"
                    document.body.innerHTML='<div id=a style="width:100px;height:20px"></div><div id=wrapper><div id=b style="width:100px;height:20px"></div></div><div id=destination></div>';
                    const a=document.getElementById('a'),b=document.getElementById('b');
                    const wrapper=document.getElementById('wrapper'),destination=document.getElementById('destination');
                    if('{movement}'==='shadow')wrapper.attachShadow({{mode:'open'}}).append(b);
                    window.log=[];window.changed=false;
                    window.first=new ResizeObserver(()=>{{
                        log.push('first');
                        if(!changed){{
                            changed=true;
                            if('{movement}'==='remove-append'){{b.remove();destination.append(b)}}
                            else if('{movement}'==='append')destination.append(b);
                            else if('{movement}'==='ancestor'||'{movement}'==='shadow'){{wrapper.remove();document.body.append(wrapper)}}
                            else if('{movement}'==='moveBefore')destination.moveBefore(b,null);
                            b.style.width='200px';
                            if({force})void b.offsetWidth;
                        }}
                    }});
                    window.second=new ResizeObserver(entries=>log.push(entries[0].contentRect.width));
                    first.observe(a);second.observe(b);
                "#)).unwrap();
                run_opportunity(&mut page, &loader).await.unwrap();
                assert_eq!(
                    page.vm_mut().eval("JSON.stringify(log)").unwrap(),
                    if force {
                        r#"["first",200]"#
                    } else if matches!(movement, "style" | "moveBefore") {
                        r#"["first",100,200]"#
                    } else {
                        r#"["first",0,200]"#
                    },
                    "move={movement}, force={force}"
                );
                assert_eq!(page.vm().layout_pass_observability_for_test().1, 2);
                assert!(
                    page.next_rendering_opportunity_deadline_for_test()
                        .is_none()
                );
            }
        }
    });
}

#[test]
fn realtime_ro_sampling_tracks_removal_after_a_callback_publishes_layout() {
    run_page_vm_large_stack_async_test("realtime-ro-removal-after-getter", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let mut page = realtime_page(&loader);
        page.vm_mut().eval(r#"
            document.body.innerHTML='<div id=a style="width:100px;height:20px"></div><div id=wrapper><div id=b style="width:100px;height:20px"></div></div>';
            window.log=[];
            window.first=new ResizeObserver(()=>{
                const target=b;target.remove();wrapper.append(target);target.style.width='200px';
                log.push(target.offsetWidth);
                target.remove();wrapper.append(target);target.style.width='300px';
            });
            window.second=new ResizeObserver(entries=>log.push(entries[0].contentRect.width));
            first.observe(a);second.observe(b);
        "#).unwrap();
        run_opportunity(&mut page, &loader).await.unwrap();
        assert_eq!(
            page.vm_mut().eval("JSON.stringify(log)").unwrap(),
            "[200,0,300]"
        );
        assert_eq!(page.vm().layout_pass_observability_for_test().1, 3);
        assert!(
            page.next_rendering_opportunity_deadline_for_test()
                .is_none()
        );
    });
}

#[test]
fn realtime_ro_child_document_open_does_not_discard_parent_samples() {
    run_page_vm_large_stack_async_test("realtime-ro-child-rewrite", || async move {
        for force in [false, true] {
            let loader =
                crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
            let mut page = realtime_page(&loader);
            page.vm_mut().eval(
                "document.body.innerHTML='<div id=a style=width:100px;height:20px></div><div id=unaffected style=width:80px;height:20px></div><iframe id=frame></iframe>'",
            ).unwrap();
            materialize_child_realm_through_page_turn_for_test(&mut page, "frame").unwrap();
            // Establish the screen viewport before observing a rewrite. The
            // rewrite must preserve this sample and not invent a new resize.
            page.vm_mut()
                .eval("frame.contentWindow.innerWidth")
                .unwrap();
            run_opportunity(&mut page, &loader).await.unwrap();
            page.vm_mut().eval(&format!(r#"
                const child=frame.contentWindow;
                child.document.body.innerHTML='<div id=b style="width:100px;height:20px"></div>';
                const b=child.document.getElementById('b');
                window.log=[];window.rewritten=false;
                window.first=new ResizeObserver(entries=>{{
                    log.push(['first',entries[0].contentRect.width]);
                    if(!rewritten){{
                        rewritten=true;
                        child.document.open();child.document.write('<!doctype html><body></body>');child.document.close();
                        child.document.body.append(b);b.style.width='200px';
                        if({force})void b.offsetWidth;
                    }}
                }});
                window.second=new ResizeObserver(entries=>log.push(['child',entries[0].contentRect.width]));
                window.third=new ResizeObserver(entries=>log.push(['parent',entries[0].contentRect.width]));
                first.observe(a);second.observe(b);third.observe(unaffected);
            "#)).unwrap();
            let before = page.vm().layout_pass_observability_for_test().1;
            run_opportunity(&mut page, &loader).await.unwrap();
            assert_eq!(
                page.vm_mut().eval("JSON.stringify(log)").unwrap(),
                if force {
                    r#"[["first",100],["child",200],["parent",80]]"#
                } else {
                    r#"[["first",100],["child",0],["parent",80]]"#
                }
            );
            let builds = page.vm().layout_pass_observability_for_test().1;
            assert_eq!(builds - before, 2);
            if !force {
                run_opportunity(&mut page, &loader).await.unwrap();
                assert_eq!(
                    page.vm_mut().eval("JSON.stringify(log)").unwrap(),
                    r#"[["first",100],["child",0],["parent",80],["child",200]]"#
                );
            }
            assert_eq!(page.vm().layout_pass_observability_for_test().1, builds);
            assert!(
                page.next_rendering_opportunity_deadline_for_test()
                    .is_none()
            );
        }
    });
}
