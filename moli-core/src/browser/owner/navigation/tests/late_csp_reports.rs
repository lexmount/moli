use super::*;
use crate::browser::NetworkOwner;
use crate::page::{
    RendererNetworkOutputItem, ScriptNetworkOutputItem, SubresourceBodyFinishedResult,
    SubresourceResourceType,
};
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
enum Retirement {
    Live,
    Close,
    CloseAfterReport,
    Replace,
    RemoveChild,
}

macro_rules! late_csp_tests {
    ($($name:ident: $retirement:ident, $report_only:literal;)*) => {
        $(#[tokio::test]
        async fn $name() {
            late_redirect_report(Retirement::$retirement, $report_only).await;
        })*
    };
}

late_csp_tests! {
    native_document_late_csp_live_report_only: Live, true;
    native_document_late_csp_live_enforce: Live, false;
    native_document_late_csp_close_report_only: Close, true;
    native_document_late_csp_close_enforce: Close, false;
    native_document_late_csp_close_after_report_only: CloseAfterReport, true;
    native_document_late_csp_close_after_report_enforce: CloseAfterReport, false;
    native_document_late_csp_replace_report_only: Replace, true;
    native_document_late_csp_replace_enforce: Replace, false;
    native_document_late_csp_child_removed_report_only: RemoveChild, true;
    native_document_late_csp_child_removed_enforce: RemoveChild, false;
}

