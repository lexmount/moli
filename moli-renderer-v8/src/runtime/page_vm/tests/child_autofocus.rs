use super::*;

#[tokio::test(flavor = "current_thread")]
async fn child_autofocus_shares_top_document_connection_order_and_processed_state() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        for move_to_parent in [false, true] {
            let (mut page_vm, _queue, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://example.com/child-autofocus-order")?,
            );
            super::rendering_update::dispatch_main_document_domcontentloaded_for_rendering_test(
                &mut page_vm,
            ).await?;
            page_vm.vm_mut().eval("document.body.innerHTML='<iframe id=child></iframe>'")?;
            materialize_child_realm_through_page_turn_for_test(&mut page_vm, "child")?;
            page_vm.vm_mut().eval(r#"
const frame=document.getElementById('child');
const child=frame.contentWindow;
const ChildFocusEvent=child.FocusEvent;
child.FocusEvent=()=>{throw new Error('native focus must not call the page constructor');};
const input=child.document.createElement('input');
input.id='first'; input.autofocus=true;
globalThis.focusEvents=[];
input.addEventListener('focus',event=>focusEvents.push(
  [event instanceof ChildFocusEvent,event instanceof FocusEvent,event.target===input,event.view===child,event.isTrusted].join(':')));
child.document.body.append(input);
document.body.insertAdjacentHTML('beforeend','<input id=second autofocus>');
'inserted'
"#)?;
            if move_to_parent {
                page_vm.vm_mut().eval("document.body.prepend(input)")?;
            }
            assert_eq!(page_vm.vm_mut().eval("focusEvents.length")?, "0");
            assert!(page_vm.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::RenderingUpdate, &loader,
            ).await?);
            if move_to_parent {
                assert_eq!(page_vm.vm_mut().eval("document.activeElement.id")?, "second");
                assert_eq!(page_vm.vm_mut().eval("focusEvents.length")?, "0");
            } else {
                assert_eq!(page_vm.vm_mut().eval("document.activeElement===frame")?, "true");
                assert_eq!(page_vm.vm_mut().eval("child.document.activeElement===input")?, "true");
                assert_eq!(page_vm.vm_mut().eval("focusEvents.join('|')")?, "true:false:true:true:true");
            }
            page_vm.vm_mut().eval(
                "input.blur(); document.getElementById('second').blur(); \
                 child.document.body.insertAdjacentHTML('beforeend','<input id=late autofocus>')"
            )?;
            assert!(page_vm.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::RenderingUpdate,
            ).is_none(), "a child must share the top Document's completed autofocus decision");
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("child autofocus order must be shared with its top Document");
}

#[tokio::test(flavor = "current_thread")]
async fn child_autofocus_discards_retired_documents_and_checks_ancestor_fragments() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        for mutation in [
            "frame.remove()",
            "child.document.open()",
            "frame.srcdoc='<body><input id=successor>'",
            "child.location.hash='anchor'",
            "document.body.insertAdjacentHTML('beforeend','<div id=anchor></div>'); location.hash='anchor'",
        ] {
            let (mut page_vm, _queue, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader, Url::parse("https://example.com/child-autofocus-revalidation")?,
            );
            super::rendering_update::dispatch_main_document_domcontentloaded_for_rendering_test(
                &mut page_vm,
            ).await?;
            page_vm.vm_mut().eval("document.body.innerHTML='<iframe id=child></iframe>'")?;
            materialize_child_realm_through_page_turn_for_test(&mut page_vm, "child")?;
            page_vm.vm_mut().eval(r#"
const frame=document.getElementById('child');
const child=frame.contentWindow;
child.document.body.innerHTML='<div id=anchor></div><input id=candidate autofocus>';
globalThis.focusEvents=0;
child.document.addEventListener('focus',()=>focusEvents++,true);
'inserted'
"#)?;
            let claimed=page_vm.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::RenderingUpdate,
            ).expect("child insertion must publish autofocus");
            page_vm.vm_mut().eval(mutation)?;
            if mutation.starts_with("frame.srcdoc") {
                run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
                    &mut page_vm,
                    ChildFrameSemanticTurnKind::NavigationCommit,
                    "retire the child autofocus candidate's Document",
                ).await;
            }
            page_vm.run_claimed_selected_page_task_for_test(claimed,&loader).await?;
            assert_eq!(page_vm.vm_mut().eval("String(focusEvents)")?, "0", "{mutation}");
            assert_eq!(page_vm.vm_mut().eval("document.activeElement===document.body")?, "true", "{mutation}");
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("child autofocus must revalidate the candidate and its ancestor Documents");
}

