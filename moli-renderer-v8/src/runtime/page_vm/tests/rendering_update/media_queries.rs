use super::realtime_opportunity::run_opportunity;
use super::*;
use moli_page_types::LayoutPolicy;

const POLICIES: [LayoutPolicy; 3] = [
    LayoutPolicy::Mock,
    LayoutPolicy::Lazy,
    LayoutPolicy::Realtime,
];

fn viewport(width: u32) -> crate::protocol_types::ViewportSurface {
    crate::protocol_types::ViewportSurface {
        inner_width: width,
        inner_height: 600,
        ..Default::default()
    }
}

async fn run_media_update(
    page: &mut PageVm,
    loader: &crate::network::ResourceRequestClient,
) -> anyhow::Result<bool> {
    if page.vm().layout_policy().is_realtime() {
        if page
            .next_rendering_opportunity_deadline_for_test()
            .is_none()
        {
            return Ok(false);
        }
        run_opportunity(page, loader).await?;
        Ok(true)
    } else {
        page.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::RenderingUpdate,
            loader,
        )
        .await
    }
}

async fn media_page(
    loader: &crate::network::ResourceRequestClient,
    policy: LayoutPolicy,
) -> PageVm {
    let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
        loader,
        Url::parse("https://example.com/media-queries").unwrap(),
    );
    page.vm_mut()
        .set_viewport_surface_for_bootstrap(Some(viewport(640)));
    page.vm_mut().set_layout_policy(policy);
    page.vm_mut().eval("'ready'").unwrap();
    for _ in 0..4 {
        if !run_media_update(&mut page, loader).await.unwrap() {
            return page;
        }
    }
    panic!("initial media page must settle: {policy:?}");
}

#[test]
fn media_queries_resize_listener_replacement_preserves_pending_work_in_all_policies() {
    run_page_vm_large_stack_async_test("media-query-resize-listeners", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        for policy in POLICIES {
            let mut page = media_page(&loader, policy).await;
            page.vm_mut()
                .eval(
                    r#"
                globalThis.events = [];
                const first = matchMedia('(min-width:800px)');
                const second = matchMedia('(min-width:800px)');
                const discarded = matchMedia('(min-width:800px)');
                const old = () => events.push('old');
                const replacement = event => events.push('first:' + event.matches);
                second.onchange = old;
                discarded.onchange = old;
                first.addEventListener('change', old);
                addEventListener('resize', () => {
                    events.push('resize:' + first.matches);
                    first.removeEventListener('change', old);
                    first.addEventListener('change', replacement);
                    second.onchange = null;
                    second.onchange = event => events.push('second:' + event.matches);
                    discarded.onchange = null;
                }, {once:true});
            "#,
                )
                .unwrap();
            page.vm_mut()
                .set_viewport_surface(Some(viewport(900)))
                .unwrap();
            page.vm_mut()
                .eval(
                    r#"
                const late = matchMedia('(min-width:800px)');
                late.onchange = event => events.push('late:' + event.matches);
                if (!first.matches) throw new Error('matches must be live before notification');
            "#,
                )
                .unwrap();
            assert!(run_media_update(&mut page, &loader).await.unwrap());
            assert_eq!(
                page.vm_mut().eval("events.join(',')").unwrap(),
                "resize:true,first:true,second:true",
                "{policy:?}"
            );
            let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
            assert_eq!(host.borrow().media_query_count_for_test(), 3, "{policy:?}");
            page.vm_mut()
                .set_viewport_surface(Some(viewport(640)))
                .unwrap();
            assert!(run_media_update(&mut page, &loader).await.unwrap());
            assert_eq!(
                page.vm_mut().eval("events.join(',')").unwrap(),
                "resize:true,first:true,second:true,first:false,second:false,late:false",
                "{policy:?}"
            );
            page.vm_mut().eval("first.removeEventListener('change', replacement);second.onchange=null;late.onchange=null").unwrap();
            assert_eq!(host.borrow().media_query_count_for_test(), 0, "{policy:?}");
            assert!(!run_media_update(&mut page, &loader).await.unwrap());
        }
    });
}

