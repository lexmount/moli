use super::*;
use web_transport_quinn::proto::{self, Frame, VarInt};

/// A real peer with direct control of CONNECT response and capsule bytes.
/// Application byte streams still use the HTTP/3 WebTransport framing.
struct Peer {
    connection: quinn::Connection,
    send: quinn::SendStream,
    _recv: quinn::RecvStream,
    _control_send: quinn::SendStream,
    _control_recv: quinn::RecvStream,
    request: proto::ConnectRequest,
}

impl Peer {
    async fn accept(endpoint: &quinn::Endpoint) -> Self {
        let connection = endpoint.accept().await.unwrap().await.unwrap();
        let sending = async {
            let mut send = connection.open_uni().await.unwrap();
            let mut settings = proto::Settings::default();
            settings.enable_webtransport(1);
            settings.write(&mut send).await.unwrap();
            send
        };
        let receiving = async {
            let mut recv = connection.accept_uni().await.unwrap();
            let settings = proto::Settings::read(&mut recv).await.unwrap();
            assert!(settings.supports_webtransport() > 0);
            recv
        };
        let (control_send, control_recv) = tokio::join!(sending, receiving);
        let (send, mut recv) = connection.accept_bi().await.unwrap();
        let request = proto::ConnectRequest::read(&mut recv).await.unwrap();
        assert_eq!(request.headers[http::header::ORIGIN], "https://origin.test");
        Self {
            connection,
            send,
            _recv: recv,
            _control_send: control_send,
            _control_recv: control_recv,
            request,
        }
    }

    async fn respond(&mut self, status: &str, protocol: Option<&str>) {
        self.send
            .write_all(&response(status, protocol))
            .await
            .unwrap();
    }

    async fn echo(&self) {
        let (mut send, mut recv) = self.connection.accept_bi().await.unwrap();
        let mut header = [0; 3];
        recv.read_exact(&mut header).await.unwrap();
        assert_eq!(header, [0x40, 0x41, 0]);
        let data = recv.read_to_end(128 * 1024).await.unwrap();
        assert_eq!(data, b"HTTP/3 compatibility\0\xff");
        send.write_all(&data).await.unwrap();
        send.finish().unwrap();
        self.connection.closed().await;
    }
}

// RFC 9204 literal field lines with no Huffman or dynamic table references.
// Keeping the response as bytes permits malformed optional Structured Fields
// and mandatory HTTP/3 errors to be tested through the actual client parser.
fn integer(bytes: &mut Vec<u8>, value: usize, prefix: u8, bits: u8) {
    let maximum = (1usize << bits) - 1;
    bytes.push(prefix | value.min(maximum) as u8);
    if value < maximum {
        return;
    }
    let mut rest = value - maximum;
    while rest >= 128 {
        bytes.push((rest as u8 & 0x7f) | 0x80);
        rest >>= 7;
    }
    bytes.push(rest as u8);
}

fn field(bytes: &mut Vec<u8>, name: &str, value: &str) {
    integer(bytes, name.len(), 0x20, 3);
    bytes.extend(name.as_bytes());
    integer(bytes, value.len(), 0, 7);
    bytes.extend(value.as_bytes());
}

fn frame(kind: Frame, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    kind.encode(&mut bytes);
    VarInt::try_from(payload.len() as u64)
        .unwrap()
        .encode(&mut bytes);
    bytes.extend(payload);
    bytes
}

fn response(status: &str, protocol: Option<&str>) -> Vec<u8> {
    let mut headers = vec![0, 0];
    field(&mut headers, ":status", status);
    if let Some(protocol) = protocol {
        field(&mut headers, "wt-protocol", protocol);
    }
    frame(Frame::HEADERS, &headers)
}

async fn exchange(session: &crate::Session) {
    let (mut send, mut recv) = session.open_bi().await.unwrap();
    send.write_all(b"HTTP/3 compatibility\0\xff").await.unwrap();
    send.finish().unwrap();
    assert_eq!(
        recv.read_to_end(128 * 1024).await.unwrap(),
        b"HTTP/3 compatibility\0\xff"
    );
    session.cancel();
}

