use std::{
    collections::VecDeque,
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc as std_mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::FetchClientHandle;
use parking_lot::Mutex;

pub(super) struct ScriptedH2Server {
    addr: std::net::SocketAddr,
    hits: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<String>>>,
    connection_stream_counts: Arc<Mutex<Vec<usize>>>,
    shutdown_tx: std_mpsc::Sender<()>,
    join_handle: Option<thread::JoinHandle<()>>,
}

pub(super) struct Http2ProtocolFallbackServer {
    addr: std::net::SocketAddr,
    h2_hits: Arc<AtomicUsize>,
    http11_hits: Arc<AtomicUsize>,
    shutdown_tx: std_mpsc::Sender<()>,
    join_handle: Option<thread::JoinHandle<()>>,
}

pub(super) struct EmptyHttpHttpsUpgradeServer {
    addr: std::net::SocketAddr,
    http_hits: Arc<AtomicUsize>,
    https_hits: Arc<AtomicUsize>,
    shutdown_tx: std_mpsc::Sender<()>,
    join_handle: Option<thread::JoinHandle<()>>,
}

pub(super) struct ScriptedHttps11Server {
    addr: std::net::SocketAddr,
    hits: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<String>>>,
    request_heads: Arc<Mutex<Vec<String>>>,
    shutdown_tx: std_mpsc::Sender<()>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl ScriptedHttps11Server {
    pub(super) fn spawn(responses: Vec<ScriptedResponse>) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener
            .set_nonblocking(true)
            .expect("test https listener should be nonblocking");
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let request_heads = Arc::new(Mutex::new(Vec::new()));
        let (shutdown_tx, shutdown_rx) = std_mpsc::channel();
        let responses = Arc::new(Mutex::new(VecDeque::from(responses)));
        let hits_for_thread = Arc::clone(&hits);
        let requests_for_thread = Arc::clone(&requests);
        let request_heads_for_thread = Arc::clone(&request_heads);
        let join_handle = thread::spawn(move || {
            run_https11_server(
                listener,
                shutdown_rx,
                hits_for_thread,
                requests_for_thread,
                request_heads_for_thread,
                responses,
            );
        });

        Self {
            addr,
            hits,
            requests,
            request_heads,
            shutdown_tx,
            join_handle: Some(join_handle),
        }
    }

    pub(super) fn url(&self) -> String {
        format!("https://{}/cache", self.addr)
    }

    pub(super) fn url_path(&self, path: &str) -> String {
        let path = path.strip_prefix('/').unwrap_or(path);
        format!("https://{}/{}", self.addr, path)
    }

    pub(super) fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    pub(super) fn requests(&self) -> Vec<String> {
        self.requests.lock().clone()
    }

    pub(super) fn request_heads(&self) -> Vec<String> {
        self.request_heads.lock().clone()
    }

    pub(super) fn shutdown(mut self) {
        let _ = self.shutdown_tx.send(());
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

impl ScriptedH2Server {
    pub(super) fn spawn(responses: Vec<ScriptedResponse>) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener
            .set_nonblocking(true)
            .expect("test h2 listener should be nonblocking");
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let connection_stream_counts = Arc::new(Mutex::new(Vec::new()));
        let (shutdown_tx, shutdown_rx) = std_mpsc::channel();
        let responses = Arc::new(Mutex::new(VecDeque::from(responses)));
        let hits_for_thread = Arc::clone(&hits);
        let requests_for_thread = Arc::clone(&requests);
        let connection_stream_counts_for_thread = Arc::clone(&connection_stream_counts);
        let join_handle = thread::spawn(move || {
            run_h2_server(
                listener,
                shutdown_rx,
                hits_for_thread,
                requests_for_thread,
                connection_stream_counts_for_thread,
                responses,
            );
        });

        Self {
            addr,
            hits,
            requests,
            connection_stream_counts,
            shutdown_tx,
            join_handle: Some(join_handle),
        }
    }

    pub(super) fn url_path(&self, path: &str) -> String {
        let path = path.strip_prefix('/').unwrap_or(path);
        format!("https://{}/{}", self.addr, path)
    }

    pub(super) fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    pub(super) fn requests(&self) -> Vec<String> {
        self.requests.lock().clone()
    }

    pub(super) fn connection_stream_counts(&self) -> Vec<usize> {
        self.connection_stream_counts.lock().clone()
    }

    pub(super) fn shutdown(mut self) {
        let _ = self.shutdown_tx.send(());
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

impl Http2ProtocolFallbackServer {
    pub(super) fn spawn() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener
            .set_nonblocking(true)
            .expect("test protocol fallback listener should be nonblocking");
        let addr = listener.local_addr().unwrap();
        let h2_hits = Arc::new(AtomicUsize::new(0));
        let http11_hits = Arc::new(AtomicUsize::new(0));
        let (shutdown_tx, shutdown_rx) = std_mpsc::channel();
        let h2_hits_for_thread = Arc::clone(&h2_hits);
        let http11_hits_for_thread = Arc::clone(&http11_hits);
        let join_handle = thread::spawn(move || {
            run_http2_protocol_fallback_server(
                listener,
                shutdown_rx,
                h2_hits_for_thread,
                http11_hits_for_thread,
            );
        });

        Self {
            addr,
            h2_hits,
            http11_hits,
            shutdown_tx,
            join_handle: Some(join_handle),
        }
    }

    pub(super) fn url(&self) -> String {
        format!("https://{}/fallback", self.addr)
    }

    pub(super) fn h2_hits(&self) -> usize {
        self.h2_hits.load(Ordering::SeqCst)
    }

    pub(super) fn http11_hits(&self) -> usize {
        self.http11_hits.load(Ordering::SeqCst)
    }

    pub(super) fn shutdown(mut self) {
        let _ = self.shutdown_tx.send(());
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

impl EmptyHttpHttpsUpgradeServer {
    pub(super) fn spawn() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener
            .set_nonblocking(true)
            .expect("test empty HTTP upgrade listener should be nonblocking");
        let addr = listener.local_addr().unwrap();
        let http_hits = Arc::new(AtomicUsize::new(0));
        let https_hits = Arc::new(AtomicUsize::new(0));
        let (shutdown_tx, shutdown_rx) = std_mpsc::channel();
        let http_hits_for_thread = Arc::clone(&http_hits);
        let https_hits_for_thread = Arc::clone(&https_hits);
        let join_handle = thread::spawn(move || {
            run_empty_http_https_upgrade_server(
                listener,
                shutdown_rx,
                http_hits_for_thread,
                https_hits_for_thread,
            );
        });

        Self {
            addr,
            http_hits,
            https_hits,
            shutdown_tx,
            join_handle: Some(join_handle),
        }
    }

    pub(super) fn url(&self) -> String {
        format!("http://upgrade.example.org:{}/fallback", self.addr.port())
    }

    pub(super) fn resolve_entry(&self) -> String {
        format!("upgrade.example.org:{}:127.0.0.1", self.addr.port())
    }

    pub(super) fn http_hits(&self) -> usize {
        self.http_hits.load(Ordering::SeqCst)
    }

    pub(super) fn https_hits(&self) -> usize {
        self.https_hits.load(Ordering::SeqCst)
    }

    pub(super) fn shutdown(mut self) {
        let _ = self.shutdown_tx.send(());
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

pub(super) struct ScriptedHttpServer {
    addr: std::net::SocketAddr,
    hits: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<String>>>,
    shutdown_tx: std_mpsc::Sender<()>,
    join_handle: Option<thread::JoinHandle<()>>,
    handler_handles: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
}

fn run_h2_server(
    listener: std::net::TcpListener,
    shutdown_rx: std_mpsc::Receiver<()>,
    hits: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<String>>>,
    connection_stream_counts: Arc<Mutex<Vec<usize>>>,
    responses: Arc<Mutex<VecDeque<ScriptedResponse>>>,
) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .expect("h2 test runtime");
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::from_std(listener).expect("tokio h2 listener");
        let tls_acceptor = h2_tls_acceptor();
        loop {
            if shutdown_rx.try_recv().is_ok() {
                break;
            }
            let accepted = tokio::time::timeout(Duration::from_millis(25), listener.accept()).await;
            let Ok(Ok((stream, _))) = accepted else {
                continue;
            };
            let tls_acceptor = tls_acceptor.clone();
            let hits = Arc::clone(&hits);
            let requests = Arc::clone(&requests);
            let connection_stream_counts = Arc::clone(&connection_stream_counts);
            let responses = Arc::clone(&responses);
            tokio::spawn(async move {
                let Ok(stream) = tls_acceptor.accept(stream).await else {
                    return;
                };
                if stream.get_ref().1.alpn_protocol() != Some(b"h2") {
                    return;
                }
                let Ok(mut connection) = h2::server::handshake(stream).await else {
                    return;
                };
                let connection_index = {
                    let mut counts = connection_stream_counts.lock();
                    counts.push(0);
                    counts.len() - 1
                };
                while let Some(result) = connection.accept().await {
                    let Ok((request, mut respond)) = result else {
                        break;
                    };
                    {
                        let mut counts = connection_stream_counts.lock();
                        counts[connection_index] += 1;
                    }
                    requests.lock().push(request.uri().path().to_owned());
                    let _ = hits.fetch_add(1, Ordering::SeqCst) + 1;
                    let response_spec = {
                        let mut responses = responses.lock();
                        responses
                            .pop_front()
                            .or_else(|| responses.back().cloned())
                            .expect("scripted h2 responses should not be empty")
                    };
                    tokio::spawn(async move {
                        if response_spec.delay_ms > 0 {
                            tokio::time::sleep(Duration::from_millis(response_spec.delay_ms)).await;
                        }
                        let response = http::Response::builder()
                            .status(response_spec.status)
                            .header("content-type", "text/plain")
                            .body(())
                            .expect("h2 test response should build");
                        let Ok(mut send) = respond.send_response(response, false) else {
                            return;
                        };
                        let _ = send.send_data(response_spec.body.into(), true);
                    });
                }
            });
        }
    });
}

fn run_http2_protocol_fallback_server(
    listener: std::net::TcpListener,
    shutdown_rx: std_mpsc::Receiver<()>,
    h2_hits: Arc<AtomicUsize>,
    http11_hits: Arc<AtomicUsize>,
) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .expect("protocol fallback test runtime");
    rt.block_on(async move {
        let listener =
            tokio::net::TcpListener::from_std(listener).expect("tokio protocol fallback listener");
        let tls_acceptor = http2_and_http11_tls_acceptor();
        loop {
            if shutdown_rx.try_recv().is_ok() {
                break;
            }
            let accepted = tokio::time::timeout(Duration::from_millis(25), listener.accept()).await;
            let Ok(Ok((stream, _))) = accepted else {
                continue;
            };
            let tls_acceptor = tls_acceptor.clone();
            let h2_hits = Arc::clone(&h2_hits);
            let http11_hits = Arc::clone(&http11_hits);
            tokio::spawn(async move {
                let Ok(mut stream) = tls_acceptor.accept(stream).await else {
                    return;
                };
                match stream.get_ref().1.alpn_protocol() {
                    Some(b"h2") => {
                        h2_hits.fetch_add(1, Ordering::SeqCst);
                        send_malformed_http2_response(&mut stream).await;
                    }
                    Some(b"http/1.1") => {
                        http11_hits.fetch_add(1, Ordering::SeqCst);
                        send_http11_fallback_response(&mut stream).await;
                    }
                    _ => {}
                }
            });
        }
    });
}

fn run_empty_http_https_upgrade_server(
    listener: std::net::TcpListener,
    shutdown_rx: std_mpsc::Receiver<()>,
    http_hits: Arc<AtomicUsize>,
    https_hits: Arc<AtomicUsize>,
) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .expect("empty HTTP upgrade test runtime");
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::from_std(listener)
            .expect("tokio empty HTTP upgrade listener");
        let tls_acceptor = http11_tls_acceptor();
        loop {
            if shutdown_rx.try_recv().is_ok() {
                break;
            }
            let accepted = tokio::time::timeout(Duration::from_millis(25), listener.accept()).await;
            let Ok(Ok((mut stream, _))) = accepted else {
                continue;
            };
            let tls_acceptor = tls_acceptor.clone();
            let http_hits = Arc::clone(&http_hits);
            let https_hits = Arc::clone(&https_hits);
            tokio::spawn(async move {
                let mut first_byte = [0; 1];
                let Ok(1) = stream.peek(&mut first_byte).await else {
                    return;
                };
                if first_byte[0] == 0x16 {
                    https_hits.fetch_add(1, Ordering::SeqCst);
                    let Ok(mut stream) = tls_acceptor.accept(stream).await else {
                        return;
                    };
                    read_http_request_head(&mut stream).await;
                    let body = b"https upgrade fallback";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    if tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes())
                        .await
                        .is_ok()
                    {
                        let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, body).await;
                        let _ = tokio::io::AsyncWriteExt::flush(&mut stream).await;
                    }
                } else {
                    http_hits.fetch_add(1, Ordering::SeqCst);
                    read_http_request_head(&mut stream).await;
                    let _ = tokio::io::AsyncWriteExt::shutdown(&mut stream).await;
                }
            });
        }
    });
}

