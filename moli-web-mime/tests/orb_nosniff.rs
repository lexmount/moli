use moli_web_mime::should_opaque_response_be_blocked_by_orb_with_body;

#[test]
fn orb_body_sniffing_cannot_override_nosniff_for_blocklisted_types() {
    for (content_type, body) in [
        ("text/html", b"\x89PNG\r\n\x1A\nrest".as_slice()),
        ("text/plain", b"\x89PNG\r\n\x1A\nrest".as_slice()),
        (
            "application/json",
            b"\"use strict\";function f(){}".as_slice(),
        ),
    ] {
        let headers = vec![
            ("Content-Type".to_owned(), content_type.as_bytes().to_vec()),
            ("X-Content-Type-Options".to_owned(), b"nosniff".to_vec()),
        ];
        assert!(
            should_opaque_response_be_blocked_by_orb_with_body(&headers, body),
            "{content_type}"
        );
    }
}

#[test]
fn nosniff_does_not_block_types_already_allowed_by_the_header_policy() {
    for content_type in ["image/png", "application/javascript", "text/css"] {
        let headers = vec![
            ("Content-Type".to_owned(), content_type.as_bytes().to_vec()),
            ("X-Content-Type-Options".to_owned(), b"nosniff".to_vec()),
        ];
        assert!(
            !should_opaque_response_be_blocked_by_orb_with_body(&headers, b"body"),
            "{content_type}"
        );
    }
}