#[tokio::test]
async fn every_successful_connect_status_establishes_a_usable_session() {
    timeout(DEADLINE, async {
        for status in ["200", "201", "202", "204", "206", "299"] {
            let fixture = Fixture::new();
            let endpoint = fixture.endpoint.clone();
            let peer = tokio::spawn(async move {
                let mut peer = Peer::accept(&endpoint).await;
                peer.respond(status, None).await;
                peer.echo().await;
            });
            let owner = Owner::new(fixture.config.clone());
            let session = owner
                .connector()
                .connect(fixture.options.clone())
                .await
                .unwrap();
            exchange(&session).await;
            peer.await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn optional_protocol_metadata_preserves_only_an_offered_structured_string() {
    timeout(DEADLINE, async {
        for (offered, received, expected) in [
            (true, Some("\"b\""), Some("b")),
            (true, Some("\"b\"; a=c; c=2"), Some("b")),
            (true, Some("\"d\""), None),
            (false, Some("\"d\""), None),
            (true, Some("b"), None),
            (true, Some("42"), None),
            (true, Some("\"a\", \"b\""), None),
            (true, None, None),
        ] {
            let fixture = Fixture::new();
            let endpoint = fixture.endpoint.clone();
            let peer = tokio::spawn(async move {
                let mut peer = Peer::accept(&endpoint).await;
                assert_eq!(
                    peer.request.protocols,
                    if offered { vec!["a", "b", "c"] } else { vec![] }
                );
                peer.respond("200", received).await;
                peer.echo().await;
            });
            let mut options = fixture.options.clone();
            if offered {
                options.protocols = ["a", "b", "c"].map(str::to_owned).to_vec();
            }
            let owner = Owner::new(fixture.config.clone());
            let session = owner.connector().connect(options).await.unwrap();
            assert_eq!(session.protocol(), expected);
            exchange(&session).await;
            peer.await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn unsuccessful_status_or_broken_qpack_is_not_accepted_as_a_session() {
    timeout(DEADLINE, async {
        for bytes in [
            response("301", None),
            response("401", None),
            response("404", None),
            response("500", None),
            response("invalid", None),
            frame(Frame::HEADERS, &[0, 0, 0x80]),
        ] {
            let fixture = Fixture::new();
            let endpoint = fixture.endpoint.clone();
            let peer = tokio::spawn(async move {
                let mut peer = Peer::accept(&endpoint).await;
                peer.send.write_all(&bytes).await.unwrap();
                peer.connection.closed().await;
            });
            let owner = Owner::new(fixture.config.clone());
            assert!(
                owner
                    .connector()
                    .connect(fixture.options.clone())
                    .await
                    .is_err()
            );
            peer.await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn clean_connect_fin_without_a_capsule_has_remote_default_close_metadata() {
    timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let endpoint = fixture.endpoint.clone();
        let peer = tokio::spawn(async move {
            let mut peer = Peer::accept(&endpoint).await;
            peer.respond("200", None).await;
            peer.send.finish().unwrap();
            peer.connection.closed().await;
        });
        let owner = Owner::new(fixture.config.clone());
        let session = owner.connector().connect(fixture.options.clone()).await.unwrap();
        assert!(matches!(session.closed().await, CloseReason::Remote(info) if info == CloseInfo {code:0,reason:String::new()}));
        peer.await.unwrap();
    }).await.unwrap();
}

#[tokio::test]
async fn truncated_capsules_or_connect_reset_keep_a_real_failure_cause() {
    timeout(DEADLINE, async {
        for bytes in [
            Some(vec![0]),
            Some(vec![0, 8, 1]),
            Some(frame(Frame::DATA, &[0x68, 0x43, 4, 0])),
            None,
        ] {
            let fixture = Fixture::new();
            let endpoint = fixture.endpoint.clone();
            let (established, ready) = tokio::sync::oneshot::channel();
            let peer = tokio::spawn(async move {
                let mut peer = Peer::accept(&endpoint).await;
                peer.respond("200", None).await;
                // A RESET sent before HEADERS are consumed can legitimately
                // fail the handshake instead of terminating an established session.
                ready.await.unwrap();
                if let Some(bytes) = bytes {
                    peer.send.write_all(&bytes).await.unwrap();
                    peer.send.finish().unwrap();
                } else {
                    peer.send.reset(quinn::VarInt::from_u32(0x100)).unwrap();
                }
                peer.connection.closed().await;
            });
            let owner = Owner::new(fixture.config.clone());
            let session = owner
                .connector()
                .connect(fixture.options.clone())
                .await
                .unwrap();
            established.send(()).unwrap();
            let CloseReason::Failed(SessionError::WebTransportError(
                WebTransportError::CapsuleError(cause),
            )) = session.closed().await
            else {
                panic!("truncated or reset CONNECT must retain the capsule failure");
            };
            assert!(!cause.is_empty());
            peer.await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn fragmented_close_capsule_after_unknown_capsule_preserves_metadata() {
    timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let endpoint = fixture.endpoint.clone();
        let peer = tokio::spawn(async move {
            let mut peer = Peer::accept(&endpoint).await;
            peer.respond("200", None).await;
            let mut capsules = Vec::new();
            proto::Capsule::Unknown {typ:VarInt::from_u32(0x1234),payload:Bytes::from_static(b"ignored")}.encode(&mut capsules);
            proto::Capsule::CloseWebTransportSession {code:73,reason:"remote \u{754c}".into()}.encode(&mut capsules);
            for bytes in capsules.chunks(3) {
                peer.send.write_all(&frame(Frame::DATA, bytes)).await.unwrap();
            }
            peer.send.finish().unwrap();
            peer.connection.closed().await;
        });
        let owner = Owner::new(fixture.config.clone());
        let session = owner.connector().connect(fixture.options.clone()).await.unwrap();
        assert!(matches!(session.closed().await, CloseReason::Remote(info) if info == CloseInfo {code:73,reason:"remote \u{754c}".into()}));
        peer.await.unwrap();
    }).await.unwrap();
}
