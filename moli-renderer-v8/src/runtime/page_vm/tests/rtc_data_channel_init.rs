use super::*;

#[tokio::test(flavor = "current_thread")]
async fn rtc_data_channel_init_first_success_queues_one_negotiation_networking_task() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://channels.test/tasks")?);
        page.vm_mut().eval(r#"
          globalThis.pc=new RTCPeerConnection();globalThis.trace=[];
          pc.onnegotiationneeded=e=>{if(!e.isTrusted)throw Error('trust');trace.push('event');Promise.resolve().then(()=>trace.push('reaction'))};
          for(const options of [{negotiated:true},{maxRetransmits:0,maxPacketLifeTime:0}])try{pc.createDataChannel('',options)}catch(e){}
          'invalid'
        "#)?;
        assert!(page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).is_none());
        page.vm_mut().eval("pc.createDataChannel('a');pc.createDataChannel('b');'created'")?;
        assert!(!page.vm().has_ready_timeout(), "native negotiation uses the networking source");
        assert_eq!(page.vm_mut().eval("trace.length")?, "0");
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
        assert_eq!(page.vm_mut().eval("trace.join('|')")?, "event|reaction");
        assert!(!page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
        page.vm_mut().eval("pc.createDataChannel('c');pc.close();'closed'")?;
        assert!(!page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn rtc_data_channel_init_retired_document_task_does_not_dispatch_or_checkpoint_replacement() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://channels.test/retirement")?);
        page.vm_mut().eval(r#"
          globalThis.trace=[];const pc=new RTCPeerConnection();pc.onnegotiationneeded=()=>trace.push('retired');
          pc.createDataChannel('');document.open();document.write('<!doctype html><body>replacement');document.close();'replaced'
        "#)?;
        page.vm_mut().eval_without_microtask_checkpoint_for_test("Promise.resolve().then(()=>trace.push('replacement microtask'));'queued'")?;
        let task = page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).expect("retired data channel task");
        page.run_claimed_selected_page_task_for_test(task, &loader).await?;
        assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("trace.length")?, "0");
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}
