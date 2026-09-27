//! Browser-compatible TLS EOF handling must leave HTTP framing and TLS errors intact.

use std::{
    fs,
    io::{Read, Write},
    net::{Shutdown, TcpListener},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use curl::easy::{Easy2, Handler, HttpVersion, WriteError};
use moli_curl::CurlTlsConfig;
use rustls::{ServerConfig, ServerConnection, StreamOwned, pki_types::PrivatePkcs8KeyDer};

const DEADLINE: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug)]
enum Ending {
    CloseNotify,
    TcpEof,
    InvalidRecord,
}

#[derive(Clone, Copy)]
enum Protocol {
    Http1,
    Http2,
}

#[derive(Default)]
struct Body(Vec<u8>);

impl Handler for Body {
    fn write(&mut self, bytes: &[u8]) -> Result<usize, WriteError> {
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
}

fn exchange(
    response: &'static [u8],
    ending: Ending,
    trust_server: bool,
) -> Result<(Result<(), curl::Error>, Vec<u8>)> {
    exchange_protocol(response, ending, trust_server, Protocol::Http1)
}

fn exchange_protocol(
    response: &'static [u8],
    ending: Ending,
    trust_server: bool,
    protocol: Protocol,
) -> Result<(Result<(), curl::Error>, Vec<u8>)> {
    let cert = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()])?;
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.cert.der().clone()],
            PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()).into(),
        )?;
    config.alpn_protocols = vec![match protocol {
        Protocol::Http1 => b"http/1.1".to_vec(),
        Protocol::Http2 => b"h2".to_vec(),
    }];
    let config = Arc::new(config);
    let fixtures = tempfile::tempdir()?;
    let ca = fixtures.path().join("ca.pem");
    fs::write(&ca, cert.cert.pem())?;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    listener.set_nonblocking(true)?;
    let server = thread::spawn(move || -> Result<()> {
        let deadline = Instant::now() + DEADLINE;
        let socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        bail!("TLS fixture did not receive a connection");
                    }
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => return Err(error.into()),
            }
        };
        socket.set_read_timeout(Some(DEADLINE))?;
        socket.set_write_timeout(Some(DEADLINE))?;
        let mut stream = StreamOwned::new(ServerConnection::new(config)?, socket);
        match protocol {
            Protocol::Http1 => {
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    match stream.read(&mut byte) {
                        Ok(0) => bail!("TLS fixture received EOF before request headers"),
                        Ok(_) => head.push(byte[0]),
                        Err(_) if !trust_server => return Ok(()),
                        Err(error) => return Err(error.into()),
                    }
                    if head.len() > 64 * 1024 {
                        bail!("TLS fixture request headers are too large");
                    }
                }
            }
            Protocol::Http2 => {
                let mut preface = [0; 24];
                stream.read_exact(&mut preface)?;
                assert_eq!(&preface, b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
                // Empty server SETTINGS frame, then consume the request's
                // connection frames and first HEADERS frame on stream 1.
                stream.write_all(&[0, 0, 0, 4, 0, 0, 0, 0, 0])?;
                stream.flush()?;
                loop {
                    let mut frame = [0; 9];
                    stream.read_exact(&mut frame)?;
                    let length = u32::from_be_bytes([0, frame[0], frame[1], frame[2]]);
                    assert!(length <= 64 * 1024);
                    stream.read_exact(&mut vec![0; length as usize])?;
                    if frame[3] == 1 {
                        assert_eq!(&frame[5..], &[0, 0, 0, 1]);
                        break;
                    }
                }
            }
        }
        stream.write_all(response)?;
        stream.flush()?;
        match ending {
            Ending::CloseNotify => {
                stream.conn.send_close_notify();
                stream.flush()?;
            }
            Ending::TcpEof => {}
            Ending::InvalidRecord => {
                // A complete TLS record with an invalid authentication tag is
                // a protocol error, distinct from transport EOF after plaintext.
                stream.sock.write_all(&[23, 3, 3, 0, 17])?;
                stream.sock.write_all(&[0x55; 17])?;
                stream.sock.flush()?;
            }
        }
        // Send TCP EOF and drain any in-flight HTTP/2 SETTINGS ACK. Closing
        // a socket with unread data could send RST instead of the intended FIN.
        stream.sock.shutdown(Shutdown::Write)?;
        let mut pending = [0; 4096];
        while matches!(stream.sock.read(&mut pending), Ok(n) if n > 0) {}
        // StreamOwned does not send close_notify on Drop.
        Ok(())
    });

    let mut easy = Easy2::new(Body::default());
    easy.url(&format!("https://{address}/response"))?;
    easy.proxy("")?;
    easy.timeout(DEADLINE)?;
    easy.http_version(match protocol {
        Protocol::Http1 => HttpVersion::V11,
        Protocol::Http2 => HttpVersion::V2,
    })?;
    CurlTlsConfig {
        ca_cert: trust_server.then_some(ca),
        ..CurlTlsConfig::default()
    }
    .configure(&mut easy, false)?;
    let result = easy.perform();
    let body = std::mem::take(&mut easy.get_mut().0);
    drop(easy);
    server.join().expect("TLS fixture thread panicked")?;
    if let Err(error) = &result {
        assert!(
            !error.is_operation_timedout(),
            "TLS fixture timed out: {error}"
        );
    }
    Ok((result, body))
}

