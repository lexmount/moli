use super::*;
use crate::browser::NetworkOwner;
use crate::page::{
    RendererNetworkOutputItem, ScriptNetworkOutputItem, SubresourceBodyFinishedResult,
};
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
enum Rejection {
    Redirect,
    PreflightRedirect,
    Cors,
    CorsXhr,
}

macro_rules! rejected_response_tests {
    ($($name:ident: $worker:literal, $rejection:ident;)*) => {
        $(#[tokio::test]
        async fn $name() {
            rejected_response($worker, Rejection::$rejection).await;
        })*
    };
}

rejected_response_tests! {
    native_rejected_response_window_redirect: false, Redirect;
    native_rejected_response_worker_redirect: true, Redirect;
    native_rejected_response_window_preflight_redirect: false, PreflightRedirect;
    native_rejected_response_worker_preflight_redirect: true, PreflightRedirect;
    native_rejected_response_window_cors: false, Cors;
    native_rejected_response_worker_cors: true, Cors;
    native_rejected_response_window_xhr_cors: false, CorsXhr;
    native_rejected_response_worker_xhr_cors: true, CorsXhr;
}

async fn rejected_response(worker: bool, rejection: Rejection) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let cross_origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let preflight = !matches!(rejection, Rejection::Redirect);
    let request_url = if preflight {
        format!("http://{}/probe", cross_origin.local_addr().unwrap())
    } else {
        format!("{origin}/probe")
    };
    let status = if matches!(rejection, Rejection::Cors | Rejection::CorsXhr) {
        200
    } else {
        302
    };
    let method = if preflight { "PUT" } else { "GET" };
    let redirect = if matches!(rejection, Rejection::Cors | Rejection::CorsXhr) {
        "follow"
    } else {
        "error"
    };
    let headers = if preflight {
        "{'x-rejected-probe':'original'}"
    } else {
        "{}"
    };
    let script = if matches!(rejection, Rejection::CorsXhr) {
        format!(
            "const xhr=new XMLHttpRequest();xhr.open('PUT',{request_url:?});xhr.setRequestHeader('x-rejected-probe','original');xhr.send()"
        )
    } else {
        format!(
            "fetch({request_url:?},{{method:{method:?},headers:{headers},redirect:{redirect:?}}}).then(()=>{{throw Error('unexpected success')}},()=>{{}})"
        )
    };
    let (body, release_body) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut saw_preflight = false;
        loop {
            let (mut stream, _) = tokio::select! {
                connection = listener.accept() => connection,
                connection = cross_origin.accept() => connection,
            }
            .unwrap();
            let mut head = Vec::new();
            let mut byte = [0];
            while !head.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).await.unwrap();
                head.push(byte[0]);
            }
            let head = String::from_utf8(head).unwrap();
            let mut line = head.split_whitespace();
            let verb = line.next().unwrap();
            let path = line.next().unwrap();
            if verb == "OPTIONS" {
                assert!(preflight && !saw_preflight);
                saw_preflight = true;
                stream.write_all(b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: PUT\r\nAccess-Control-Allow-Headers: x-rejected-probe\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                continue;
            }
            if path == "/probe" {
                assert_eq!(verb, method);
                assert_eq!(saw_preflight, preflight);
                if preflight {
                    assert!(
                        head.to_ascii_lowercase()
                            .contains("x-rejected-probe: original")
                    );
                }
                // No response body is available at the rejection boundary.
                stream.write_all(format!("HTTP/1.1 {status} response\r\nLocation: /must-not-follow\r\nX-Rejected-Response: physical\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                release_body.await.unwrap();
                // Rejection may already have cancelled this connection.
                let _ = stream.write_all(b"body").await;
                break;
            }
            let (content_type, text) = match path {
                "/page" => (
                    "text/html",
                    if worker {
                        "<!doctype html><script>globalThis.worker=new Worker('/worker.js')</script>"
                            .into()
                    } else {
                        format!("<!doctype html><script>{script}</script>")
                    },
                ),
                "/worker.js" if worker => ("text/javascript", script.clone()),
                _ => panic!("rejected request must not follow a redirect: {path}"),
            };
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",text.len()).as_bytes()).await.unwrap();
        }
    });
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    let document = navigate(&context, contents, &format!("{origin}/page")).await;
    let mut admitted = None;
    let mut response = None;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap().event;
            let (BrowserEvent::NetworkRequestStarted(occurrence)
            | BrowserEvent::NetworkActivity(occurrence)
            | BrowserEvent::NetworkRequestCompleted(occurrence)) = event
            else {
                continue;
            };
            let RendererNetworkOutputItem::Resource(item) = &occurrence.renderer.item else {
                continue;
            };
            if let ScriptNetworkOutputItem::SubresourceRequestStarted(start) = item.as_ref()
                && start.url().as_str() == request_url
                && start.method() == method
            {
                assert!(admitted.is_none(), "one request admission");
                if worker {
                    assert!(matches!(occurrence.owner, NetworkOwner::Worker(_)));
                } else {
                    assert_eq!(occurrence.owner, NetworkOwner::Document(document));
                }
                admitted = Some((
                    occurrence.owner,
                    occurrence.renderer.source.clone(),
                    start.handle(),
                ));
            }
            let Some((owner, source, handle)) = &admitted else {
                continue;
            };
            match item.as_ref() {
                ScriptNetworkOutputItem::SubresourceResponseStarted(head)
                    if head.handle() == *handle =>
                {
                    assert_eq!(&occurrence.owner, owner);
                    assert_eq!(&occurrence.renderer.source, source);
                    assert!(
                        response.replace(head.clone()).is_none(),
                        "one physical response head"
                    );
                    assert_eq!(head.status(), status);
                    assert_eq!(head.final_url().as_str(), request_url);
                    let sent = head
                        .network_request_headers()
                        .expect("the rejected physical request retains its wire headers");
                    if preflight {
                        assert!(sent.iter().any(|(name, value)| {
                            name.eq_ignore_ascii_case("x-rejected-probe") && value == "original"
                        }));
                    }
                    assert!(head.response_headers().iter().any(|(name, value)| {
                        name.eq_ignore_ascii_case("x-rejected-response") && value == "physical"
                    }));
                }
                ScriptNetworkOutputItem::SubresourceDataReceived(data)
                    if data.handle() == *handle =>
                {
                    panic!("rejection precedes the held response body");
                }
                ScriptNetworkOutputItem::SubresourceBodyFinished(terminal)
                    if terminal.handle() == *handle =>
                {
                    assert!(
                        response.is_some(),
                        "{rejection:?} lost its physical response head: {:?}",
                        terminal.result()
                    );
                    let SubresourceBodyFinishedResult::FailedWithPartialBody {
                        error_text,
                        partial_body,
                    } = terminal.result()
                    else {
                        panic!(
                            "rejection retains its response facts: {:?}",
                            terminal.result()
                        )
                    };
                    if worker && matches!(rejection, Rejection::Cors | Rejection::CorsXhr) {
                        assert_eq!(error_text, "net::ERR_FAILED");
                    } else {
                        assert!(
                            error_text.contains(
                                if matches!(rejection, Rejection::Cors | Rejection::CorsXhr) {
                                    "CORS check failed"
                                } else {
                                    "redirect mode error"
                                }
                            ),
                            "{error_text}"
                        );
                    }
                    assert!(partial_body.clone_body_bytes().is_empty());
                    break;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("response rejection must settle before releasing any body");
    body.send(()).unwrap();
    server.await.unwrap();
    context
        .close_web_contents(contents)
        .unwrap()
        .close_async()
        .await;
    service.shutdown();
}