#[test]
fn media_queries_deferred_round_trip_reclaims_unsubscribed_lists_without_events() {
    run_page_vm_large_stack_async_test("media-query-deferred-retirement", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        for policy in [LayoutPolicy::Mock, LayoutPolicy::Lazy] {
            let mut page = media_page(&loader, policy).await;
            page.vm_mut().eval("globalThis.events=[];let query=matchMedia('(min-width:800px)');globalThis.weak=new WeakRef(query);query.onchange=()=>events.push('change')").unwrap();
            page.vm_mut()
                .set_viewport_surface(Some(viewport(900)))
                .unwrap();
            page.vm_mut()
                .set_viewport_surface(Some(viewport(640)))
                .unwrap();
            page.vm_mut()
                .eval("query.onchange=null;query=null")
                .unwrap();
            let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
            assert_eq!(host.borrow().media_query_count_for_test(), 1);
            for _ in 0..2 {
                page.vm_mut()
                    .renderer_document_isolate_ops()
                    .collect_renderer_document_isolate_garbage()
                    .unwrap();
            }
            assert_eq!(
                page.vm_mut()
                    .eval("String(weak.deref() !== undefined)")
                    .unwrap(),
                "true"
            );
            assert!(run_media_update(&mut page, &loader).await.unwrap());
            assert_eq!(
                page.vm_mut().eval("events.join(',')").unwrap(),
                "",
                "{policy:?}"
            );
            assert_eq!(host.borrow().media_query_count_for_test(), 0, "{policy:?}");
            for _ in 0..2 {
                page.vm_mut()
                    .renderer_document_isolate_ops()
                    .collect_renderer_document_isolate_garbage()
                    .unwrap();
            }
            assert_eq!(
                page.vm_mut()
                    .eval("String(weak.deref() === undefined)")
                    .unwrap(),
                "true",
                "{policy:?}"
            );
            assert!(!run_media_update(&mut page, &loader).await.unwrap());
        }
    });
}

#[test]
fn media_queries_preserve_sampling_boundaries_without_consuming_changes_on_read() {
    run_page_vm_large_stack_async_test("media-query-sampling-boundaries", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        for policy in POLICIES {
            let mut page = media_page(&loader, policy).await;
            page.vm_mut().eval("globalThis.events=[];const query=matchMedia('(prefers-color-scheme:dark)');query.onchange=e=>events.push(e.matches)").unwrap();
            page.vm_mut()
                .set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides {
                    color_scheme: Some("dark".to_owned()),
                    ..Default::default()
                });
            assert_eq!(page.vm_mut().eval("String(query.matches)").unwrap(), "true");
            page.vm_mut().set_emulated_media(&Default::default());
            assert_eq!(
                page.vm_mut().eval("String(query.matches)").unwrap(),
                "false"
            );
            assert!(run_media_update(&mut page, &loader).await.unwrap());
            assert_eq!(
                page.vm_mut().eval("JSON.stringify(events)").unwrap(),
                if policy.is_realtime() {
                    "[false]"
                } else {
                    "[]"
                },
                "{policy:?}"
            );
            assert!(!run_media_update(&mut page, &loader).await.unwrap());
        }
    });
}

#[test]
fn media_queries_share_live_events_and_checkpoint_between_notifications() {
    run_page_vm_large_stack_async_test("media-query-event-checkpoints", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        for policy in POLICIES {
            let mut page = media_page(&loader, policy).await;
            page.vm_mut()
                .eval(
                    r#"
                globalThis.events = [];globalThis.saved=null;
                const first = matchMedia('(min-width:800px)');
                const second = matchMedia('(min-width:800px)');
                const snapshot = new MediaQueryListEvent('change', {matches:true});
                first.onchange = event => {
                    if (!saved) saved=event;
                    events.push('first');
                    Promise.resolve().then(() => {
                        events.push('microtask');
                        second.onchange=null;
                    });
                };
                second.onchange = () => events.push('second');
            "#,
                )
                .unwrap();
            page.vm_mut()
                .set_viewport_surface(Some(viewport(900)))
                .unwrap();
            assert!(run_media_update(&mut page, &loader).await.unwrap());
            assert_eq!(
                page.vm_mut().eval("events.join(',')").unwrap(),
                "first,microtask",
                "{policy:?}"
            );
            let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
            assert_eq!(host.borrow().media_query_count_for_test(), 1);
            page.vm_mut()
                .set_viewport_surface(Some(viewport(640)))
                .unwrap();
            assert_eq!(
                page.vm_mut()
                    .eval("JSON.stringify([saved.matches,snapshot.matches])")
                    .unwrap(),
                "[false,true]",
                "{policy:?}"
            );
            assert!(run_media_update(&mut page, &loader).await.unwrap());
            assert_eq!(
                page.vm_mut().eval("events.join(',')").unwrap(),
                "first,microtask,first,microtask",
                "{policy:?}"
            );
            page.vm_mut().eval("first.onchange=null").unwrap();
            assert_eq!(host.borrow().media_query_count_for_test(), 0);
            assert!(!run_media_update(&mut page, &loader).await.unwrap());
        }
    });
}

