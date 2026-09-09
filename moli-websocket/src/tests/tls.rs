use super::*;
use moli_curl::CurlTlsConfig;

fn tls_context(tls: CurlTlsConfig) -> ConnectOptions {
    ConnectOptions {
        tls,
        http_proxy: Some(String::new()),
        http_no_proxy: Some(String::new()),
        ..test_websocket_context()
    }
}

async fn exercise_tls_connection(url: String, context: ConnectOptions, opens: bool) {
    let (events, mut receiver) = mpsc::channel(32);
    let connection = spawn_connection(501, url, Vec::new(), context, events);
    if !opens {
        assert!(
            !recv_handshake_failure_events(&mut receiver)
                .await
                .is_empty()
        );
        return;
    }
    recv_open_event(&mut receiver).await;
    connection
        .send(Command::SendText("TLS echo".to_owned()))
        .unwrap();
    let mut echoed = false;
    loop {
        match timeout(Duration::from_secs(5), receiver.recv())
            .await
            .unwrap()
            .unwrap()
        {
            Event::TextMessage { data, .. } => {
                assert_eq!(data, "TLS echo");
                echoed = true;
                connection
                    .send(Command::Close {
                        code: Some(1000),
                        reason: "done".to_owned(),
                    })
                    .unwrap();
            }
            Event::Close {
                code, was_clean, ..
            } => {
                assert!(echoed);
                assert_eq!(code, 1000);
                assert!(was_clean);
                break;
            }
            Event::Error { message, .. } => panic!("TLS connection failed: {message}"),
            _ => {}
        }
    }
}

#[tokio::test]
async fn websocket_tls_custom_ca_preserves_chain_and_hostname_verification() {
    let fixture = TlsWebSocketFixture::new();
    let unrelated = TlsWebSocketFixture::new();
    for (name, ca, mismatch, opens) in [
        ("trusted", fixture.tls_config().ca_cert, false, true),
        ("missing CA", None, false, false),
        ("wrong CA", unrelated.tls_config().ca_cert, false, false),
        ("wrong hostname", fixture.tls_config().ca_cert, true, false),
    ] {
        let tls = CurlTlsConfig {
            ca_cert: ca,
            ..CurlTlsConfig::default()
        };
        let (mut url, server) = fixture.spawn(false).await;
        if mismatch {
            url = url.replace("localhost", "127.0.0.1");
        }
        exercise_tls_connection(url, tls_context(tls), opens).await;
        let result = server.await.unwrap();
        if opens {
            assert!(
                result.unwrap().is_empty(),
                "{name}: no client identity configured"
            );
        } else {
            assert!(result.is_err(), "{name}: untrusted TLS must fail");
        }
    }
}

#[tokio::test]
async fn websocket_tls_client_identity_supports_certificate_formats_and_passwords() {
    let fixture = TlsWebSocketFixture::new();
    let pem = fixture.tls_config();
    let mut combined = pem.clone();
    combined
        .client_cert
        .as_mut()
        .unwrap()
        .set_file_name("client-combined.pem");
    combined.client_key = None;
    let mut der = pem.clone();
    der.client_cert.as_mut().unwrap().set_extension("der");
    let mut encrypted = pem.clone();
    encrypted
        .client_key
        .as_mut()
        .unwrap()
        .set_file_name("client-encrypted-key.pem");
    let mut pfx = fixture.pkcs12_config();
    pfx.client_cert.as_mut().unwrap().set_extension("PFX");
    let mut insecure = pem.clone();
    insecure.verify = false;
    insecure.ca_cert = None;
    for (name, tls) in [
        ("PEM", pem),
        ("combined PEM", combined),
        ("DER", der),
        ("encrypted PEM key", encrypted),
        ("P12", fixture.pkcs12_config()),
        ("PFX", pfx),
        ("verification disabled", insecure),
    ] {
        let (url, server) = fixture.spawn(true).await;
        exercise_tls_connection(url, tls_context(tls), true).await;
        assert_eq!(
            server.await.unwrap().unwrap(),
            std::slice::from_ref(&fixture.client_certificate),
            "{name}"
        );
    }
}

#[tokio::test]
async fn websocket_tls_rejects_missing_wrong_or_locked_client_identity() {
    let fixture = TlsWebSocketFixture::new();
    let unrelated = TlsWebSocketFixture::new();
    let missing = CurlTlsConfig {
        ca_cert: fixture.tls_config().ca_cert,
        ..CurlTlsConfig::default()
    };
    let mut wrong_identity = unrelated.tls_config();
    wrong_identity.ca_cert = fixture.tls_config().ca_cert;
    let mut wrong_key = fixture.tls_config();
    wrong_key.client_key = unrelated.tls_config().client_key;
    let mut wrong_password = fixture.pkcs12_config();
    wrong_password.client_cert_password = Some("incorrect-password".to_owned());
    let mut missing_password = fixture.pkcs12_config();
    missing_password.client_cert_password = None;
    let mut wrong_pem_password = fixture.tls_config();
    wrong_pem_password
        .client_key
        .as_mut()
        .unwrap()
        .set_file_name("client-encrypted-key.pem");
    wrong_pem_password.client_cert_password = Some("incorrect-password".to_owned());
    for (name, tls) in [
        ("missing identity", missing),
        ("wrong identity", wrong_identity),
        ("mismatched key", wrong_key),
        ("wrong P12 password", wrong_password),
        ("missing P12 password", missing_password),
        ("wrong PEM password", wrong_pem_password),
    ] {
        let (url, server) = fixture.spawn(true).await;
        exercise_tls_connection(url, tls_context(tls), false).await;
        assert!(
            server.await.unwrap().is_err(),
            "{name}: mTLS must not succeed"
        );
    }
}

#[tokio::test]
async fn websocket_tls_credentials_apply_inside_http_connect_tunnel() {
    let fixture = TlsWebSocketFixture::new();
    let (url, server) = fixture.spawn(true).await;
    let (proxy_url, request, proxy) = spawn_http_connect_proxy().await;
    let mut context = tls_context(fixture.pkcs12_config());
    context.http_proxy = Some(proxy_url);
    exercise_tls_connection(url.clone(), context, true).await;
    assert_eq!(
        server.await.unwrap().unwrap(),
        std::slice::from_ref(&fixture.client_certificate)
    );
    let target = Url::parse(&url).unwrap();
    let request = timeout(Duration::from_secs(5), request)
        .await
        .unwrap()
        .unwrap();
    assert!(request.starts_with(&format!(
        "CONNECT localhost:{} HTTP/1.1\r\n",
        target.port().unwrap()
    )));
    proxy.await.unwrap();
}
