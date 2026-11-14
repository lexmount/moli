use super::*;

fn upload_request() -> Result<(Request, crate::RequestHeaders)> {
    let headers = crate::RequestHeaders::from(vec![
        ("Content-Type".into(), "text/plain".into()),
        ("Content-Length".into(), "3".into()),
        ("Content-Encoding".into(), "identity".into()),
        ("Content-Language".into(), "en".into()),
        ("Content-Location".into(), "/payload".into()),
        ("X-Keep".into(), "yes".into()),
    ]);
    Ok((
        Request::new(
            "POST",
            "https://example.test/start",
            Some("abc".into()),
            headers.clone(),
        )?,
        headers,
    ))
}

fn follow(request: &mut Request, status: u16) -> Result<()> {
    let mut redirect = synthetic_redirect(&request.url, &Url::parse("https://example.test/next")?);
    redirect.status = status;
    request.follow_redirect(redirect)
}

fn current(headers: crate::RequestHeaders) -> Option<crate::RequestHeaderOverride> {
    Some(crate::RequestHeaderOverride::current_request(headers, None))
}

fn assert_no_upload_headers(request: &Request) {
    let names = request
        .request_headers
        .iter()
        .map(|(name, _)| name.to_ascii_lowercase())
        .collect::<Vec<_>>();
    assert_eq!(names, ["x-keep"], "stale upload fields must stay removed");
    assert!(request.body.as_ref().is_none_or(bytes::Bytes::is_empty));
}

#[test]
fn noop_method_override_cannot_restore_redirected_upload_headers() -> Result<()> {
    for status in [301, 302, 303] {
        for method in ["GET", "get"] {
            let (mut request, headers) = upload_request()?;
            follow(&mut request, status)?;
            crate::RequestOverrides {
                method: Some(method.into()),
                headers: current(headers),
                ..Default::default()
            }
            .apply(&mut request);
            assert_no_upload_headers(&request);
        }
    }
    Ok(())
}

#[test]
fn removing_or_emptying_body_cannot_restore_redirected_upload_headers() -> Result<()> {
    for body in [None, Some(bytes::Bytes::new())] {
        let (mut request, headers) = upload_request()?;
        follow(&mut request, 302)?;
        crate::RequestOverrides {
            body: Some(body),
            headers: current(headers),
            ..Default::default()
        }
        .apply(&mut request);
        assert_no_upload_headers(&request);
    }
    Ok(())
}

#[test]
fn generated_content_length_replay_cannot_restore_a_redirected_upload() -> Result<()> {
    let (mut request, headers) = upload_request()?;
    request
        .request_headers
        .retain(|(name, _)| !name.eq_ignore_ascii_case("content-length"));
    follow(&mut request, 302)?;
    crate::RequestOverrides {
        method: Some("GET".into()),
        headers: current(headers),
        ..Default::default()
    }
    .apply(&mut request);
    assert_no_upload_headers(&request);
    Ok(())
}

#[test]
fn redirected_request_accepts_new_body_header_values() -> Result<()> {
    let (mut request, _) = upload_request()?;
    follow(&mut request, 302)?;
    crate::RequestOverrides {
        headers: current(
            vec![
                ("Content-Type".into(), "application/x-new".into()),
                ("Content-Language".into(), "fr".into()),
            ]
            .into(),
        ),
        ..Default::default()
    }
    .apply(&mut request);
    assert_eq!(
        request.request_headers.to_byte_strings(),
        [
            ("Content-Type".into(), "application/x-new".into()),
            ("Content-Language".into(), "fr".into())
        ]
    );
    assert!(request.body.is_none());
    Ok(())
}

#[test]
fn repeated_original_header_overrides_stay_removed_across_get_redirects() -> Result<()> {
    let (mut request, headers) = upload_request()?;
    follow(&mut request, 302)?;
    for status in [302, 307, 308] {
        crate::RequestOverrides {
            headers: current(headers.clone()),
            ..Default::default()
        }
        .apply(&mut request);
        assert_no_upload_headers(&request);
        follow(&mut request, status)?;
    }
    Ok(())
}

#[test]
fn dropping_or_changing_override_allows_a_later_header_value() -> Result<()> {
    for fresh in [None, Some("application/x-new")] {
        let (mut request, headers) = upload_request()?;
        follow(&mut request, 302)?;
        crate::RequestOverrides {
            headers: fresh.map(|value| {
                crate::RequestHeaderOverride::current_request(
                    vec![("Content-Type".into(), value.into())].into(),
                    None,
                )
            }),
            ..Default::default()
        }
        .apply(&mut request);
        follow(&mut request, 302)?;
        crate::RequestOverrides {
            headers: current(headers),
            ..Default::default()
        }
        .apply(&mut request);
        assert!(request.request_headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("content-type") && value == b"text/plain"
        }));
    }
    Ok(())
}

#[test]
fn new_upload_can_restore_its_original_content_headers() -> Result<()> {
    let (mut request, headers) = upload_request()?;
    follow(&mut request, 302)?;
    crate::RequestOverrides {
        method: Some("POST".into()),
        body: Some(Some(bytes::Bytes::from_static(b"abc"))),
        headers: current(headers.clone()),
        ..Default::default()
    }
    .apply(&mut request);
    assert_eq!(request.method, "POST");
    assert_eq!(request.body.as_deref(), Some(b"abc".as_slice()));
    assert_eq!(request.request_headers, headers);
    Ok(())
}

#[test]
fn redirected_browser_header_echo_cannot_override_generated_origin() -> Result<()> {
    let source = Url::parse("https://example.test/page")?;
    let mut request = Request::new_browser(
        "POST",
        source.clone(),
        Some(bytes::Bytes::from_static(b"abc")),
        Vec::new(),
        (&source).into(),
    )
    .with_request_mode(RequestMode::Cors);
    let old = request.interception_request_headers();
    assert!(
        old.iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("origin"))
    );
    follow(&mut request, 302)?;
    crate::RequestOverrides {
        headers: current(old),
        ..Default::default()
    }
    .apply(&mut request);
    let wire = crate::blocking::outgoing_request_headers(&FetchConfig::default(), &request, None);
    assert!(
        !wire
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("origin")),
        "{wire:?}"
    );
    Ok(())
}
