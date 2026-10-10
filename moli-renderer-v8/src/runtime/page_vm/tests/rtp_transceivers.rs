use super::*;

#[tokio::test(flavor = "current_thread")]
async fn rtp_transceiver_tasks_use_networking_fifo_and_one_selected_boundary() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://rtp.test/fifo")?);
        page.vm_mut().eval(r#"
          globalThis.trace=[];globalThis.pc=new RTCPeerConnection();
          pc.addEventListener('negotiationneeded',()=>trace.push('before'));
          pc.onnegotiationneeded=()=>trace.push('original');
          pc.addEventListener('negotiationneeded',()=>trace.push('after'));
          pc.onnegotiationneeded=e=>{if(!e.isTrusted)throw Error('untrusted negotiation');trace.push('negotiation');};
          globalThis.audio=pc.addTransceiver('audio');globalThis.video=pc.addTransceiver('video');
          pc.addTransceiver('audio');
          audio.receiver.track.onended=()=>{trace.push('audio');Promise.resolve().then(()=>trace.push('audio microtask'));};
          video.receiver.track.onended=()=>trace.push('video');
          audio.stop();video.stop();'queued'
        "#)?;
        assert!(!page.vm().has_ready_timeout(), "RTP tasks cannot borrow the timer source");
        for expected in ["before|negotiation|after", "before|negotiation|after|audio|audio microtask", "before|negotiation|after|audio|audio microtask|video"] {
            assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
            assert_eq!(page.vm_mut().eval("trace.join('|')")?, expected);
        }
        assert!(!page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn rtp_transceiver_tasks_cannot_dispatch_or_checkpoint_after_document_open() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://rtp.test/retirement")?);
        page.vm_mut().eval(r#"
          globalThis.trace=[];const pc=new RTCPeerConnection();const t=pc.addTransceiver('audio');
          t.receiver.track.onended=()=>trace.push('retired ended');t.stop();
          document.open();document.write('<!doctype html><body>replacement');document.close();'replaced'
        "#)?;
        page.vm_mut().eval_without_microtask_checkpoint_for_test("Promise.resolve().then(()=>trace.push('replacement microtask'));'queued'")?;
        for _ in 0..2 {
            let task = page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).expect("retired task retains its FIFO residence");
            page.run_claimed_selected_page_task_for_test(task, &loader).await?;
        }
        assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("trace.join('|')")?, "", "stale tasks must not checkpoint the replacement realm");
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}
