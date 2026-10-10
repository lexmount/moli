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

#[tokio::test(flavor = "current_thread")]
async fn rtp_sender_replacement_mutates_track_then_resolves_on_distinct_networking_turns() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://rtp.test/sender-fifo")?);
        page.vm_mut().eval(r#"
          globalThis.trace=[];globalThis.pc=new RTCPeerConnection();
          globalThis.t=pc.addTransceiver('audio');globalThis.s=t.sender;globalThis.track=t.receiver.track;
          globalThis.id=s.getParameters().transactionId;
          s.replaceTrack(track).then(()=>trace.push('replaced'));
          pc.createOffer().then(()=>trace.push('offer'));'queued'
        "#)?;
        assert!(!page.vm().has_ready_timeout());
        // Negotiation-needed defers while the operations chain is occupied.
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
        assert_eq!(page.vm_mut().eval("s.track===null && trace.length===0 && s.getParameters().transactionId===id")?, "true");
        // getParameters expiry is an independent networking task, not a microtask.
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
        assert_eq!(page.vm_mut().eval("s.track===null && trace.length===0")?, "true");
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
        assert_eq!(page.vm_mut().eval("s.track===track && trace.length===0")?, "true");
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
        assert_eq!(page.vm_mut().eval("trace.join('|')")?, "replaced");
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
        assert_eq!(page.vm_mut().eval("trace.join('|')")?, "replaced|offer");
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn rtp_sender_retired_document_tasks_cannot_mutate_resolve_or_checkpoint() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://rtp.test/sender-retirement")?);
        page.vm_mut().eval(r#"
          globalThis.trace=[];globalThis.pc=new RTCPeerConnection();
          globalThis.t=pc.addTransceiver('audio');globalThis.s=t.sender;
          const params=s.getParameters();params.encodings[0].active=false;
          s.setParameters(params).then(()=>trace.push('retired parameters'));
          s.replaceTrack(t.receiver.track).then(()=>trace.push('retired replacement'));
          pc.createOffer().then(()=>trace.push('retired offer'));
          document.open();document.write('<!doctype html><body>replacement');document.close();'replaced'
        "#)?;
        page.vm_mut().eval_without_microtask_checkpoint_for_test("Promise.resolve().then(()=>trace.push('replacement microtask'));'queued'")?;
        for _ in 0..4 {
            let task = page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).expect("retired sender tasks retain their FIFO residence");
            page.run_claimed_selected_page_task_for_test(task, &loader).await?;
        }
        assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("trace.join('|')")?, "");
        assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("s.track===null")?, "true");
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn rtp_sender_close_aborts_pending_connection_operations_without_completion() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://rtp.test/sender-close")?);
        page.vm_mut().eval(r#"
          globalThis.trace=[];globalThis.pc=new RTCPeerConnection();
          globalThis.t=pc.addTransceiver('audio');globalThis.s=t.sender;
          s.replaceTrack(t.receiver.track).then(()=>trace.push('replacement'),()=>trace.push('replacement rejected'));
          pc.createOffer().then(()=>trace.push('offer'),()=>trace.push('offer rejected'));
          pc.close();'closed'
        "#)?;
        for _ in 0..3 {
            let task = page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).expect("queued closed-connection tasks");
            page.run_claimed_selected_page_task_for_test(task, &loader).await?;
        }
        assert_eq!(page.vm_mut().eval("trace.length===0 && s.track===null && t.receiver.track.readyState==='ended'")?, "true");
        assert!(!page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn webrtc_signaling_close_in_event_aborts_outer_promise_and_following_operations() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://signaling.test/close-in-event")?);
        page.vm_mut().eval(r#"
          globalThis.trace=[];globalThis.pc=new RTCPeerConnection();
          pc.onsignalingstatechange=()=>{trace.push(pc.signalingState);pc.close();};
          pc.setLocalDescription().then(()=>trace.push('resolved'),()=>trace.push('rejected'));
          pc.setLocalDescription({type:'rollback'}).then(()=>trace.push('rollback'),()=>trace.push('rollback rejected'));
          'queued'
        "#)?;
        assert_eq!(page.vm_mut().eval("pc.signalingState==='stable'&&trace.length===0")?, "true");
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
        assert_eq!(page.vm_mut().eval("trace.join(',')")?, "have-local-offer");
        assert_eq!(page.vm_mut().eval("pc.signalingState==='closed'&&pc.localDescription===pc.pendingLocalDescription&&pc.localDescription.type==='offer'")?, "true");
        assert!(!page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn webrtc_signaling_retired_task_never_fires_or_checkpoints_replacement_document() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://signaling.test/retirement")?);
        page.vm_mut().eval(r#"
          globalThis.trace=[];globalThis.pc=new RTCPeerConnection();
          pc.onsignalingstatechange=()=>trace.push('event');
          pc.setLocalDescription().then(()=>trace.push('resolved'),()=>trace.push('rejected'));
          document.open();document.write('<!doctype html><body>replacement');document.close();'replaced'
        "#)?;
        page.vm_mut().eval_without_microtask_checkpoint_for_test("Promise.resolve().then(()=>trace.push('replacement microtask'));'queued'")?;
        let task = page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).expect("retired signaling task retains FIFO residence");
        page.run_claimed_selected_page_task_for_test(task, &loader).await?;
        assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("trace.length===0&&pc.signalingState==='stable'&&pc.localDescription===null")?, "true");
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}