#[test]
fn media_queries_keep_creation_ids_after_the_registry_becomes_empty() {
    run_page_vm_large_stack_async_test("media-query-stable-identities", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        for policy in POLICIES {
            let mut page = media_page(&loader, policy).await;
            page.vm_mut().eval("globalThis.events=[];const old=matchMedia('(min-width:800px)');old.onchange=()=>{};old.onchange=null").unwrap();
            let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
            assert_eq!(host.borrow().media_query_count_for_test(), 0);
            page.vm_mut().eval("const fresh=matchMedia('(min-width:800px)');fresh.onchange=()=>events.push('fresh');old.onchange=()=>events.push('old')").unwrap();
            page.vm_mut()
                .set_viewport_surface(Some(viewport(900)))
                .unwrap();
            assert!(run_media_update(&mut page, &loader).await.unwrap());
            assert_eq!(
                page.vm_mut().eval("events.join(',')").unwrap(),
                "old,fresh",
                "{policy:?}"
            );
            assert_eq!(host.borrow().media_query_count_for_test(), 2);
        }
    });
}

#[test]
fn media_queries_document_open_transfers_pending_work_and_reclaims_idle_entries() {
    run_page_vm_large_stack_async_test("media-query-document-open", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        for policy in POLICIES {
            let mut page = media_page(&loader, policy).await;
            page.vm_mut()
                .eval(
                    r#"
                globalThis.events=[];
                const first=matchMedia('(min-width:800px)');
                const discarded=matchMedia('(min-width:800px)');
                first.onchange=e=>events.push(e.matches+':'+document.body.textContent);
                discarded.onchange=()=>events.push('discarded');
            "#,
                )
                .unwrap();
            page.vm_mut()
                .set_viewport_surface(Some(viewport(900)))
                .unwrap();
            page.vm_mut()
                .eval(
                    r#"
                discarded.onchange=null;
                document.open();document.write('<!doctype html><body>new</body>');document.close();
            "#,
                )
                .unwrap();
            for _ in 0..4 {
                if !run_media_update(&mut page, &loader).await.unwrap() {
                    break;
                }
            }
            assert_eq!(
                page.vm_mut().eval("events.join(',')").unwrap(),
                "true:new",
                "{policy:?}"
            );
            let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
            assert_eq!(host.borrow().media_query_count_for_test(), 1, "{policy:?}");
            page.vm_mut().eval("first.onchange=null").unwrap();
            assert_eq!(host.borrow().media_query_count_for_test(), 0);
            assert!(!run_media_update(&mut page, &loader).await.unwrap());
        }
    });
}

#[test]
fn media_queries_document_open_in_callback_preserves_the_frozen_batch() {
    run_page_vm_large_stack_async_test("media-query-running-document-open", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        for policy in POLICIES {
            let mut page = media_page(&loader, policy).await;
            page.vm_mut().eval(r#"
                globalThis.events=[];
                const first=matchMedia('(min-width:800px)');
                const second=matchMedia('(min-width:800px)');
                first.onchange=()=>{
                    events.push('first');
                    document.open();document.write('<!doctype html><body>new</body>');document.close();
                };
                second.onchange=e=>events.push('second:'+e.matches+':'+document.body.textContent);
            "#).unwrap();
            page.vm_mut()
                .set_viewport_surface(Some(viewport(900)))
                .unwrap();
            assert!(run_media_update(&mut page, &loader).await.unwrap());
            assert_eq!(
                page.vm_mut().eval("events.join(',')").unwrap(),
                "first,second:true:new",
                "{policy:?}"
            );
            page.vm_mut()
                .eval("first.onchange=null;second.onchange=null")
                .unwrap();
            let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
            assert_eq!(host.borrow().media_query_count_for_test(), 0);
        }
    });
}

