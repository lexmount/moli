use super::*;
use base64::Engine;
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::net::TcpStream;

struct PendingMediaGate {
    accepted: AtomicUsize,
    finished: AtomicUsize,
    released: tokio::sync::watch::Sender<bool>,
}

impl Default for PendingMediaGate {
    fn default() -> Self {
        let (released, _) = tokio::sync::watch::channel(false);
        Self {
            accepted: AtomicUsize::new(0),
            finished: AtomicUsize::new(0),
            released,
        }
    }
}

type PendingMediaGates = Arc<Mutex<HashMap<String, Arc<PendingMediaGate>>>>;

async fn serve_pending_media_request(
    mut stream: TcpStream,
    gates: PendingMediaGates,
) -> anyhow::Result<()> {
    let request = read_http_request_head(&mut stream).await?;
    let path = request
        .split_whitespace()
        .nth(1)
        .expect("HTTP request path");
    let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
    let gated = parts.first() == Some(&"gate");
    let control = matches!(parts.first(), Some(&"gate-status" | &"gate-release"));
    let gate = if gated || control {
        Some(gates.lock().entry(parts[1].to_owned()).or_default().clone())
    } else {
        None
    };
    let (body, mime) = if control {
        let gate = gate.as_ref().expect("control gate");
        if parts[0] == "gate-release" {
            gate.released.send_replace(true);
        }
        (
            serde_json::to_vec(&serde_json::json!({
                "accepted": gate.accepted.load(Ordering::Acquire),
                "finished": gate.finished.load(Ordering::Acquire),
                "released": *gate.released.borrow(),
            }))?,
            "application/json",
        )
    } else {
        let kind = if gated { parts[2] } else { parts[1] };
        if let Some(gate) = &gate {
            gate.accepted.fetch_add(1, Ordering::Release);
            let mut released = gate.released.subscribe();
            loop {
                if *released.borrow_and_update() {
                    break;
                }
                released.changed().await?;
            }
        }
        match kind {
            "audio" => (
                include_bytes!("../../../../tests/fixtures/media/sound_5.oga").to_vec(),
                "audio/ogg",
            ),
            "video" => (
                include_bytes!("../../../../../moli-media/tests/fixtures/counting.webm").to_vec(),
                "video/webm",
            ),
            other => anyhow::bail!("unexpected media fixture path: {other}"),
        }
    };
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let result = async {
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(&body).await
    }
    .await;
    if gated {
        gate.as_ref()
            .unwrap()
            .finished
            .fetch_add(1, Ordering::Release);
    }
    // Retiring the old Window may close its socket before the response is released.
    if control || !gated {
        result?;
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn media_pending_loads_follow_window_resources_and_independent_parser_delays() {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let gates: PendingMediaGates = Arc::default();
        let mut server = tokio::task::JoinSet::new();
        server.spawn(async move {
            let mut requests = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    connection = listener.accept() => {
                        let (stream, _) = connection.expect("media request");
                        requests.spawn(serve_pending_media_request(stream, gates.clone()));
                    }
                    Some(result) = requests.join_next(), if !requests.is_empty() => {
                        result.expect("media handler task").expect("media fixture response");
                    }
                }
            }
        });
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        for kind in [crate::types::SubresourceResourceType::Audio, crate::types::SubresourceResourceType::Video] {
            loader.set_optional_resource_fetch_enabled(kind, true);
        }
        let (mut page, _source, _wake) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse(&base)?);
        page.vm_mut().set_layout_policy(moli_page_types::LayoutPolicy::Mock);
        let local_executor = page.local_executor.clone();
        local_executor.run(async move {
        let sources = serde_json::json!({
            "data": {
                "audio": format!("data:audio/ogg;base64,{}", base64::engine::general_purpose::STANDARD.encode(include_bytes!("../../../../tests/fixtures/media/sound_5.oga"))),
                "video": format!("data:video/webm;base64,{}", base64::engine::general_purpose::STANDARD.encode(include_bytes!("../../../../../moli-media/tests/fixtures/counting.webm"))),
            },
            "http": {"audio": format!("{base}/valid/audio"), "video": format!("{base}/valid/video")},
        });
        page.vm_mut().eval(&format!("globalThis.__mediaTimelineSources={sources};"))?;
        let mut pending_lifecycle = None;
        let snapshot = page.document_replacement_lifecycle_action_snapshot();
        page.vm_mut().eval(include_str!("../../../../tests/fixtures/media-pending-loads.js"))?;
        page.reconcile_document_replacement_lifecycle_after_owner_action(snapshot, &mut pending_lifecycle).await?;
        let deadline = Instant::now() + Duration::from_secs(180);
        while page.vm_mut().eval("typeof __uiEventResults")? == "undefined" {
            if let Some(document) = pending_lifecycle.as_ref().map(|pending| pending.document) {
                let snapshot = page.document_replacement_lifecycle_action_snapshot();
                page.advance_post_parse_lifecycle_one_owner_turn(&mut pending_lifecycle, document).await?;
                page.reconcile_document_replacement_lifecycle_after_owner_action(snapshot, &mut pending_lifecycle).await?;
            }
            let snapshot = page.document_replacement_lifecycle_action_snapshot();
            let _ = page.run_next_child_frame_task_source_for_semantic_test().await;
            page.reconcile_document_replacement_lifecycle_after_owner_action(snapshot, &mut pending_lifecycle).await?;
            for selector in [PageSelectedTaskTestSelector::ResourceCompletion, PageSelectedTaskTestSelector::MediaElementEvent, PageSelectedTaskTestSelector::AnyDomManipulation, PageSelectedTaskTestSelector::InternalLoading, PageSelectedTaskTestSelector::MainParserContinuation] {
                let snapshot = page.document_replacement_lifecycle_action_snapshot();
                let _ = page.run_exact_selected_page_task_for_test(selector, &loader).await?;
                page.reconcile_document_replacement_lifecycle_after_owner_action(snapshot, &mut pending_lifecycle).await?;
            }
            let snapshot = page.document_replacement_lifecycle_action_snapshot();
            let _ = page.run_one_due_timer_selected_task_for_test(&loader).await?;
            page.reconcile_document_replacement_lifecycle_after_owner_action(snapshot, &mut pending_lifecycle).await?;
            anyhow::ensure!(Instant::now() < deadline, "pending media fixture timed out: {}", page.vm_mut().eval("JSON.stringify(__mediaPendingLoadProgress)")?);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        server.abort_all();
        while server.join_next().await.is_some() {}
        let result: serde_json::Value = serde_json::from_str(&page.vm_mut().eval("JSON.stringify(__uiEventResults)")?)?;
        let failures: Vec<_> = result["checks"].as_array().unwrap().iter().filter(|check| check["passed"] != true).collect();
        assert!(failures.is_empty(), "{failures:#?}");
        assert_eq!(result["total"], 86);
        Ok::<_, anyhow::Error>(())
        }).await
    }).await.expect("pending media resources survive parser replacement with exact Window authority");
}