async fn read_http_request_head<S>(stream: &mut S)
where
    S: tokio::io::AsyncRead + Unpin,
{
    let mut request = Vec::new();
    let mut buf = [0; 1024];
    loop {
        let read = match tokio::io::AsyncReadExt::read(stream, &mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        request.extend_from_slice(&buf[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return;
        }
    }
}

async fn send_malformed_http2_response(
    stream: &mut tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
) {
    const CLIENT_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
    const SETTINGS: &[u8] = &[0, 0, 0, 4, 0, 0, 0, 0, 0];
    const SETTINGS_ACK: &[u8] = &[0, 0, 0, 4, 1, 0, 0, 0, 0];
    const INVALID_RESPONSE_HEADERS: &[u8] = &[
        0x88, // :status: 200
        0x00, 0x07, b'u', b'p', b'g', b'r', b'a', b'd', b'e', 0x02, b'h', b'2',
    ];

    let mut preface = [0; 24];
    if tokio::io::AsyncReadExt::read_exact(stream, &mut preface)
        .await
        .is_err()
        || preface != CLIENT_PREFACE
    {
        return;
    }
    if tokio::io::AsyncWriteExt::write_all(stream, SETTINGS)
        .await
        .is_err()
    {
        return;
    }

    let mut request_stream_id = None;
    loop {
        let mut frame_header = [0; 9];
        if tokio::io::AsyncReadExt::read_exact(stream, &mut frame_header)
            .await
            .is_err()
        {
            return;
        }
        let payload_len = usize::from(frame_header[0]) << 16
            | usize::from(frame_header[1]) << 8
            | usize::from(frame_header[2]);
        let frame_type = frame_header[3];
        let flags = frame_header[4];
        let stream_id = u32::from_be_bytes([
            frame_header[5] & 0x7f,
            frame_header[6],
            frame_header[7],
            frame_header[8],
        ]);
        let mut payload = vec![0; payload_len];
        if tokio::io::AsyncReadExt::read_exact(stream, &mut payload)
            .await
            .is_err()
        {
            return;
        }
        if frame_type == 4 && flags & 0x1 == 0 {
            if tokio::io::AsyncWriteExt::write_all(stream, SETTINGS_ACK)
                .await
                .is_err()
            {
                return;
            }
        } else if frame_type == 1 && stream_id != 0 {
            request_stream_id = Some(stream_id);
            if flags & 0x4 != 0 {
                break;
            }
        } else if frame_type == 9 && request_stream_id == Some(stream_id) && flags & 0x4 != 0 {
            break;
        }
    }

    let stream_id = request_stream_id.expect("request headers should identify a stream");
    let payload_len = INVALID_RESPONSE_HEADERS.len();
    let mut response = Vec::with_capacity(9 + payload_len);
    response.extend_from_slice(&[
        ((payload_len >> 16) & 0xff) as u8,
        ((payload_len >> 8) & 0xff) as u8,
        (payload_len & 0xff) as u8,
        1,
        0x5,
    ]);
    response.extend_from_slice(&(stream_id & 0x7fff_ffff).to_be_bytes());
    response.extend_from_slice(INVALID_RESPONSE_HEADERS);
    let _ = tokio::io::AsyncWriteExt::write_all(stream, &response).await;
    let _ = tokio::io::AsyncWriteExt::flush(stream).await;

    // Keep the connection alive until libcurl has classified the malformed
    // response. A wall-clock delay is racy under load and can turn the
    // intended HTTP/2 protocol error into a peer-reset error instead.
    wait_for_http2_client_error(stream, stream_id).await;
}

async fn wait_for_http2_client_error(
    stream: &mut tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    request_stream_id: u32,
) {
    loop {
        let mut frame_header = [0; 9];
        if tokio::io::AsyncReadExt::read_exact(stream, &mut frame_header)
            .await
            .is_err()
        {
            break;
        }
        let payload_len = usize::from(frame_header[0]) << 16
            | usize::from(frame_header[1]) << 8
            | usize::from(frame_header[2]);
        let frame_type = frame_header[3];
        let stream_id = u32::from_be_bytes([
            frame_header[5] & 0x7f,
            frame_header[6],
            frame_header[7],
            frame_header[8],
        ]);
        let mut payload = vec![0; payload_len];
        if tokio::io::AsyncReadExt::read_exact(stream, &mut payload)
            .await
            .is_err()
        {
            break;
        }

        const RST_STREAM: u8 = 3;
        const GOAWAY: u8 = 7;
        if (frame_type == RST_STREAM && stream_id == request_stream_id) || frame_type == GOAWAY {
            break;
        }
    }

    let _ = tokio::io::AsyncWriteExt::shutdown(stream).await;
}

async fn send_http11_fallback_response(
    stream: &mut tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
) {
    let mut request = Vec::new();
    let mut buf = [0; 1024];
    loop {
        let read = match tokio::io::AsyncReadExt::read(stream, &mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        request.extend_from_slice(&buf[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let body = b"http1 fallback";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nUpgrade: h2\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    if tokio::io::AsyncWriteExt::write_all(stream, response.as_bytes())
        .await
        .is_ok()
    {
        let _ = tokio::io::AsyncWriteExt::write_all(stream, body).await;
        let _ = tokio::io::AsyncWriteExt::flush(stream).await;
        // Send close_notify, then wait for the peer's half of the TLS close
        // handshake. Dropping a socket with unread peer data can turn this
        // otherwise successful response into an observable TCP reset.
        let _ = tokio::io::AsyncWriteExt::shutdown(stream).await;
        let mut peer_tail = [0; 64];
        let _ = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match tokio::io::AsyncReadExt::read(stream, &mut peer_tail).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        })
        .await;
    }
}

fn h2_tls_acceptor() -> tokio_rustls::TlsAcceptor {
    use tokio_rustls::rustls::{
        ServerConfig,
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    };

    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
        .expect("self-signed h2 certificate");
    let cert_der = CertificateDer::from(cert.cert.der().to_vec());
    let key_der = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()));
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .expect("h2 tls config");
    config.alpn_protocols = vec![b"h2".to_vec()];
    tokio_rustls::TlsAcceptor::from(Arc::new(config))
}

fn http2_and_http11_tls_acceptor() -> tokio_rustls::TlsAcceptor {
    use tokio_rustls::rustls::{
        ServerConfig,
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    };

    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
        .expect("self-signed protocol fallback certificate");
    let cert_der = CertificateDer::from(cert.cert.der().to_vec());
    let key_der = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()));
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .expect("protocol fallback tls config");
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    tokio_rustls::TlsAcceptor::from(Arc::new(config))
}

