use super::*;

#[tokio::test(flavor = "current_thread")]
async fn removed_iframe_focus_does_not_suppress_later_autofocus_after_reinsertion() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page_vm, _queue, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/removed-frame-autofocus")?,
        );
        super::rendering_update::dispatch_main_document_domcontentloaded_for_rendering_test(
            &mut page_vm,
        )
        .await?;
        page_vm
            .vm_mut()
            .eval("document.body.innerHTML='<iframe id=frame></iframe>'")?;
        materialize_child_realm_through_page_turn_for_test(&mut page_vm, "frame")?;
        assert_eq!(
            page_vm.vm_mut().eval(
                r#"
const frame=document.getElementById('frame');
const child=frame.contentDocument;
const input=child.createElement('input');
child.body.append(input);
input.focus();
String(document.activeElement===frame)
"#
            )?,
            "true"
        );
        page_vm.vm_mut().eval(
            r#"
frame.remove();
document.body.append(frame);
document.body.insertAdjacentHTML('beforeend','<input id=candidate autofocus>');
'reinserted'
"#,
        )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::RenderingUpdate,
                    &loader,
                )
                .await?
        );
        assert_eq!(
            page_vm.vm_mut().eval("document.activeElement.id")?,
            "candidate"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("removed iframe focus must not survive reinsertion");
}

#[tokio::test(flavor = "current_thread")]
async fn popup_autofocus_uses_native_focusability_and_connection_order() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        for (markup, prepare, expected) in [
            ("<div id=editable contenteditable autofocus></div>", "", "editable"),
            ("<span id=span tabindex=0 autofocus></span>", "", "span"),
            (
                "<div id=host autofocus></div>",
                "globalThis.root = d.getElementById('host').attachShadow({mode:'closed',delegatesFocus:true}); root.innerHTML = '<input id=delegate>';",
                "host",
            ),
            (
                "<div id=empty autofocus></div><input id=fallback autofocus>",
                "d.getElementById('empty').attachShadow({mode:'closed',delegatesFocus:true});",
                "fallback",
            ),
            (
                "<map name=map><area id=area href=# autofocus></map><img usemap=#map>",
                "",
                "area",
            ),
            (
                "<map name=foreign><area id=wrong-document href=# autofocus></map><input id=fallback autofocus>",
                "document.body.innerHTML='<img usemap=#foreign>';",
                "fallback",
            ),
            (
                "<input id=first autofocus><input id=second autofocus>",
                "const first=d.getElementById('first'); first.remove(); d.body.prepend(first);",
                "second",
            ),
            (
                "<input id=enabled disabled autofocus><input id=later autofocus>",
                "d.getElementById('enabled').disabled=false;",
                "enabled",
            ),
        ] {
            let (mut page_vm, _queue, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
                &loader,
                Url::parse("https://example.com/popup-autofocus")?,
            );
            page_vm.vm_mut().eval(&format!(
                "globalThis.popup=open(); const d=popup.document; d.body.innerHTML={markup:?}; \
                 globalThis.order=[]; {prepare} \
                 d.addEventListener('focus', () => {{ order.push('focus'); queueMicrotask(() => order.push('microtask')); }}, true); \
                 'installed'"
            ))?;
            assert_eq!(page_vm.vm_mut().eval("order.join('|')")?, "");
            assert!(page_vm.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::RenderingUpdate, &loader,
            ).await?, "{expected}: insertion must publish rendering work");
            assert_eq!(page_vm.vm_mut().eval("popup.document.activeElement.id")?, expected);
            assert_eq!(page_vm.vm_mut().eval("order.join('|')")?, "focus|microtask");
            if expected == "host" {
                assert_eq!(page_vm.vm_mut().eval("root.activeElement.id")?, "delegate");
            }
            assert!(!page_vm.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::RenderingUpdate, &loader,
            ).await?, "one Document's insertions must coalesce");
            page_vm.vm_mut().eval("popup.close()")?;
        }
        Ok::<_, anyhow::Error>(())
    }).await.expect("popup native autofocus should run");
}

