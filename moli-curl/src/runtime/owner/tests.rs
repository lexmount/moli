use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    num::NonZeroUsize,
    thread,
};

use curl::easy::{Easy2, WriteError};

use super::*;
use crate::websocket::CurlWebSocketConnector;
use crate::{CurlDnsResolution, CurlHttpSender, CurlMultiJob, CurlOriginKey};

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Capture(Vec<u8>);

impl Handler for Capture {
    fn write(&mut self, data: &[u8]) -> Result<usize, WriteError> {
        self.0.extend_from_slice(data);
        Ok(data.len())
    }
}

fn request(
    address: SocketAddr,
    path: &'static str,
    deadline: Option<Instant>,
) -> CurlMultiJob<Capture, &'static str> {
    let mut easy = Easy2::new(Capture::default());
    easy.url(&format!("http://{address}{path}")).unwrap();
    easy.proxy("").unwrap();
    CurlMultiJob {
        easy,
        context: path,
        origin: Some(CurlOriginKey {
            scheme: "http".to_owned(),
            host: address.ip().to_string(),
            port: Some(address.port()),
        }),
        deadline,
        dns_resolution: CurlDnsResolution::curl_managed(),
        priority: 1,
        label: path.to_owned(),
    }
}

fn read_path(stream: &mut TcpStream) -> String {
    stream.set_read_timeout(Some(TEST_TIMEOUT)).unwrap();
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        request.push(byte[0]);
        assert!(request.len() < 4096);
    }
    String::from_utf8(request)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned()
}

fn completion_starts_queued_job(per_origin: bool) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let (accepted, ready) = crossbeam_channel::bounded(1);
    let (release, released) = crossbeam_channel::bounded(1);
    let server = thread::spawn(move || {
        let mut paths = Vec::new();
        loop {
            let (mut stream, _) = listener.accept().unwrap();
            let path = read_path(&mut stream);
            if path == "/stop" {
                return paths;
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n")
                .unwrap();
            if path == "/held" {
                accepted.send(()).unwrap();
                released.recv_timeout(TEST_TIMEOUT).unwrap();
            }
            stream.write_all(b"ok").unwrap();
            paths.push(path);
        }
    });

    let config = CurlMultiRuntimeConfig {
        max_active: NonZeroUsize::new(if per_origin { 2 } else { 1 }).unwrap(),
        max_host_active: per_origin.then(|| NonZeroUsize::new(1).unwrap()),
        ..CurlMultiRuntimeConfig::default()
    };
    let multi = make_runtime_multi(&config);
    let (command_tx, command_rx) = crossbeam_channel::unbounded();
    let (completion_tx, completed) = crossbeam_channel::unbounded();
    let (_websocket_tx, websocket_rx) = CurlWebSocketConnector::channel();
    let shutdown_requested = Arc::new(AtomicBool::new(false));
    let sender = CurlHttpSender {
        command_tx,
        owner_waker: multi.waker(),
        shutdown_requested: shutdown_requested.clone(),
    };
    let mut owner = CurlRuntimeOwner {
        command_rx,
        shutdown_requested,
        closed: false,
        poll_interval: config.poll_interval,
        diagnostics: Diagnostics::from_env(),
        http: HttpRegistry::new(config, completion_tx),
        websockets: WebSocketRegistry::new(websocket_rx),
        multi,
    };

    // Prepare a real active transfer on this thread so the test can explicitly
    // consume all submission wakeups while the peer still holds its response.
    let first = sender.submit(request(address, "/held", None)).unwrap();
    owner.drain_commands();
    owner.http.advance(&mut owner.multi);
    let setup_deadline = Instant::now() + TEST_TIMEOUT;
    loop {
        owner.process_completed_transfers();
        if ready.try_recv().is_ok() {
            break;
        }
        assert!(
            Instant::now() < setup_deadline,
            "first request did not start"
        );
        owner.wait_for_curl_progress(false);
    }
    let second = sender
        .submit(request(
            address,
            "/queued",
            Some(Instant::now() + Duration::from_millis(500)),
        ))
        .unwrap();
    owner.drain_commands();
    owner.http.advance(&mut owner.multi);
    assert!(owner.http.contains(first));
    assert!(!owner.http.contains(second));
    // In particular, B's submit() wakeup must not rescue the owner after A
    // completes. The native poll consumes it before we release A's body.
    owner.wait_for_curl_progress(false);

    let finished = thread::spawn(move || {
        let completions = (0..2)
            .map(|_| completed.recv_timeout(TEST_TIMEOUT))
            .collect::<Result<Vec<_>, _>>();
        // No new command is sent after A completes. Only B's terminal (or the
        // test watchdog) permits shutdown to wake the native owner.
        sender
            .command_tx
            .send(CurlRuntimeCommand::Shutdown)
            .unwrap();
        sender.owner_waker.wakeup().unwrap();
        completions
    });
    release.send(()).unwrap();
    owner.drive();
    let completions = finished.join().unwrap();
    let mut stop = TcpStream::connect(address).unwrap();
    stop.write_all(b"GET /stop HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let paths = server.join().unwrap();

    for (completion, id) in completions.unwrap().into_iter().zip([first, second]) {
        assert_eq!(completion.transfer_id, id);
        completion
            .result
            .expect("a queued request must start when its active slot is released");
        assert_eq!(completion.easy.unwrap().get_ref().0, b"ok");
    }
    assert_eq!(paths, ["/held", "/queued"]);
}

#[test]
fn http_completion_starts_queued_job_before_global_deadline() {
    completion_starts_queued_job(false);
}

#[test]
fn http_completion_starts_queued_job_before_per_origin_deadline() {
    completion_starts_queued_job(true);
}
