use super::*;

#[tokio::test]
async fn websocket_source_send_returns_before_io_and_preserves_binary_then_text_order() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let source = ControlledSource::install(&mut page);
    let (url, server) =
        moli_websocket::test_support::spawn_text_binary_echo_websocket_server().await;
    page.eval(&format!(r#"
        globalThis.socket = new WebSocket({url:?});
        socket.binaryType = 'arraybuffer';
        globalThis.messages = [];
        socket.onmessage = event => messages.push(typeof event.data === 'string' ? event.data : Array.from(new Uint8Array(event.data)).join(','));
    "#)).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "String(socket.readyState)",
        "1",
        "source socket open",
    )
    .await;
    assert_eq!(page.eval("socket.send(controlledBlob.slice(100,108)); socket.send('tail'); String(socket.bufferedAmount)").unwrap(),"12");
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        source.requested.notified(),
    )
    .await
    .unwrap();
    assert_eq!(
        page.eval("messages.length + '|' + socket.bufferedAmount")
            .unwrap(),
        "0|12"
    );
    source.finish(100..108);
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "messages.join('|')",
        "100,101,102,103,104,105,106,107|tail",
        "source socket messages",
    )
    .await;
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "String(socket.bufferedAmount)",
        "0",
        "source send accounting",
    )
    .await;
    page.eval("socket.close(1000); 'closing'").unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "String(socket.readyState)",
        "3",
        "source socket closed",
    )
    .await;
    server.await.unwrap();
}

#[tokio::test]
async fn websocket_stream_source_write_settles_after_native_send() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let source = ControlledSource::install(&mut page);
    let (url, server) =
        moli_websocket::test_support::spawn_text_binary_echo_websocket_server().await;
    page.eval(&format!(r#"
        globalThis.stream = new WebSocketStream({url:?});
        globalThis.streamReady = false;
        stream.opened.then(value => {{globalThis.writer=value.writable.getWriter();globalThis.reader=value.readable.getReader();streamReady=true;}});
    "#)).unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "String(streamReady)",
        "true",
        "source stream open",
    )
    .await;
    page.eval(
        r#"
        globalThis.sent = 'pending';
        writer.write(controlledBlob.slice(100,108)).then(() => sent='done', error=>sent=error.name);
    "#,
    )
    .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        source.requested.notified(),
    )
    .await
    .unwrap();
    assert_eq!(page.eval("sent").unwrap(), "pending");
    source.finish(100..108);
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "sent",
        "done",
        "source stream send",
    )
    .await;
    page.eval("globalThis.echo='pending';reader.read().then(result=>echo=Array.from(result.value).join(','));").unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "echo",
        "100,101,102,103,104,105,106,107",
        "source stream echo",
    )
    .await;
    page.eval("globalThis.streamClosed=false;writer.close().then(()=>streamClosed=true);")
        .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "String(streamClosed)",
        "true",
        "source stream close",
    )
    .await;
    server.await.unwrap();
}

#[tokio::test]
async fn sqlite_source_blob_can_be_sent_by_a_worker_after_the_web_database_closes() {
    let manager = manager(None);
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = page(&manager, &loader);
    let (url, server) =
        moli_websocket::test_support::spawn_text_binary_echo_websocket_server().await;
    let body=r#"
        const db=await open('worker-socket',1,db=>db.createObjectStore('records'));
        const write=db.transaction('records','readwrite');
        write.objectStore('records').put(new Blob(['from sqlite']),1);await complete(write);db.close();
        const code = `onmessage=async event=>{
          const db=await new Promise((resolve,reject)=>{const r=indexedDB.open('worker-socket');r.onsuccess=()=>resolve(r.result);r.onerror=()=>reject(r.error);});
          const blob=await new Promise((resolve,reject)=>{const r=db.transaction('records').objectStore('records').get(1);r.onsuccess=()=>resolve(r.result);r.onerror=()=>reject(r.error);});
          db.close();
          const socket=new WebSocket(event.data);socket.binaryType='arraybuffer';
          socket.onopen=()=>{socket.send(blob);socket.send('tail');};
          const messages=[];
          socket.onmessage=e=>{messages.push(typeof e.data==='string'?e.data:new TextDecoder().decode(e.data));if(messages.length===2)socket.close(1000);};
          socket.onclose=()=>{postMessage(messages);close();};
          socket.onerror=()=>{postMessage('socket error');close();};
        };`;
        const workerURL=URL.createObjectURL(new Blob([code],{type:'text/javascript'}));
        const worker=new Worker(workerURL);
        const answer=new Promise((resolve,reject)=>{worker.onmessage=e=>resolve(e.data);worker.onerror=reject;});
        worker.postMessage(SOCKET_URL);
        const messages=await answer;worker.terminate();URL.revokeObjectURL(workerURL);return messages;
    "#.replace("SOCKET_URL",&serde_json::to_string(&url).unwrap());
    assert_eq!(
        run(&mut page, &loader, &body).await,
        r#"["from sqlite","tail"]"#
    );
    server.await.unwrap();
}