#[tokio::test(flavor = "current_thread")]
async fn popup_autofocus_is_independent_of_opener_focus_and_processed_state() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page_vm, _queue, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader, Url::parse("https://example.com/independent-autofocus")?,
        );
        page_vm.vm_mut().eval("document.body.innerHTML='<input id=main autofocus>'")?;
        super::rendering_update::dispatch_main_document_domcontentloaded_for_rendering_test(&mut page_vm).await?;
        assert!(page_vm.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::RenderingUpdate, &loader,
        ).await?);
        assert_eq!(page_vm.vm_mut().eval("document.activeElement.id")?, "main");
        page_vm.vm_mut().eval(
            "globalThis.popup=open(); popup.document.body.innerHTML='<input id=other autofocus>'; 'inserted'"
        )?;
        assert!(page_vm.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::RenderingUpdate, &loader,
        ).await?);
        assert_eq!(page_vm.vm_mut().eval("popup.document.activeElement.id")?, "other");
        page_vm.vm_mut().eval("popup.close()")?;
        Ok::<_, anyhow::Error>(())
    }).await.expect("each top-level Document has its own autofocus state");
}

#[tokio::test(flavor = "current_thread")]
async fn popup_autofocus_revalidates_removed_candidates_and_closed_owners() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page_vm, _queue, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader, Url::parse("https://example.com/retired-popup-autofocus")?,
        );
        page_vm.vm_mut().eval(
            "globalThis.popup=open(); popup.document.body.innerHTML='<input id=old autofocus>'; \
             globalThis.focusEvents=0; popup.document.addEventListener('focus',()=>focusEvents++,true); 'inserted'"
        )?;
        let claimed = page_vm.claim_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::RenderingUpdate,
        ).expect("the old Document should publish autofocus");
        page_vm.vm_mut().eval("popup.close()")?;
        page_vm.run_claimed_selected_page_task_for_test(claimed, &loader).await?;
        assert_eq!(page_vm.vm_mut().eval("String(focusEvents)")?, "0");
        page_vm.vm_mut().eval(
            "globalThis.fresh=open(); const d=fresh.document; d.body.innerHTML='<input id=removed autofocus><input id=kept autofocus>'; \
             d.getElementById('removed').remove(); d.addEventListener('focus',()=>focusEvents++,true); 'replaced'"
        )?;
        assert!(page_vm.run_exact_selected_page_task_for_test(
            PageSelectedTaskTestSelector::RenderingUpdate, &loader,
        ).await?);
        assert_eq!(page_vm.vm_mut().eval("fresh.document.activeElement.id+'|'+focusEvents")?, "kept|1");
        page_vm.vm_mut().eval("fresh.close()")?;
        Ok::<_, anyhow::Error>(())
    }).await.expect("retired popup autofocus must not run in a successor");
}

#[tokio::test(flavor = "current_thread")]
async fn popup_autofocus_does_not_admit_windowless_documents_or_foreign_elements() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page_vm, _queue, _wake) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://example.com/inert-autofocus")?,
        );
        page_vm.vm_mut().eval(
            r#"
const inert = document.implementation.createHTMLDocument('');
inert.body.innerHTML='<input autofocus><iframe></iframe>';
void inert.querySelector('iframe').contentWindow;
globalThis.popup=open();
const d=popup.document;
const foreign=d.createElementNS('urn:foreign','input');
foreign.setAttribute('autofocus','');
foreign.setAttribute('tabindex','0');
d.body.append(foreign);
"inserted"
"#,
        )?;
        assert!(
            page_vm
                .claim_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::RenderingUpdate,
                )
                .is_none(),
            "neither inert Documents nor unsupported namespaces own autofocus work"
        );
        page_vm.vm_mut().eval(
            "popup.document.body.insertAdjacentHTML('beforeend','<input id=real autofocus>')",
        )?;
        assert!(
            page_vm
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::RenderingUpdate,
                    &loader,
                )
                .await?
        );
        assert_eq!(
            page_vm.vm_mut().eval("popup.document.activeElement.id")?,
            "real"
        );
        page_vm.vm_mut().eval("popup.close()")?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("only native live top-level Documents admit autofocus");
}