#[cfg(feature = "devtools")]
#[test]
fn media_queries_isolated_only_subscriptions_receive_environment_tasks() {
    run_page_vm_large_stack_async_test("media-query-isolated-only", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        for policy in POLICIES {
            let mut page = media_page(&loader, policy).await;
            let isolated = page
                .vm_mut()
                .create_isolated_world("media-only-world", false)
                .unwrap();
            page.vm_mut()
                .exec_in_execution_context(
                    isolated,
                    r#"
                globalThis.query=matchMedia('(prefers-color-scheme:dark)');
                query.onchange=e=>document.body.dataset.isolated=String(e.matches);
            "#,
                )
                .unwrap();
            page.vm_mut()
                .set_emulated_media(&crate::protocol_types::EmulatedMediaOverrides {
                    color_scheme: Some("dark".to_owned()),
                    ..Default::default()
                });
            assert!(run_media_update(&mut page, &loader).await.unwrap());
            assert_eq!(
                page.vm_mut()
                    .eval("document.body.dataset.isolated")
                    .unwrap(),
                "true",
                "{policy:?}"
            );
            page.vm_mut()
                .exec_in_execution_context(isolated, "query.onchange=null")
                .unwrap();
            let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
            assert_eq!(host.borrow().media_query_count_for_test(), 0);
            assert!(!run_media_update(&mut page, &loader).await.unwrap());
        }
    });
}

#[cfg(feature = "devtools")]
#[test]
fn media_queries_freeze_one_document_batch_across_realms_in_all_policies() {
    run_page_vm_large_stack_async_test("media-query-realm-batch", || async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        for policy in POLICIES {
            let mut page = media_page(&loader, policy).await;
            page.vm_mut()
                .eval(
                    r#"
                document.body.dataset.order='';
                const first=matchMedia('(min-width:800px)');
                first.onchange=e=>{
                    document.body.dataset.order+='first:'+e.matches+'|';
                    document.body.dataset.changed='yes';
                    globalThis.late=matchMedia('(min-width:800px)');
                    late.onchange=()=>document.body.dataset.order+='late|';
                };
            "#,
                )
                .unwrap();
            let session =
                moli_page_types::DevToolsSessionKey::from_wire_session_id(Some("media-world"));
            let isolated = page
                .vm_mut()
                .ensure_isolated_world_for_owner(Some(&session), "media-world", false)
                .unwrap();
            page.vm_mut()
                .exec_in_execution_context(
                    isolated,
                    r#"
                globalThis.query=matchMedia('(min-width:800px)');
                query.onchange=function(event){
                    const valid=this===query && event.target===query &&
                        Object.getPrototypeOf(event)===MediaQueryListEvent.prototype;
                    document.body.dataset.order+='isolated:'+event.matches+':'+
                        document.body.dataset.changed+':'+valid+'|';
                };
            "#,
                )
                .unwrap();
            page.vm_mut().eval("const third=matchMedia('(min-width:800px)');third.onchange=e=>document.body.dataset.order+='third:'+e.matches+'|'").unwrap();
            page.vm_mut()
                .set_viewport_surface(Some(viewport(900)))
                .unwrap();
            assert!(run_media_update(&mut page, &loader).await.unwrap());
            assert_eq!(
                page.vm_mut().eval("document.body.dataset.order").unwrap(),
                "first:true|isolated:true:yes:true|third:true|",
                "{policy:?}"
            );
            let host = page.vm().context_host_weak_for_test().upgrade().unwrap();
            assert_eq!(host.borrow().media_query_count_for_test(), 4);
            page.vm_mut()
                .eval("first.onchange=null;third.onchange=null;late.onchange=null")
                .unwrap();
            assert!(
                page.vm_mut()
                    .detach_runtime_inspector_session(Some("media-world"))
            );
            assert_eq!(host.borrow().media_query_count_for_test(), 0, "{policy:?}");
            assert!(!run_media_update(&mut page, &loader).await.unwrap());
        }
    });
}
