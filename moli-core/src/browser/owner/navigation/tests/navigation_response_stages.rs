use super::*;
use crate::browser::{BrowserEventReceiver, NavigationRequest, NavigationResponseSnapshot};
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
enum Finish {
    Complete,
    Partial,
    Cancel,
}

macro_rules! navigation_stage_tests {
    ($($name:ident: $xml:literal, $finish:ident;)*) => {
        $(#[tokio::test]
        async fn $name() {
            navigation_response_stages($xml, Finish::$finish).await;
        })*
    };
}

navigation_stage_tests! {
    native_navigation_stages_html: false, Complete;
    native_navigation_stages_xml: true, Complete;
    native_navigation_stages_html_partial: false, Partial;
    native_navigation_stages_xml_partial: true, Partial;
    native_navigation_stages_html_cancel: false, Cancel;
    native_navigation_stages_xml_cancel: true, Cancel;
}

async fn response_at(
    context: &BrowserContextHandle,
    events: &mut BrowserEventReceiver,
    request: NavigationRequest,
    ready: impl Fn(&NavigationResponseSnapshot) -> bool,
) -> NavigationResponseSnapshot {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(response) = context
                .navigation_observation(request.web_contents)
                .unwrap()
                .responses
                .into_iter()
                .find(|response| response.request == request && ready(response))
            {
                return response;
            }
            events.recv().await.unwrap();
        }
    })
    .await
    .expect("original navigation must publish this phase before the fixture releases the next one")
}

async fn navigation_response_stages(xml: bool, finish: Finish) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url: Url = format!("http://{}/document", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let (prefix, tail, mime) = if xml {
        ("<root>", "done</root>", "application/xml")
    } else {
        (
            "<!doctype html><html><body>",
            "done</body></html>",
            "text/html",
        )
    };
    let (first, release_first) = oneshot::channel();
    let (last, release_last) = oneshot::channel();
    let (closed, peer_closed) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
        }
        assert!(request.starts_with(b"GET /document "));
        stream.write_all(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            prefix.len() + tail.len(),
        ).as_bytes()).await.unwrap();
        release_first.await.unwrap();
        stream.write_all(prefix.as_bytes()).await.unwrap();
        if matches!(finish, Finish::Cancel) {
            assert_eq!(
                stream.read(&mut byte).await.unwrap(),
                0,
                "cancellation must close the original transport while its tail is withheld"
            );
            closed.send(()).unwrap();
        } else {
            release_last.await.unwrap();
            if matches!(finish, Finish::Complete) {
                stream.write_all(tail.as_bytes()).await.unwrap();
            }
        }
    });
    let service = BrowserService::start().unwrap();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = service.handle().subscribe().unwrap();
    let waiter = context
        .navigate_document(
            contents,
            crate::browser::web_contents::NavigationRequestInterception::new(
                url.clone(),
                "GET".into(),
                None,
                Vec::new().into(),
                NavigationRequestLoadPolicy::BrowserInitiated,
            ),
        )
        .unwrap();
    let request = waiter.request();
    let head = response_at(&context, &mut events, request, |_| true).await;
    assert_eq!(head.response.as_ref().unwrap().final_url, url);
    assert_eq!(head.received_bytes, 0);
    assert!(head.body.is_none());
    first.send(()).unwrap();
    let progress = response_at(&context, &mut events, request, |response| {
        response.received_bytes != 0
    })
    .await;
    assert_eq!(progress.received_bytes, prefix.len());
    assert!(
        progress.body.is_none(),
        "a body prefix is not a terminal response"
    );
    if matches!(finish, Finish::Cancel) {
        assert!(
            context
                .cancel_document_navigation(contents, &request.navigation)
                .unwrap()
        );
        tokio::time::timeout(Duration::from_secs(5), peer_closed)
            .await
            .expect("canceled physical connection must close before releasing the fixture")
            .unwrap();
    } else {
        last.send(()).unwrap();
    }
    let terminal = response_at(&context, &mut events, request, |response| {
        response.body.is_some()
    })
    .await;
    let expected = if matches!(finish, Finish::Complete) {
        format!("{prefix}{tail}")
    } else {
        prefix.to_owned()
    };
    assert_eq!(terminal.received_bytes, expected.len());
    let body = match (finish, terminal.body.unwrap()) {
        (Finish::Complete, Ok(body)) => body,
        (Finish::Partial | Finish::Cancel, Err(failure)) => {
            assert!(!failure.error_text.is_empty());
            failure
                .partial_body
                .expect("failure must preserve its exact received prefix")
        }
        result => panic!("unexpected physical terminal: {result:?}"),
    };
    assert_eq!(body.materialize_bytes().unwrap(), expected.as_bytes());
    server.await.unwrap();
    if matches!(finish, Finish::Complete) {
        waiter.wait().await.unwrap();
    }
    service.shutdown();
}
