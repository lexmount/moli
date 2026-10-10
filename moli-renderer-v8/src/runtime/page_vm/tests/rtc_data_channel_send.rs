use super::*;

#[tokio::test(flavor = "current_thread")]
async fn rtc_data_channel_send_failures_do_not_queue_transport_tasks_or_mutate_buffering() {
    run_page_vm_async_test(async move {
        let loader=crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page,_,_)=page_vm_with_bound_task_sources_and_owner_wake(&loader,Url::parse("https://channel-send.test/tasks")?);
        page.vm_mut().eval(r#"
          globalThis.pc=new RTCPeerConnection();globalThis.c=pc.createDataChannel('send');globalThis.events=[];
          for(const name of ['message','error','bufferedamountlow'])c.addEventListener(name,()=>events.push(name));'created'
        "#)?;
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc,&loader).await?);
        page.vm_mut().eval(r#"
          for(const value of ['x',new Blob(['x']),new Uint8Array(8),Symbol()])try{c.send(value)}catch(e){}
          Promise.resolve().then(()=>events.push('reaction'));'sent'
        "#)?;
        assert_eq!(page.vm_mut().eval("events.join('|')")?,"reaction");
        assert_eq!(page.vm_mut().eval("c.bufferedAmount===0 && c.readyState==='connecting'")?,"true");
        assert!(!page.vm().has_ready_timeout());
        assert!(!page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc,&loader).await?);
        page.vm_mut().eval("pc.close();'closed'")?;
        Ok::<_,anyhow::Error>(())
    }).await.unwrap();
}