#[tokio::test(flavor = "current_thread")]
async fn child_focus_events_use_intrinsic_constructor_and_ignore_inherited_init_options() {
    run_page_vm_async_test(async move {
        let loader=crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page_vm,_queue,_wake)=page_vm_with_bound_task_sources_and_owner_wake(
            &loader,Url::parse("https://example.com/child-focus-event-realms")?,
        );
        page_vm.vm_mut().eval("document.body.innerHTML='<iframe id=child></iframe>'")?;
        materialize_child_realm_through_page_turn_for_test(&mut page_vm,"child")?;
        assert_eq!(page_vm.vm_mut().eval(r#"
const child=document.getElementById('child').contentWindow;
const C=child.FocusEvent;
let constructorCalls=0, inheritedReads=0;
child.FocusEvent=()=>{constructorCalls++;throw new Error('page constructor');};
child.Object.defineProperty(child.Object.prototype,'detail',{
  configurable:true,get(){inheritedReads++;return 123;}
});
child.document.body.innerHTML='<input id=a><input id=b>';
const a=child.document.getElementById('a'),b=child.document.getElementById('b');
const events=[];
for(const target of [a,b])for(const type of ['focus','focusin','blur','focusout'])
  target.addEventListener(type,e=>events.push([
    type,e.target.id,e.relatedTarget?.id??'null',e instanceof C,e.view===child,e.isTrusted,e.detail
  ].join(':')));
a.focus();b.focus();b.blur();
[constructorCalls,inheritedReads,events.join('|')].join(';')
"#)?,
            "0;0;focus:a:null:true:true:true:0|focusin:a:null:true:true:true:0|blur:a:b:true:true:true:0|focusout:a:b:true:true:true:0|focus:b:a:true:true:true:0|focusin:b:a:true:true:true:0|blur:b:null:true:true:true:0|focusout:b:null:true:true:true:0"
        );
        Ok::<_, anyhow::Error>(())
    }).await.expect("native focus events must be owned by their target Document's realm");
}

#[tokio::test(flavor = "current_thread")]
async fn child_autofocus_obeys_sandbox_and_inherited_permissions_policy() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        for attributes in [
            "sandbox='allow-same-origin'",
            "allow=\"focus-without-user-activation 'none'\"",
        ] {
            let (mut page_vm, _queue, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader, Url::parse("https://example.com/child-autofocus-policy")?,
            );
            super::rendering_update::dispatch_main_document_domcontentloaded_for_rendering_test(
                &mut page_vm,
            ).await?;
            let markup=format!("<iframe id=child {attributes}></iframe>");
            page_vm.vm_mut().eval(&format!("document.body.innerHTML={markup:?}"))?;
            materialize_child_realm_through_page_turn_for_test(&mut page_vm,"child")?;
            page_vm.vm_mut().eval(
                "document.getElementById('child').contentDocument.body.innerHTML='<input autofocus>'"
            )?;
            assert!(page_vm.claim_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::RenderingUpdate,
            ).is_none(), "{attributes}: denied child insertion must not queue candidates");
            page_vm.vm_mut().eval("document.body.insertAdjacentHTML('beforeend','<input id=allowed autofocus>')")?;
            assert!(page_vm.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::RenderingUpdate,&loader,
            ).await?);
            assert_eq!(page_vm.vm_mut().eval("document.activeElement.id")?,"allowed");
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("child autofocus must use its own inherited policy");
}

#[tokio::test(flavor = "current_thread")]
async fn child_autofocus_preserves_candidates_until_its_stylesheet_completion() {
    run_page_vm_async_test(async move {
        let (base_url,server)=spawn_path_response_http_server(vec![(
            "/autofocus.css","HTTP/1.1 200 OK","#hidden { display: none }".to_owned(),Duration::ZERO,
        )]).await;
        let loader=crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page_vm,mut queue,mut wake)=page_vm_with_bound_task_sources_and_owner_wake(
            &loader,Url::parse(&format!("{base_url}/parent"))?,
        );
        super::rendering_update::dispatch_main_document_domcontentloaded_for_rendering_test(
            &mut page_vm,
        ).await?;
        let markup=format!("<link rel=stylesheet href='{base_url}/autofocus.css'><body><input id=hidden autofocus><input id=shown autofocus>");
        page_vm.vm_mut().eval(&format!(
            "globalThis.frame=document.createElement('iframe'); frame.id='child'; frame.srcdoc={markup:?}; document.body.append(frame); 'inserted'"
        ))?;
        run_expected_child_frame_task_source_after_realm_prerequisite_for_wait(
            &mut page_vm,ChildFrameSemanticTurnKind::NavigationCommit,"child autofocus parser",
        ).await;
        page_vm.vm_mut().eval("document.body.insertAdjacentHTML('beforeend','<input id=parentCandidate autofocus>')")?;
        assert!(page_vm.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::RenderingUpdate,&loader,
        ).await?);
        assert_eq!(page_vm.vm_mut().eval("document.activeElement===document.body")?,"true",
            "an unresolved child stylesheet preserves queue order before the parent's candidate");
        super::child_document_completion::wait_for_page_resource_completion(
            &mut queue,&mut wake,"child autofocus stylesheet",
        ).await;
        assert!(page_vm.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::ResourceCompletion,&loader,
        ).await?);
        assert!(page_vm.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::RenderingUpdate,&loader,
        ).await?,"stylesheet settlement must publish another rendering opportunity");
        assert_eq!(page_vm.vm_mut().eval("document.activeElement===frame")?,"true");
        assert_eq!(page_vm.vm_mut().eval("frame.contentDocument.activeElement.id")?,"shown");
        server.await?;
        Ok::<_, anyhow::Error>(())
    }).await.expect("child stylesheet readiness must preserve and wake autofocus");
}
