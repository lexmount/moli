use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn font_face_cross_frame_message_uses_creation_window_csp() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let child = r#"<!doctype html><script>
        globalThis.childFace=new FontFace('ChildOwner',parent.parentFace.source);
        childFace.loaded.catch(()=>{});
        onmessage=()=>parent.parentFace.load().then(
            ()=>parent.postMessage('parent:loaded','*'),
            error=>parent.postMessage('parent:'+error.name,'*'));
        parent.postMessage('ready','*');
    </script>"#;
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut bytes = [0; 1024];
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let count = socket.read(&mut bytes).await.unwrap();
            assert!(count > 0);
            request.extend_from_slice(&bytes[..count]);
        }
        assert!(request.starts_with(b"GET /child.html "));
        socket.write_all(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Security-Policy: font-src 'none'\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{child}",
            child.len()
        ).as_bytes()).await.unwrap();
    });
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(
        &format!("{origin}/parent.html"),
        &loader,
    );
    vm.set_response_content_security_policies(&["font-src data:".into()]);
    let source = serde_json::to_string(&format!("url(\"{}\")", fixture_font_url())).unwrap();
    vm.eval(&format!(
        r#"
        globalThis.ownerResults=[];
        globalThis.parentFace=new FontFace('ParentOwner', {source});
        document.fonts.add(parentFace);
        globalThis.frame=document.createElement('iframe');
        frame.src='{origin}/child.html';
        addEventListener('message',event=>{{
          if(event.data==='ready') frame.contentWindow.postMessage('load','*');
          else ownerResults.push(event.data);
        }});
        (document.body||document.documentElement||document).append(frame);
    "#
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(ownerResults.length)",
        "1",
        "cross-frame FontFace load result",
    )
    .await;
    assert_eq!(vm.eval("ownerResults[0]").unwrap(), "parent:loaded");
    assert_eq!(
        vm.eval("frame.contentWindow.childFace.load();frame.contentWindow.childFace.status")
            .unwrap(),
        "error",
        "the child FontFace must still use its own blocking CSP"
    );
    server.await.unwrap();
}
