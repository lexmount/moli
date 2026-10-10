use super::*;

#[tokio::test(flavor = "current_thread")]
async fn rtc_data_channel_close_uses_two_networking_boundaries_and_one_event_checkpoint() {
    run_page_vm_async_test(async move {
        let loader=crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page,_,_)=page_vm_with_bound_task_sources_and_owner_wake(&loader,Url::parse("https://channel-close.test/tasks")?);
        page.vm_mut().eval(r#"globalThis.trace=[];globalThis.pc=new RTCPeerConnection();globalThis.c=pc.createDataChannel('close');c.onclose=e=>{if(!e.isTrusted||c.readyState!=='closed')throw Error('event');trace.push('close');Promise.resolve().then(()=>trace.push('reaction'))};'created'"#)?;
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc,&loader).await?);
        page.vm_mut().eval("c.close();c.close();'closing'")?;
        assert_eq!(page.vm_mut().eval("c.readyState+'|'+trace.join('|')")?,"closing|");
        assert!(!page.vm().has_ready_timeout(),"close uses networking tasks without a JS timer");
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc,&loader).await?);
        assert_eq!(page.vm_mut().eval("c.readyState+'|'+trace.join('|')")?,"closing|");
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc,&loader).await?);
        assert_eq!(page.vm_mut().eval("c.readyState+'|'+trace.join('|')")?,"closed|close|reaction");
        page.vm_mut().eval("c.close();pc.close();'closed'")?;
        assert!(page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).is_none());
        Ok::<_,anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn rtc_data_channel_close_preserves_the_first_creation_negotiation_flag() {
    run_page_vm_async_test(async move {
        let loader=crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page,_,_)=page_vm_with_bound_task_sources_and_owner_wake(&loader,Url::parse("https://channel-close.test/first")?);
        page.vm_mut().eval("globalThis.pc=new RTCPeerConnection();globalThis.trace=[];pc.onnegotiationneeded=()=>trace.push('negotiate');globalThis.c=pc.createDataChannel('first');'created'")?;
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc,&loader).await?);
        assert_eq!(page.vm_mut().eval("trace.join('|')")?,"negotiate");
        page.vm_mut().eval("c.close();'closing'")?;
        for _ in 0..2 {assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc,&loader).await?);}
        page.vm_mut().eval("pc.createDataChannel('later');'created'")?;
        assert!(page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).is_none(),"a later channel is not the first-ever creation");
        page.vm_mut().eval("pc.close();'closed'")?;
        Ok::<_,anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn rtc_data_channel_pc_close_suppresses_a_queued_closing_procedure() {
    run_page_vm_async_test(async move {
        let loader=crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page,_,_)=page_vm_with_bound_task_sources_and_owner_wake(&loader,Url::parse("https://channel-close.test/abort-start")?);
        page.vm_mut().eval("globalThis.pc=new RTCPeerConnection();globalThis.c=pc.createDataChannel('');globalThis.trace=[];c.onclose=()=>trace.push('unexpected');'created'")?;
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc,&loader).await?);
        page.vm_mut().eval("c.close();pc.close();'aborted'")?;
        assert_eq!(page.vm_mut().eval("c.readyState")?,"closed");
        let task=page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).expect("queued close still owns its FIFO residence");
        page.run_claimed_selected_page_task_for_test(task,&loader).await?;
        assert!(page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).is_none());
        assert_eq!(page.vm_mut().eval("trace.length")?,"0");
        Ok::<_,anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn rtc_data_channel_pc_close_reaches_removed_closing_channels_and_reused_ids() {
    run_page_vm_async_test(async move {
        let loader=crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page,_,_)=page_vm_with_bound_task_sources_and_owner_wake(&loader,Url::parse("https://channel-close.test/abort-finish")?);
        page.vm_mut().eval("globalThis.pc=new RTCPeerConnection();globalThis.c=pc.createDataChannel('old',{negotiated:true,id:61});globalThis.trace=[];c.onclose=()=>trace.push('unexpected');'created'")?;
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc,&loader).await?);
        page.vm_mut().eval("c.close();'closing'")?;
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc,&loader).await?);
        assert_eq!(page.vm_mut().eval("c.readyState")?,"closing");
        page.vm_mut().eval("globalThis.next=pc.createDataChannel('next',{negotiated:true,id:61});pc.close();'aborted'")?;
        assert_eq!(page.vm_mut().eval("c.readyState+'|'+next.readyState")?,"closed|closed");
        let task=page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).expect("queued close announcement");
        page.run_claimed_selected_page_task_for_test(task,&loader).await?;
        assert_eq!(page.vm_mut().eval("trace.length")?,"0");
        assert!(page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).is_none());
        Ok::<_,anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn rtc_data_channel_close_retired_start_does_not_dispatch_or_checkpoint_replacement() {
    run_page_vm_async_test(async move {
        let loader=crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page,_,_)=page_vm_with_bound_task_sources_and_owner_wake(&loader,Url::parse("https://channel-close.test/retire-start")?);
        page.vm_mut().eval("globalThis.pc=new RTCPeerConnection();globalThis.c=pc.createDataChannel('');globalThis.trace=[];c.onclose=()=>trace.push('retired');'created'")?;
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc,&loader).await?);
        page.vm_mut().eval("c.close();document.open();document.write('<!doctype html><body>replacement');document.close();'replaced'")?;
        page.vm_mut().eval_without_microtask_checkpoint_for_test("Promise.resolve().then(()=>trace.push('replacement microtask'));'queued'")?;
        let task=page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).expect("retired closing procedure");
        page.run_claimed_selected_page_task_for_test(task,&loader).await?;
        assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("trace.length")?,"0");
        assert!(page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).is_none());
        Ok::<_,anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn rtc_data_channel_close_retired_completion_does_not_dispatch_or_checkpoint_replacement() {
    run_page_vm_async_test(async move {
        let loader=crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page,_,_)=page_vm_with_bound_task_sources_and_owner_wake(&loader,Url::parse("https://channel-close.test/retire-completion")?);
        page.vm_mut().eval("globalThis.pc=new RTCPeerConnection();globalThis.c=pc.createDataChannel('');globalThis.trace=[];c.onclose=()=>trace.push('retired');'created'")?;
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc,&loader).await?);
        page.vm_mut().eval("c.close();'closing'")?;
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc,&loader).await?);
        page.vm_mut().eval("document.open();document.write('<!doctype html><body>replacement');document.close();'replaced'")?;
        page.vm_mut().eval_without_microtask_checkpoint_for_test("Promise.resolve().then(()=>trace.push('replacement microtask'));'queued'")?;
        let task=page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).expect("retired close announcement");
        page.run_claimed_selected_page_task_for_test(task,&loader).await?;
        assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("trace.length")?,"0");
        assert!(page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).is_none());
        Ok::<_,anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn rtc_data_channel_close_tasks_follow_the_channel_document_not_callee_or_pc() {
    run_page_vm_async_test(async move {
        let loader=crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page,_,_)=page_vm_with_bound_task_sources_and_owner_wake(&loader,Url::parse("https://channel-close.test/owner")?);
        page.vm_mut().eval(r#"
          document.body.innerHTML='<iframe></iframe>';globalThis.child=document.querySelector('iframe').contentWindow;globalThis.trace=[];
          globalThis.pc=new RTCPeerConnection();globalThis.other=new child.RTCPeerConnection();
          globalThis.childChannel=child.RTCPeerConnection.prototype.createDataChannel.call(pc,'child realm');
          globalThis.mainChannel=RTCPeerConnection.prototype.createDataChannel.call(other,'main realm');
          childChannel.onclose=()=>trace.push('retired child');mainChannel.onclose=e=>{if(!(e instanceof Event)||e instanceof child.Event)throw Error('event realm');trace.push('main close')};'created'
        "#)?;
        for _ in 0..2 {assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc,&loader).await?);}
        page.vm_mut().eval(r#"
          RTCDataChannel.prototype.close.call(childChannel);child.RTCDataChannel.prototype.close.call(mainChannel);
          child.document.open();child.document.write('<!doctype html><body>replacement');child.document.close();'replaced child'
        "#)?;
        page.vm_mut().eval_without_microtask_checkpoint_for_test("child.Promise.resolve().then(()=>trace.push('replacement child microtask'));'queued'")?;
        let task=page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).expect("retired target channel task");
        page.run_claimed_selected_page_task_for_test(task,&loader).await?;
        assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("trace.length")?,"0");
        for _ in 0..2 {assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc,&loader).await?);}
        assert_eq!(page.vm_mut().eval("trace.includes('main close')&&!trace.includes('retired child')&&mainChannel.readyState==='closed'")?,"true");
        page.vm_mut().eval("pc.close();other.close();'closed'")?;
        Ok::<_,anyhow::Error>(())
    }).await.unwrap();
}
