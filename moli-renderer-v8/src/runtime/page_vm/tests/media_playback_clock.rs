use super::*;

#[tokio::test(flavor = "current_thread")]
async fn windowless_network_load_is_cancelled_when_its_relevant_document_retires() {
    run_page_vm_async_test(async move {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/pending.ogg", listener.local_addr()?);
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("windowless request");
            read_http_request_head(&mut stream).await.expect("request head");
            let _ = accepted_tx.send(());
            let _ = release_rx.await;
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        });
        let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())?;
        loader.set_optional_resource_fetch_enabled(crate::types::SubresourceResourceType::Audio, true);
        let (mut page, _source, _wake) = page_vm_with_bound_task_sources_and_owner_wake(&loader, Url::parse("https://windowless-load-retire.test/")?);
        page.vm_mut().eval(&format!(r#"
globalThis.retiredMediaEvents = [];
const inert = document.implementation.createHTMLDocument('');
globalThis.pendingMedia = inert.createElement('audio');
inert.body.appendChild(pendingMedia);
pendingMedia.onerror = () => retiredMediaEvents.push('error');
pendingMedia.src = {url:?};
pendingMedia.play().then(() => retiredMediaEvents.push('fulfilled'), () => retiredMediaEvents.push('rejected'));
"#))?;
        tokio::time::timeout(Duration::from_secs(5), accepted_rx).await??;
        let pending_before = page.vm_mut().with_default_context_scope_for_test(|scope, _| {
            let global = scope.get_current_context().global(scope);
            let key = crate::util::v8str(scope, "pendingMedia");
            let value = global.get(scope, key.into()).unwrap();
            let object = v8::Local::<v8::Object>::try_from(value).unwrap();
            let (host, media) = crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, object)?;
            Ok(unsafe { &*host }.pending_media_load_sequence(media).is_some())
        })?;
        assert!(pending_before);
        page.vm_mut().eval("document.open(); document.write('<!doctype html><body>replacement'); document.close();")?;
        let pending_after = page.vm_mut().with_default_context_scope_for_test(|scope, _| {
            let global = scope.get_current_context().global(scope);
            let key = crate::util::v8str(scope, "pendingMedia");
            let value = global.get(scope, key.into()).unwrap();
            let object = v8::Local::<v8::Object>::try_from(value).unwrap();
            let (host, media) = crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, object)?;
            Ok(unsafe { &*host }.pending_media_load_sequence(media).is_some())
        })?;
        assert!(!pending_after, "retiring a relevant global cancels its load-neutral request");
        let _ = release_tx.send(());
        server.await?;
        assert_eq!(page.vm_mut().eval("retiredMediaEvents.join('|')")?, "");
        Ok::<_, anyhow::Error>(())
    }).await.expect("windowless resources share the relevant global lifetime");
}