#[test]
fn tls_close_delimited_response_accepts_transport_eof() -> Result<()> {
    for ending in [Ending::CloseNotify, Ending::TcpEof] {
        let (result, body) = exchange(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nhello",
            ending,
            true,
        )?;
        assert!(result.is_ok(), "{ending:?}: {result:?}");
        assert_eq!(body, b"hello");
    }
    Ok(())
}

#[test]
fn tls_eof_preserves_http_response_framing() -> Result<()> {
    for ending in [Ending::CloseNotify, Ending::TcpEof] {
        for (response, complete) in [
            (
                &b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello"[..],
                true,
            ),
            (
                &b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\nhello"[..],
                false,
            ),
            (
                &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n"[..],
                true,
            ),
            (
                &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n"[..],
                false,
            ),
        ] {
            let (result, body) = exchange(response, ending, true)?;
            assert_eq!(result.is_ok(), complete, "{ending:?}: {result:?}");
            assert_eq!(body, b"hello");
        }
    }
    Ok(())
}

#[test]
fn tls_eof_accepts_complete_empty_and_informational_responses() -> Result<()> {
    for ending in [Ending::CloseNotify, Ending::TcpEof] {
        for (response, expected) in [
            (&b"HTTP/1.0 200 OK\r\n\r\nhello"[..], &b"hello"[..]),
            (&b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"[..], &b""[..]),
            (&b"HTTP/1.1 204 No Content\r\n\r\n"[..], &b""[..]),
            (&b"HTTP/1.1 103 Early Hints\r\nLink: </style.css>; rel=preload\r\n\r\nHTTP/1.1 200 OK\r\n\r\nhello"[..], &b"hello"[..]),
        ] {
            let (result, body) = exchange(response, ending, true)?;
            assert!(result.is_ok(), "{ending:?}: {result:?}");
            assert_eq!(body, expected);
        }
    }
    Ok(())
}

#[test]
fn tls_eof_does_not_hide_invalid_tls_records() -> Result<()> {
    let (result, _) = exchange(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nhello",
        Ending::InvalidRecord,
        true,
    )?;
    assert!(result.is_err(), "invalid TLS record accepted");
    Ok(())
}

#[test]
fn tls_eof_rejects_incomplete_response_headers() -> Result<()> {
    for ending in [Ending::CloseNotify, Ending::TcpEof] {
        for response in [
            &b""[..],
            &b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n"[..],
            &b"HTTP/1.1 103 Early Hints\r\nLink: </style.css>; rel=preload\r\n\r\n"[..],
        ] {
            let (result, body) = exchange(response, ending, true)?;
            assert!(
                result.is_err(),
                "{ending:?}: incomplete headers accepted: {response:?}"
            );
            assert!(body.is_empty());
        }
    }
    Ok(())
}

#[test]
fn tls_eof_preserves_http2_stream_framing() -> Result<()> {
    for ending in [Ending::CloseNotify, Ending::TcpEof] {
        // HEADERS with END_HEADERS and HPACK :status 200, followed by DATA.
        // Only the first response includes the required END_STREAM flag.
        for (response, complete) in [
            (&b"\x00\x00\x01\x01\x04\x00\x00\x00\x01\x88\x00\x00\x05\x00\x01\x00\x00\x00\x01hello"[..], true),
            (&b"\x00\x00\x01\x01\x04\x00\x00\x00\x01\x88\x00\x00\x05\x00\x00\x00\x00\x00\x01hello"[..], false),
        ] {
            let (result, body) = exchange_protocol(response, ending, true, Protocol::Http2)?;
            assert_eq!(result.is_ok(), complete, "{ending:?}: {result:?}");
            assert_eq!(body, b"hello");
        }
    }
    Ok(())
}

#[test]
fn tls_eof_does_not_hide_certificate_errors() -> Result<()> {
    let (result, body) = exchange(b"HTTP/1.1 200 OK\r\n\r\nhello", Ending::TcpEof, false)?;
    assert!(
        result.is_err_and(|error| error.is_peer_failed_verification()),
        "untrusted self-signed server certificate must fail verification"
    );
    assert!(body.is_empty());
    Ok(())
}
