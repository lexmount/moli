use std::{
    io::{Read, Write},
    net::TcpListener,
    time::Duration,
};

use super::*;
use crate::{CurlDnsResolution, CurlMultiJob};
use curl::easy::Easy2;

#[derive(Debug)]
struct TestHandler;

impl Handler for TestHandler {}

#[test]
fn submitted_identity_reaches_the_matching_runtime_completion() {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .expect("test HTTP listener should bind to a local port");
    let address = listener
        .local_addr()
        .expect("test HTTP listener should have an address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("curl should connect to the test HTTP listener");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("test HTTP connection should accept a read timeout");
        let mut request = [0; 4096];
        let _ = stream
            .read(&mut request)
            .expect("test HTTP request should be readable");
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .expect("test HTTP response should be writable");
    });

    let (runtime, completion_rx) = CurlMultiRuntime::new(CurlMultiRuntimeConfig {
        poll_interval: Duration::from_millis(5),
        ..CurlMultiRuntimeConfig::default()
    })
    .expect("test curl runtime should start");
    let mut easy = Easy2::new(TestHandler);
    easy.url(&format!("http://{address}/identity"))
        .expect("test curl URL should be valid");
    let transfer_id = runtime
        .http_sender()
        .submit(CurlMultiJob {
            easy,
            context: "matching-context".to_owned(),
            origin: None,
            deadline: None,
            dns_resolution: CurlDnsResolution::curl_managed(),
            priority: 1,
            label: "identity-test".to_owned(),
        })
        .expect("test curl transfer should be accepted");

    let completion = completion_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("test curl transfer should reach terminal completion");
    assert_eq!(completion.transfer_id, transfer_id);
    assert_eq!(completion.context, "matching-context");
    assert!(completion.easy.is_some());
    completion
        .result
        .expect("test curl transfer should complete successfully");

    runtime.shutdown();
    server.join().expect("test HTTP server should finish");
}

#[test]
fn http_sender_does_not_keep_owner_alive_and_returns_rejected_job() {
    let (runtime, completed) =
        CurlMultiRuntime::<TestHandler, Vec<u8>>::new(Default::default()).unwrap();
    let sender = runtime.http_sender();
    let retained = sender.clone();
    drop(runtime);
    for sender in [sender, retained] {
        let mut easy = Easy2::new(TestHandler);
        easy.url("http://127.0.0.1:1/must-not-connect").unwrap();
        let error = sender
            .submit(CurlMultiJob {
                easy,
                context: vec![7; 1024],
                origin: None,
                deadline: None,
                dns_resolution: CurlDnsResolution::curl_managed(),
                priority: 1,
                label: "closed".into(),
            })
            .unwrap_err();
        assert!(error.error.to_string().contains("shutting down"));
        assert_eq!(error.job.context, vec![7; 1024]);
        assert_eq!(error.job.label, "closed");
    }
    assert!(matches!(
        completed.try_recv(),
        Err(crossbeam_channel::TryRecvError::Disconnected)
    ));
}
