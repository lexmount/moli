use web_transport_proto::{ConnectError, ConnectResponse};

// Literal QPACK fields with a zero dynamic-table prefix, carried by one HEADERS
// frame. Keep the peer's bytes independent from the codec under test.
fn response(status: &str, protocol: Option<&str>) -> Vec<u8> {
    let mut fields = vec![0, 0];
    for (name, value) in [(":status", Some(status)), ("wt-protocol", protocol)] {
        let Some(value) = value else { continue };
        let name = name.as_bytes();
        let value = value.as_bytes();
        assert!((7..=127).contains(&name.len()));
        assert!(value.len() < 128);
        fields.extend_from_slice(&[0x27, (name.len() - 7) as u8]);
        fields.extend_from_slice(name);
        fields.push(value.len() as u8);
        fields.extend_from_slice(value);
    }
    assert!(fields.len() < 64);
    let mut wire = vec![1, fields.len() as u8];
    wire.extend_from_slice(&fields);
    wire
}

#[test]
fn legacy_connect_ignores_malformed_optional_protocol_without_losing_status() {
    for status in ["200", "201", "204", "299"] {
        for protocol in ["token", "\"unterminated", "\"a\", \"b\""] {
            let wire = response(status, Some(protocol));
            let decoded = ConnectResponse::decode(&mut wire.as_slice()).unwrap();
            assert_eq!(decoded.status.as_str(), status);
            assert_eq!(decoded.protocol, None, "{protocol}");
        }
    }
}

#[test]
fn legacy_connect_preserves_valid_optional_structured_string() {
    for protocol in [None, Some("\"offered\"")] {
        let wire = response("200", protocol);
        let decoded = ConnectResponse::decode(&mut wire.as_slice()).unwrap();
        assert_eq!(decoded.status, http::StatusCode::OK);
        assert_eq!(decoded.protocol.as_deref(), protocol.map(|_| "offered"));
    }
}

#[test]
fn optional_protocol_tolerance_does_not_accept_error_status_or_broken_qpack() {
    let wire = response("404", Some("token"));
    assert!(matches!(
        ConnectResponse::decode(&mut wire.as_slice()),
        Err(ConnectError::WrongStatus(_))
    ));
    let mut wire = response("200", None);
    wire[2] = 0xff;
    assert!(ConnectResponse::decode(&mut wire.as_slice()).is_err());
}