async fn late_redirect_report(retirement: Retirement, report_only: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let final_url = format!("http://{}/final", destination.local_addr().unwrap());
    let hold_body = matches!(retirement, Retirement::CloseAfterReport);
    let (body_ready, release_body) = tokio::sync::oneshot::channel();
    let final_server = tokio::spawn(async move {
        let (mut stream, _) = destination.accept().await.unwrap();
        let (head, _) = read_request(&mut stream).await;
        assert!(head.starts_with("GET /final HTTP/1.1\r\n"));
        stream.write_all(b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").await.unwrap();
        if hold_body {
            release_body.await.unwrap();
        }
        let body = stream.write_all(b"late").await;
        if !hold_body || report_only {
            body.unwrap();
        }
    });
    let child = matches!(retirement, Retirement::RemoveChild);
    let (arrived, request_arrived) = tokio::sync::oneshot::channel();
    let (redirect, release_redirect) = tokio::sync::oneshot::channel();
    let mut server = tokio::spawn(async move {
        let mut arrived = Some(arrived);
        let mut release_redirect = Some(release_redirect);
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (head, body) = read_request(&mut stream).await;
            let path = head.split_whitespace().nth(1).unwrap();
            if path == "/initial" {
                arrived.take().unwrap().send(()).unwrap();
                release_redirect.take().unwrap().await.unwrap();
                stream.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {final_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                continue;
            }
            if path == "/csp" {
                assert!(head.starts_with("POST /csp HTTP/1.1\r\n"));
                stream.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                return serde_json::from_slice::<serde_json::Value>(&body).unwrap();
            }
            let source = path == if child { "/child" } else { "/page" };
            let html = if source {
                "<!doctype html><script>fetch('/initial',{keepalive:true}).catch(()=>{})</script>"
            } else {
                assert_eq!(path, "/page");
                assert!(child);
                "<!doctype html><iframe src='/child'></iframe>"
            };
            let policy = if source {
                let header = if report_only {
                    "Content-Security-Policy-Report-Only"
                } else {
                    "Content-Security-Policy"
                };
                format!("{header}: connect-src 'self'; report-uri /csp\r\n")
            } else {
                String::new()
            };
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n{policy}Content-Length: {}\r\nConnection: close\r\n\r\n{html}",html.len()).as_bytes()).await.unwrap();
        }
    });

    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    let document = navigate(&context, contents, &format!("{origin}/page")).await;
    tokio::time::timeout(Duration::from_secs(5), request_arrived)
        .await
        .expect("keepalive must reach the first server before retirement")
        .unwrap();
    let (source, parent) = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            if let BrowserEvent::NetworkRequestStarted(occurrence) = event.event
                && occurrence.owner == NetworkOwner::Document(document)
                && let RendererNetworkOutputItem::Resource(item) = &occurrence.renderer.item
                && let ScriptNetworkOutputItem::SubresourceRequestStarted(request) = item.as_ref()
                && request.url().as_str() == format!("{origin}/initial")
            {
                assert!(request.keepalive());
                break (occurrence.renderer.source.clone(), request.clone());
            }
        }
    })
    .await
    .expect("the parent must already have native request authority");

    let mut redirect = Some(redirect);
    let mut wire = None;
    if hold_body {
        redirect.take().unwrap().send(()).unwrap();
        wire = Some(
            tokio::time::timeout(Duration::from_secs(5), &mut server)
                .await
                .expect("the live Window must report before its body finishes")
                .unwrap(),
        );
    }
    match retirement {
        Retirement::Live => {}
        Retirement::Close | Retirement::CloseAfterReport => {
            tokio::time::timeout(
                Duration::from_secs(5),
                context.close_web_contents(contents).unwrap().close_async(),
            )
            .await
            .expect("Page retirement must not wait for its keepalive redirect");
            assert!(
                !browser
                    .subscribe()
                    .unwrap()
                    .0
                    .web_contents
                    .contains(&contents)
            );
        }
        Retirement::Replace => {
            let replacement = navigate(
                &context,
                contents,
                "data:text/html,<!doctype html><title>replacement</title>",
            )
            .await;
            assert_ne!(replacement, document);
        }
        Retirement::RemoveChild => {
            assert_eq!(context.evaluate_document_expression_for_test(document,
                "document.querySelector('iframe').remove(); document.querySelector('iframe') === null", false,
            ).await.unwrap()["value"], true);
        }
    }
    if let Some(redirect) = redirect {
        redirect.send(()).unwrap();
    }
    if hold_body {
        body_ready.send(()).unwrap();
    }
    let mut parent_terminal = None;
    let mut report_handle = None;
    let mut report_head = false;
    let mut report_terminal = false;
    let observed = tokio::time::timeout(Duration::from_secs(5), async {
        while wire.is_none() || !report_terminal || parent_terminal.is_none() {
            tokio::select! {
                body = &mut server, if wire.is_none() => wire = Some(body.unwrap()),
                event = events.recv() => {
                    let event = event.unwrap().event;
                    if let BrowserEvent::NetworkSourceClosed { source: closed, .. } = &event {
                        assert!(closed != &source.identity() || report_terminal,
                            "source closed before its late CSP report; parent={parent_terminal:?}, wire={wire:?}");
                    }
                    let (BrowserEvent::NetworkRequestStarted(occurrence)
                        | BrowserEvent::NetworkActivity(occurrence)
                        | BrowserEvent::NetworkRequestCompleted(occurrence)) = event else { continue };
                    let RendererNetworkOutputItem::Resource(item) = &occurrence.renderer.item else { continue };
                    if occurrence.renderer.source != source { continue; }
                    match item.as_ref() {
                        ScriptNetworkOutputItem::SubresourceRequestStarted(request)
                            if request.url().as_str() == format!("{origin}/csp") => {
                            assert_eq!(occurrence.owner, NetworkOwner::Document(document));
                            assert_eq!(request.document_url(), parent.document_url());
                            assert_eq!(request.frame_id(), parent.frame_id());
                            assert_eq!(request.resource_type(), SubresourceResourceType::CspReport);
                            assert!(request.keepalive());
                            assert!(report_handle.replace(request.handle()).is_none(), "one report admission");
                        }
                        ScriptNetworkOutputItem::SubresourceResponseStarted(head)
                            if Some(head.handle()) == report_handle => {
                            assert!(!report_head, "one report response");
                            assert_eq!(head.status(), 204);
                            report_head = true;
                        }
                        ScriptNetworkOutputItem::SubresourceBodyFinished(body)
                            if body.handle() == parent.handle() => {
                            assert!(parent_terminal.replace(body.clone()).is_none(), "one parent terminal");
                        }
                        ScriptNetworkOutputItem::SubresourceBodyFinished(body)
                            if Some(body.handle()) == report_handle => {
                            assert!(report_head && !report_terminal, "one report terminal after its head");
                            assert!(matches!(body.result(), SubresourceBodyFinishedResult::Ready(body) if body.clone_body_bytes().is_empty()));
                            report_terminal = true;
                        }
                        _ => {}
                    }
                }
            }
        }
    }).await;
    assert!(
        observed.is_ok(),
        "late CSP must reach HTTP and finish natively: {retirement:?}, report_only={report_only}, parent={parent_terminal:?}, wire={wire:?}"
    );
    let report = &wire.unwrap()["csp-report"];
    assert_eq!(report["document-uri"], parent.document_url().as_str());
    assert_eq!(report["effective-directive"], "connect-src");
    let parent_terminal = parent_terminal.unwrap();
    match (report_only, parent_terminal.result()) {
        (true, SubresourceBodyFinishedResult::Ready(body)) => {
            assert_eq!(body.clone_body_bytes(), b"late")
        }
        (false, SubresourceBodyFinishedResult::Failed(message))
        | (
            false,
            SubresourceBodyFinishedResult::FailedWithPartialBody {
                error_text: message,
                ..
            },
        ) => assert!(message.contains("Content Security Policy"), "{message}"),
        other => panic!("the admitted policy must still settle its request: {other:?}"),
    }
    final_server.await.unwrap();
    if !matches!(retirement, Retirement::Close | Retirement::CloseAfterReport) {
        context
            .close_web_contents(contents)
            .unwrap()
            .close_async()
            .await;
    }
    service.shutdown();
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> (String, Vec<u8>) {
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await.unwrap();
        head.push(byte[0]);
    }
    let head = String::from_utf8(head).unwrap();
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().unwrap())
        })
        .unwrap_or(0);
    let mut body = vec![0; length];
    stream.read_exact(&mut body).await.unwrap();
    (head, body)
}