fn http11_tls_acceptor() -> tokio_rustls::TlsAcceptor {
    use tokio_rustls::rustls::{
        ServerConfig,
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    };

    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
        .expect("self-signed http/1.1 certificate");
    let cert_der = CertificateDer::from(cert.cert.der().to_vec());
    let key_der = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()));
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .expect("http/1.1 tls config");
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    tokio_rustls::TlsAcceptor::from(Arc::new(config))
}

fn run_https11_server(
    listener: std::net::TcpListener,
    shutdown_rx: std_mpsc::Receiver<()>,
    hits: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<String>>>,
    request_heads: Arc<Mutex<Vec<String>>>,
    responses: Arc<Mutex<VecDeque<ScriptedResponse>>>,
) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .expect("http/1.1 tls test runtime");
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::from_std(listener).expect("tokio tls listener");
        let tls_acceptor = http11_tls_acceptor();
        loop {
            if shutdown_rx.try_recv().is_ok() {
                break;
            }
            let accepted = tokio::time::timeout(Duration::from_millis(25), listener.accept()).await;
            let Ok(Ok((stream, _))) = accepted else {
                continue;
            };
            let tls_acceptor = tls_acceptor.clone();
            let hits = Arc::clone(&hits);
            let requests = Arc::clone(&requests);
            let request_heads = Arc::clone(&request_heads);
            let responses = Arc::clone(&responses);
            tokio::spawn(async move {
                let Ok(mut stream) = tls_acceptor.accept(stream).await else {
                    return;
                };
                let mut request = Vec::new();
                let mut buf = [0; 1024];
                loop {
                    let read = match tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => read,
                    };
                    request.extend_from_slice(&buf[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                if let Some(path) = request_path_from_head(&request) {
                    requests.lock().push(path);
                }
                request_heads
                    .lock()
                    .push(String::from_utf8_lossy(&request).into_owned());
                let _ = hits.fetch_add(1, Ordering::SeqCst) + 1;
                let response_spec = {
                    let mut responses = responses.lock();
                    responses
                        .pop_front()
                        .or_else(|| responses.back().cloned())
                        .expect("scripted https responses should not be empty")
                };
                if response_spec.delay_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(response_spec.delay_ms)).await;
                }
                let response = scripted_http_response_bytes(&response_spec);
                let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, &response).await;
                let _ = tokio::io::AsyncWriteExt::flush(&mut stream).await;
                if response_spec.hold_open_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(response_spec.hold_open_ms)).await;
                }
            });
        }
    });
}

