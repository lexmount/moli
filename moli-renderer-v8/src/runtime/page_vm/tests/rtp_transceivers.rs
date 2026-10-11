use super::*;

#[tokio::test(flavor = "current_thread")]
async fn rtp_sframe_mode_initializes_on_one_owned_networking_turn() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://rtp.test/sframe-mode")?);
        page.vm_mut().eval(r#"
          globalThis.worker=new Worker('data:text/javascript,onrtctransform=()=>{}');
          globalThis.pc=new RTCPeerConnection();globalThis.t=pc.addTransceiver('audio');
          globalThis.trace=[];pc.onnegotiationneeded=()=>trace.push('negotiation');'queued'
        "#)?;
        assert!(!page.vm().has_ready_timeout());
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
        assert_eq!(page.vm_mut().eval(r#"(() => {
          try {t.sender.transform=new RTCRtpScriptTransform({worker,type:'sframe'});return false;}
          catch(error){return error instanceof DOMException && error.name==='InvalidModificationError' && t.sender.transform===null && trace.length===0;}
        })()"#)?, "true", "The selected initialization task fixes an unset mode to false before negotiation.");
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
        assert_eq!(page.vm_mut().eval("trace.join('|')")?, "negotiation");
        page.vm_mut().eval("worker.terminate();pc.close();'done'")?;
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}

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
        // SFrame mode initialization has its own networking turn for each
        // transceiver, interleaved with the existing negotiation/event FIFO.
        for expected in ["", "before|negotiation|after", "before|negotiation|after", "before|negotiation|after", "before|negotiation|after|audio|audio microtask", "before|negotiation|after|audio|audio microtask|video"] {
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
        for _ in 0..3 {
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
        // Initializing SFrame mode is independent of the operations chain.
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
        assert_eq!(page.vm_mut().eval("s.track===null && trace.length===0 && s.getParameters().transactionId===id")?, "true");
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
        for _ in 0..5 {
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
        // Initialization, deferred negotiation, canceled operation and track end
        // each retain their queued networking residence after close().
        for _ in 0..4 {
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

#[tokio::test(flavor = "current_thread")]
async fn sctp_association_commits_with_the_description_on_one_networking_turn() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://sctp.test/commit")?);
        page.vm_mut().eval(r#"
          globalThis.trace=[];globalThis.pc=new RTCPeerConnection();
          pc.createDataChannel('data');
          pc.onsignalingstatechange=()=>trace.push(pc.signalingState+':'+pc.sctp.state+':'+String(pc.sctp.maxMessageSize));
          pc.setLocalDescription().then(()=>trace.push('resolved'));
          'queued'
        "#)?;
        assert_eq!(page.vm_mut().eval("pc.sctp===null&&pc.localDescription===null&&trace.length===0")?, "true");
        assert!(!page.vm().has_ready_timeout(), "SCTP association must use networking tasks");
        // The negotiation-needed task cannot run through the active operation.
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
        assert_eq!(page.vm_mut().eval("pc.sctp===null&&trace.length===0")?, "true");
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc, &loader).await?);
        assert_eq!(page.vm_mut().eval("trace.join('|')")?, "have-local-offer:connecting:null|resolved");
        assert_eq!(page.vm_mut().eval("pc.sctp instanceof RTCSctpTransport&&pc.pendingLocalDescription.type==='offer'")?, "true");
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn sctp_association_cannot_commit_resolve_or_checkpoint_after_document_open() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://sctp.test/retired-association")?);
        page.vm_mut().eval(r#"
          globalThis.trace=[];globalThis.pc=new RTCPeerConnection();pc.createDataChannel('data');
          pc.onsignalingstatechange=()=>trace.push('retired signaling');
          pc.setLocalDescription().then(()=>trace.push('retired association'));
          document.open();document.write('<!doctype html><body>replacement');document.close();'replaced'
        "#)?;
        page.vm_mut().eval_without_microtask_checkpoint_for_test("Promise.resolve().then(()=>trace.push('replacement microtask'));'queued'")?;
        for _ in 0..2 {
            let task=page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).expect("retired negotiation and local-description tasks retain their FIFO residence");
            page.run_claimed_selected_page_task_for_test(task,&loader).await?;
        }
        assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("trace.length===0&&pc.sctp===null&&pc.localDescription===null&&pc.signalingState==='stable'")?, "true");
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn sctp_rollback_events_cannot_dispatch_or_checkpoint_a_replacement_document() {
    run_page_vm_async_test(async move {
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        let (mut page, _, _) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://sctp.test/retired-events")?);
        page.vm_mut().eval("globalThis.pc=new RTCPeerConnection();pc.createDataChannel('data');pc.setLocalDescription();'queued'")?;
        for _ in 0..2 {
            assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc,&loader).await?);
        }
        // Completing the offer requeues the deferred negotiation-needed task.
        // Run it before enqueueing rollback so the next turn applies rollback.
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc,&loader).await?);
        assert!(page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).is_none());
        page.vm_mut().eval(r#"
          globalThis.trace=[];globalThis.sctp=pc.sctp;globalThis.dtls=sctp.transport;
          sctp.onstatechange=()=>trace.push('retired SCTP');dtls.onstatechange=()=>trace.push('retired DTLS');
          pc.setLocalDescription({type:'rollback'}).then(()=>trace.push('rollback'));'queued'
        "#)?;
        assert!(page.run_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc,&loader).await?);
        assert_eq!(page.vm_mut().eval("trace.join('|')")?, "rollback");
        page.vm_mut().eval("document.open();document.write('<!doctype html><body>replacement');document.close();'replaced'")?;
        page.vm_mut().eval_without_microtask_checkpoint_for_test("Promise.resolve().then(()=>trace.push('replacement microtask'));'queued'")?;
        for _ in 0..2 {
            let task=page.claim_exact_selected_page_task_for_test(PageSelectedTaskTestSelector::WebRtc).expect("retired SCTP and DTLS events retain their FIFO residence");
            page.run_claimed_selected_page_task_for_test(task,&loader).await?;
        }
        assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("trace.join('|')")?, "rollback");
        assert_eq!(page.vm_mut().eval_without_microtask_checkpoint_for_test("sctp.state==='closed'&&dtls.state==='closed'&&pc.sctp===null")?, "true");
        Ok::<_, anyhow::Error>(())
    }).await.unwrap();
}
