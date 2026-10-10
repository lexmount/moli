use super::*;

#[tokio::test(flavor = "current_thread")]
async fn rtc_stats_networking_tasks_resolve_at_distinct_selected_boundaries() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://stats.test/fifo")?);
        page.vm_mut().eval(r#"
          globalThis.trace=[];const pc=new RTCPeerConnection();
          pc.getStats().then(()=>{trace.push('first');Promise.resolve().then(()=>trace.push('reaction'))});
          pc.getStats().then(()=>trace.push('second'));'queued'
        "#)?;
        assert!(!page.vm().has_ready_timeout(), "getStats uses the networking source");
        assert_eq!(page.vm_mut().eval("trace.length")?, "0");
        for expected in ["first|reaction", "first|reaction|second"] {
            assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
            assert_eq!(page.vm_mut().eval("trace.join('|')")?, expected);
        }
        assert!(!page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn rtc_stats_bypass_the_closed_connections_pending_operations_chain() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(
            &loader,
            Url::parse("https://stats.test/closed-chain")?,
        );
        page.vm_mut().eval(
            r#"
          globalThis.trace=[];const pc=new RTCPeerConnection();
          pc.createOffer().then(()=>trace.push('offer'),()=>trace.push('offer rejected'));
          pc.getStats().then(report=>trace.push([...report.values()][0].type));
          pc.close();'queued'
        "#,
        )?;
        assert!(
            page.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::WebRtc,
                &loader
            )
            .await?
        );
        assert_eq!(page.vm_mut().eval("trace.length")?, "0");
        assert!(
            page.run_exact_selected_page_task_for_test(
                PageSelectedTaskTestSelector::WebRtc,
                &loader
            )
            .await?
        );
        assert_eq!(page.vm_mut().eval("trace.join('|')")?, "peer-connection");
        assert!(
            !page
                .run_exact_selected_page_task_for_test(
                    PageSelectedTaskTestSelector::WebRtc,
                    &loader
                )
                .await?
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn rtc_stats_retired_document_tasks_never_resolve_or_checkpoint_replacements() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://stats.test/retirement")?);
        page.vm_mut().eval(r#"
          globalThis.trace=[];new RTCPeerConnection().getStats().then(()=>trace.push('retired stats'));
          document.open();document.write('<!doctype html><body>replacement');document.close();'replaced'
        "#)?;
        page.vm_mut().eval_without_microtask_checkpoint_for_test("Promise.resolve().then(()=>trace.push('replacement microtask'));'queued'")?;
        let task = page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).expect("retired stats task retains FIFO residence");
        page.run_claimed_selected_page_task_for_test(task, &loader).await?;
        assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("trace.length")?, "0");
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn rtc_stats_tasks_follow_the_callee_document_instead_of_connection_creation() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://stats.test/callee-owner")?);
        page.vm_mut().eval(r#"
          document.body.innerHTML='<iframe></iframe>';globalThis.child=document.querySelector('iframe').contentWindow;
          globalThis.trace=[];const pc=new RTCPeerConnection(),other=new child.RTCPeerConnection();
          child.RTCPeerConnection.prototype.getStats.call(pc).then(()=>trace.push('retired callee'));
          RTCPeerConnection.prototype.getStats.call(other).then(report=>{
            if(!(report instanceof RTCStatsReport))throw Error('report realm');trace.push('main callee');
          });
          child.document.open();child.document.write('<!doctype html><body>replacement');child.document.close();'replaced child'
        "#)?;
        page.vm_mut().eval_without_microtask_checkpoint_for_test("child.Promise.resolve().then(()=>trace.push('replacement child microtask'));'queued'")?;
        let task = page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).expect("retired child callee task");
        page.run_claimed_selected_page_task_for_test(task, &loader).await?;
        assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("trace.length")?, "0");
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
        assert_eq!(page.vm_mut().eval("trace.includes('main callee')&&!trace.includes('retired callee')")?, "true");
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}