impl ScriptedHttpServer {
    pub(super) fn spawn(responses: Vec<ScriptedResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener
            .set_nonblocking(true)
            .expect("test listener should be nonblocking");
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_for_thread = Arc::clone(&hits);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_for_thread = Arc::clone(&requests);
        let (shutdown_tx, shutdown_rx) = std_mpsc::channel();
        let responses = Arc::new(Mutex::new(VecDeque::from(responses)));
        let responses_for_thread = Arc::clone(&responses);
        let handler_handles = Arc::new(Mutex::new(Vec::new()));
        let handler_handles_for_thread = Arc::clone(&handler_handles);

        let join_handle = thread::spawn(move || {
            loop {
                if shutdown_rx.try_recv().is_ok() {
                    break;
                }

                match listener.accept() {
                    Ok((stream, _)) => {
                        let hits_for_handler = Arc::clone(&hits_for_thread);
                        let requests_for_handler = Arc::clone(&requests_for_thread);
                        let responses_for_handler = Arc::clone(&responses_for_thread);
                        let handle = thread::spawn(move || {
                            handle_scripted_connection(
                                stream,
                                hits_for_handler,
                                requests_for_handler,
                                responses_for_handler,
                            );
                        });
                        handler_handles_for_thread.lock().push(handle);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            addr,
            hits,
            requests,
            shutdown_tx,
            join_handle: Some(join_handle),
            handler_handles,
        }
    }

    pub(super) fn url(&self) -> String {
        format!("http://{}/cache", self.addr)
    }

    pub(super) fn origin(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub(super) fn url_path(&self, path: &str) -> String {
        let path = path.strip_prefix('/').unwrap_or(path);
        format!("http://{}/{}", self.addr, path)
    }

    pub(super) fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    pub(super) fn requests(&self) -> Vec<String> {
        self.requests.lock().clone()
    }

    pub(super) fn shutdown(mut self) {
        let _ = self.shutdown_tx.send(());
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
        let mut handler_handles = self.handler_handles.lock();
        for handle in handler_handles.drain(..) {
            let _ = handle.join();
        }
    }
}

fn handle_scripted_connection(
    mut stream: std::net::TcpStream,
    hits: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<String>>>,
    responses: Arc<Mutex<VecDeque<ScriptedResponse>>>,
) {
    let request = read_scripted_http_request(&mut stream);
    let request_text = String::from_utf8_lossy(&request).into_owned();
    requests.lock().push(request_text);
    let _ = hits.fetch_add(1, Ordering::SeqCst) + 1;
    let response_spec = {
        let mut responses = responses.lock();
        responses
            .pop_front()
            .or_else(|| responses.back().cloned())
            .expect("scripted responses should not be empty")
    };
    if response_spec.delay_ms > 0 {
        thread::sleep(Duration::from_millis(response_spec.delay_ms));
    }
    let response = scripted_http_response_bytes(&response_spec);
    let _ = stream.write_all(&response);
    let _ = stream.flush();
    if response_spec.hold_open_ms > 0 {
        thread::sleep(Duration::from_millis(response_spec.hold_open_ms));
    }
}

fn read_scripted_http_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let mut request = Vec::new();
    let mut expected_len = None;
    let mut buffer = [0; 1024];

    loop {
        let bytes_read = match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(bytes_read) => bytes_read,
        };
        request.extend_from_slice(&buffer[..bytes_read]);

        if expected_len.is_none()
            && let Some(head_end) = request
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|offset| offset + 4)
        {
            let content_length = std::str::from_utf8(&request[..head_end])
                .ok()
                .and_then(|head| {
                    head.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                })
                .unwrap_or(0);
            expected_len = Some(head_end.saturating_add(content_length));
        }

        if expected_len.is_some_and(|expected_len| request.len() >= expected_len) {
            break;
        }
    }

    request
}

fn scripted_http_response_bytes(response_spec: &ScriptedResponse) -> Vec<u8> {
    let body = response_spec.body.clone();
    let mut extra_headers = String::new();
    let has_connection_header = response_spec
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("connection"));
    for (name, value) in &response_spec.headers {
        extra_headers.push_str(name);
        extra_headers.push_str(": ");
        extra_headers.push_str(value);
        extra_headers.push_str("\r\n");
    }
    if response_spec.close_connection && !has_connection_header {
        extra_headers.push_str("Connection: close\r\n");
    }
    format!(
        "HTTP/{} {} {}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n{}\r\n{}",
        response_spec.http_version,
        response_spec.status,
        response_spec.reason,
        body.len(),
        extra_headers,
        body
    )
    .into_bytes()
}

fn request_path_from_head(request: &[u8]) -> Option<String> {
    let request = std::str::from_utf8(request).ok()?;
    request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .map(str::to_owned)
}

#[derive(Clone)]
pub(super) struct ScriptedResponse {
    http_version: &'static str,
    status: u16,
    reason: &'static str,
    headers: Vec<(String, String)>,
    body: String,
    delay_ms: u64,
    hold_open_ms: u64,
    close_connection: bool,
}

impl ScriptedResponse {
    pub(super) fn ok(body: &str) -> Self {
        Self {
            http_version: "1.1",
            status: 200,
            reason: "OK",
            headers: Vec::new(),
            body: body.to_owned(),
            delay_ms: 0,
            hold_open_ms: 0,
            close_connection: true,
        }
    }

    pub(super) fn status(status: u16, reason: &'static str) -> Self {
        Self {
            http_version: "1.1",
            status,
            reason,
            headers: Vec::new(),
            body: String::new(),
            delay_ms: 0,
            hold_open_ms: 0,
            close_connection: true,
        }
    }

    pub(super) fn with_body(mut self, body: &str) -> Self {
        self.body = body.to_owned();
        self
    }

    pub(super) fn with_http_version(mut self, http_version: &'static str) -> Self {
        self.http_version = http_version;
        self
    }

    pub(super) fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub(super) fn with_delay_ms(mut self, delay_ms: u64) -> Self {
        self.delay_ms = delay_ms;
        self
    }

    pub(super) fn with_hold_open_ms(mut self, hold_open_ms: u64) -> Self {
        self.hold_open_ms = hold_open_ms;
        self.close_connection = false;
        self
    }
}

pub(super) fn wait_for_runtime_owner_count(client: &FetchClientHandle, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        if client.runtime_owner_count_for_testing() == expected {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(client.runtime_owner_count_for_testing(), expected);
}

pub(super) fn unique_test_cache_dir() -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "moli-fetch-cache-test-{}-{unique}",
        std::process::id()
    ))
}
