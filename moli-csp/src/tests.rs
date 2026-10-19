use super::*;
use crate::policy::{
    content_security_policy_header_values, content_security_policy_headers,
    content_security_policy_report_only_headers,
};
use crate::reporting::*;

fn protected_url() -> Url {
    Url::parse("https://app.test/page.html").unwrap()
}

fn request_url(raw: &str) -> Url {
    Url::parse(raw).unwrap()
}

fn allowed(policy: &str, kind: ContentSecurityPolicyResourceKind, request: &str) -> bool {
    content_security_policy_allows_url(
        &[policy.to_owned().into()],
        &protected_url(),
        &request_url(request),
        kind,
    )
}

fn script_element_request_allowed(
    policy: &str,
    request: ContentSecurityPolicyScriptElementRequest<'_>,
) -> bool {
    content_security_policy_script_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
        &[policy.to_owned().into()],
        &protected_url(),
        &request_url("https://cdn.test/script.js"),
        ContentSecurityPolicyRedirectStatus::NoRedirect,
        ContentSecurityPolicyDisposition::Enforce,
        &ContentSecurityPolicyReportingEndpoints::default(),
        request,
    )
    .is_none()
}

fn frame_ancestors_violation(
    policies: &[&str],
    protected_url: &str,
    ancestor_origins: &[Option<&str>],
) -> Option<ContentSecurityPolicyUrlViolation> {
    content_security_policy_frame_ancestors_violation_with_disposition_and_reporting_endpoints(
        &policies
            .iter()
            .map(|policy| (*policy).into())
            .collect::<Vec<_>>(),
        &request_url(protected_url),
        &ancestor_origins
            .iter()
            .map(|ancestor| ancestor.map(request_url))
            .collect::<Vec<_>>(),
        ContentSecurityPolicyDisposition::Enforce,
        &ContentSecurityPolicyReportingEndpoints::default(),
    )
}

#[test]
fn content_security_policy_headers_collect_enforce_headers_only() {
    let headers: Vec<(String, Vec<u8>)> = vec![
        (
            "Content-Security-Policy-Report-Only".to_owned(),
            b"worker-src 'none'".to_vec(),
        ),
        (
            "content-security-policy".to_owned(),
            b" worker-src 'self' ".to_vec(),
        ),
        ("Content-Security-Policy".to_owned(), Vec::new()),
    ];

    assert_eq!(
        content_security_policy_headers(&headers)
            .iter()
            .map(|policy| policy.as_str())
            .collect::<Vec<_>>(),
        vec!["worker-src 'self'".to_owned()]
    );
    assert_eq!(
        content_security_policy_report_only_headers(&headers)
            .iter()
            .map(|policy| policy.as_str())
            .collect::<Vec<_>>(),
        vec!["worker-src 'none'".to_owned()]
    );
}

#[test]
fn csp_headers_preserve_policy_order_and_disposition() {
    let headers: Vec<(String, Vec<u8>)> = vec![
        (
            "CONTENT-SECURITY-POLICY".to_owned(),
            b" , img-src *; report-uri /first, img-src 'none'; report-uri /second, ".to_vec(),
        ),
        (
            "Content-Security-Policy-Report-Only".to_owned(),
            b"script-src * , script-src 'none'".to_vec(),
        ),
        (
            "content-security-policy".to_owned(),
            b"worker-src 'self'".to_vec(),
        ),
    ];
    assert_eq!(
        content_security_policy_headers(&headers)
            .iter()
            .map(|policy| policy.as_str())
            .collect::<Vec<_>>(),
        [
            "img-src *; report-uri /first",
            "img-src 'none'; report-uri /second",
            "worker-src 'self'",
        ]
    );
    assert_eq!(
        content_security_policy_report_only_headers(&headers)
            .iter()
            .map(|policy| policy.as_str())
            .collect::<Vec<_>>(),
        ["script-src *", "script-src 'none'"]
    );
}

#[test]
fn csp_header_policy_lists_enforce_every_policy() {
    for (value, request, expected) in [
        (
            "img-src * , img-src 'none'",
            "https://cdn.test/asset",
            false,
        ),
        ("img-src *,img-src 'none'", "https://cdn.test/asset", false),
        (
            "img-src *; ignored first, img-src 'none'; ignored second",
            "https://cdn.test/asset",
            false,
        ),
        (
            "img-src https://cdn.test/foo,ignored",
            "https://cdn.test/foo",
            true,
        ),
        (
            "img-src https://cdn.test/foo,ignored",
            "https://cdn.test/foo,ignored",
            false,
        ),
        (
            "img-src https://cdn.test/foo%2cbar",
            "https://cdn.test/foo%2cbar",
            true,
        ),
        (
            "img-src https://cdn.test/foo%2cbar",
            "https://cdn.test/foo,bar",
            true,
        ),
        (
            ", , img-src * , , img-src 'none', ,",
            "https://cdn.test/asset",
            false,
        ),
        (
            "img-src *; img-src 'none', img-src *",
            "https://cdn.test/asset",
            true,
        ),
        (
            "img-src https://cdn.test/foo, img-src https://cdn.test/bar",
            "https://cdn.test/foo",
            false,
        ),
        (
            "img-src https://cdn.test/foo, img-src https://cdn.test/bar",
            "https://cdn.test/bar",
            false,
        ),
        (
            "img-src *; ignored \"start, img-src 'none'; ignored end\"",
            "https://cdn.test/asset",
            false,
        ),
    ] {
        let policies = content_security_policy_headers(&[(
            "Content-Security-Policy".to_owned(),
            value.as_bytes().to_vec(),
        )]);
        assert_eq!(
            content_security_policy_allows_url(
                &policies,
                &protected_url(),
                &request_url(request),
                ContentSecurityPolicyResourceKind::DocumentImage,
            ),
            expected,
            "{value:?} with {request}"
        );
    }
}

#[test]
fn csp_header_lists_keep_report_only_violations_with_their_policy() {
    let headers: Vec<(String, Vec<u8>)> = vec![
        ("Content-Security-Policy".to_owned(), b"img-src *".to_vec()),
        (
            "Content-Security-Policy-Report-Only".to_owned(),
            b"img-src *; report-uri /allowed, img-src 'none'; report-uri /blocked".to_vec(),
        ),
    ];
    let protected = protected_url();
    let request = request_url("https://cdn.test/asset");
    assert!(content_security_policy_allows_url(
        &content_security_policy_headers(&headers),
        &protected,
        &request,
        ContentSecurityPolicyResourceKind::DocumentImage,
    ));
    let violation = content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints(
        &content_security_policy_report_only_headers(&headers),
        &protected,
        &request,
        ContentSecurityPolicyResourceKind::DocumentImage,
        ContentSecurityPolicyRedirectStatus::NoRedirect,
        ContentSecurityPolicyDisposition::Report,
        &ContentSecurityPolicyReportingEndpoints::default(),
    ).expect("the second report-only policy must produce a violation");
    assert_eq!(
        violation.disposition,
        ContentSecurityPolicyDisposition::Report
    );
    assert_eq!(
        violation.original_policy,
        "img-src 'none'; report-uri /blocked"
    );
    assert_eq!(violation.report_uri_endpoints, ["https://app.test/blocked"]);
}

#[test]
fn csp_header_lists_preserve_invalid_whitespace_and_skip_empty_policies() {
    for name in [
        "content-security-policy",
        "content-security-policy-report-only",
    ] {
        let collect = |value: &[u8]| {
            content_security_policy_header_values(&[(name.to_owned(), value.to_vec())], name)
        };
        assert!(collect(b", ; ; , \t , img-src 'none'\xa0,").is_empty());
        assert_eq!(
            collect(b"\t\n\r\x0c img-src 'none' , \t")
                .iter()
                .map(|policy| policy.as_str())
                .collect::<Vec<_>>(),
            ["img-src 'none'"]
        );
        assert_eq!(
            collect(b"img-src *\x0b, img-src 'none'")
                .iter()
                .map(|policy| policy.as_str())
                .collect::<Vec<_>>(),
            ["img-src 'none'"]
        );
        let policies = collect(b"img-src *\x0b");
        assert!(policies.is_empty());
        assert!(content_security_policy_allows_url(
            &policies,
            &protected_url(),
            &request_url("https://cdn.test/asset"),
            ContentSecurityPolicyResourceKind::DocumentImage,
        ));
        assert!(!content_security_policy_allows_url(
            &collect(b"img-src *\x0b, img-src 'none'"),
            &protected_url(),
            &request_url("https://cdn.test/asset"),
            ContentSecurityPolicyResourceKind::DocumentImage,
        ));
    }
}

#[test]
fn csp_single_directive_values_reject_commas() {
    let directives = ContentSecurityPolicy::from("img-src https://cdn.test/a,b; script-src 'none'");
    assert!(directive_source_list(&directives, IMG_SRC).is_none());
    assert_eq!(
        directive_source_list(&directives, SCRIPT_SRC),
        Some(vec!["'none'"])
    );
}

#[test]
fn csp_invalid_directive_characters_do_not_block_resources_or_discard_other_directives() {
    for invalid in ('\0'..='\u{0008}')
        .chain(['\u{000b}'])
        .chain('\u{000e}'..='\u{001f}')
        .chain(['\u{007f}', '\u{0080}', '\u{00a0}', '\u{feff}', '中'])
    {
        for value in [
            format!("{invalid}'none'"),
            format!("'no{invalid}ne'"),
            format!("'none'{invalid}"),
        ] {
            let policy = format!("img-src {value}; script-src 'none'; report-uri /blocked");
            let policies = [policy.clone().into()];
            for disposition in [
                ContentSecurityPolicyDisposition::Enforce,
                ContentSecurityPolicyDisposition::Report,
            ] {
                let violation = |kind| {
                    content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints(
                        &policies,
                        &protected_url(),
                        &request_url("https://cdn.test/asset"),
                        kind,
                        ContentSecurityPolicyRedirectStatus::NoRedirect,
                        disposition,
                        &ContentSecurityPolicyReportingEndpoints::default(),
                    )
                };
                assert!(
                    violation(ContentSecurityPolicyResourceKind::DocumentImage).is_none(),
                    "{policy:?}",
                );
                let blocked = violation(ContentSecurityPolicyResourceKind::DocumentScriptElement)
                    .expect("the valid script-src directive must still block");
                assert_eq!(blocked.effective_directive, SCRIPT_SRC_ELEM);
                assert_eq!(blocked.disposition, disposition);
                assert_eq!(blocked.original_policy, policy);
                assert_eq!(blocked.report_uri_endpoints, ["https://app.test/blocked"]);
            }
        }
    }
}

#[test]
fn csp_invalid_directives_preserve_fallbacks_and_trusted_types_requirements() {
    for invalid in ['\0', '\u{000b}', '\u{001f}', '\u{007f}', '\u{00a0}'] {
        let policy = format!(
            "default-src 'self'; img-src *{invalid}; \
             script-src 'unsafe-eval'{invalid}; trusted-types *{invalid}; \
             require-trusted-types-for 'script'"
        );
        assert!(!allowed(
            &policy,
            ContentSecurityPolicyResourceKind::DocumentImage,
            "https://cdn.test/asset",
        ));
        assert!(allowed(
            &policy,
            ContentSecurityPolicyResourceKind::DocumentImage,
            "https://app.test/asset",
        ));
        assert!(
            content_security_policy_non_url_violation_with_source(
                &ContentSecurityPolicy::from(policy.as_str()),
                &protected_url(),
                ContentSecurityPolicyNonUrlKind::Eval,
                None,
                ContentSecurityPolicyDisposition::Enforce,
                &ContentSecurityPolicyReportingEndpoints::default(),
            )
            .is_some(),
            "{policy:?}",
        );
        assert!(policy_requires_trusted_types_for_script(
            &ContentSecurityPolicy::from(policy.as_str())
        ));
        for duplicate in [false, true] {
            assert!(policy_allows_trusted_type_policy_name(
                &ContentSecurityPolicy::from(policy.as_str()),
                "unlisted",
                duplicate,
            ));
        }

        let policy = format!(
            "trusted-types allowed; require-trusted-types-for 'script'{invalid}; img-src 'none'"
        );
        assert!(!policy_requires_trusted_types_for_script(
            &ContentSecurityPolicy::from(policy.as_str())
        ));
        assert!(policy_allows_trusted_type_policy_name(
            &ContentSecurityPolicy::from(policy.as_str()),
            "allowed",
            false
        ));
        assert!(!policy_allows_trusted_type_policy_name(
            &ContentSecurityPolicy::from(policy.as_str()),
            "unlisted",
            false
        ));
        assert!(!allowed(
            &policy,
            ContentSecurityPolicyResourceKind::DocumentImage,
            "https://app.test/asset",
        ));
    }
}

#[test]
fn csp_invalid_first_values_do_not_activate_later_duplicate_directives() {
    for invalid in ['\0', '\u{000b}', '\u{001f}', '\u{007f}', '\u{00a0}', '中'] {
        let policy = format!("default-src 'none'; img-src *{invalid}; IMG-SRC *");
        assert!(!allowed(
            &policy,
            ContentSecurityPolicyResourceKind::DocumentImage,
            "https://cdn.test/asset",
        ));
        let policy = format!("img-src 'none'{invalid}; IMG-SRC 'none'");
        assert!(allowed(
            &policy,
            ContentSecurityPolicyResourceKind::DocumentImage,
            "https://cdn.test/asset",
        ));
        for policy in [
            format!("default-src 'none'; img-src{invalid} 'none'; img-src *"),
            format!("default-src 'none'; {invalid}img-src 'none'; img-src *"),
        ] {
            assert!(
                allowed(
                    &policy,
                    ContentSecurityPolicyResourceKind::DocumentImage,
                    "https://cdn.test/asset",
                ),
                "{policy:?}"
            );
        }
        let policy = format!("trusted-types *{invalid}; TRUSTED-TYPES allowed");
        for duplicate in [false, true] {
            assert!(policy_allows_trusted_type_policy_name(
                &ContentSecurityPolicy::from(policy.as_str()),
                "unlisted",
                duplicate,
            ));
        }
        let policy = format!("trusted-types allowed; TRUSTED-TYPES *{invalid}");
        assert!(policy_allows_trusted_type_policy_name(
            &ContentSecurityPolicy::from(policy.as_str()),
            "allowed",
            false
        ));
        assert!(!policy_allows_trusted_type_policy_name(
            &ContentSecurityPolicy::from(policy.as_str()),
            "unlisted",
            false
        ));
        assert!(!policy_allows_trusted_type_policy_name(
            &ContentSecurityPolicy::from(policy.as_str()),
            "allowed",
            true,
        ));
    }
}

#[test]
fn csp_keywords_do_not_strip_control_characters_from_source_tokens() {
    use ContentSecurityPolicyNonUrlKind::*;
    for (kind, keyword) in [
        (DocumentInlineScript, "unsafe-inline"),
        (DocumentInlineStyleElement, "unsafe-inline"),
        (DocumentInlineEventHandler, "unsafe-inline"),
        (DocumentInlineNavigation, "unsafe-inline"),
        (DocumentInlineStyleAttribute, "unsafe-inline"),
        (Eval, "unsafe-eval"),
        (TrustedTypesEval, "unsafe-eval"),
        (TrustedTypesEval, "trusted-types-eval"),
        (WasmEval, "unsafe-eval"),
        (WasmEval, "wasm-unsafe-eval"),
    ] {
        for padding in [
            "",
            " \t\n\r\u{000c}",
            "\0",
            "\u{000b}",
            "\u{001f}",
            "\u{007f}",
        ] {
            let valid = padding.is_empty() || padding.starts_with(' ');
            for source in [
                format!("{padding}'{keyword}'"),
                format!("'{keyword}'{padding}"),
            ] {
                let policy = format!(
                    "default-src 'none'; {} {source}",
                    kind.effective_directive()
                );
                let violation = content_security_policy_non_url_violation_with_source(
                    &ContentSecurityPolicy::from(policy.as_str()),
                    &protected_url(),
                    kind,
                    None,
                    ContentSecurityPolicyDisposition::Enforce,
                    &ContentSecurityPolicyReportingEndpoints::default(),
                );
                assert_eq!(violation.is_none(), valid, "{kind:?}: {policy:?}");
            }
        }
    }
}

#[test]
fn csp_nonce_and_hash_tokens_preserve_their_boundaries() {
    let source = "t1.done();";
    let integrity = "sha256-wmuLCpoj8EMqfQlPnt5NIMgKkCK62CxAkAiewI0zZps=";
    for expression in ["'nonce-abc'".to_owned(), format!("'{integrity}'")] {
        for (prefix, suffix, valid) in [
            ("", "", true),
            ("\t\n", " \r\u{000c}", true),
            ("\u{000b}", "", false),
            ("", "\u{000b}", false),
        ] {
            let token = format!("{prefix}{expression}{suffix}");
            let policy = format!("default-src 'none'; script-src {token}; style-src {token}");
            let request = ContentSecurityPolicyScriptElementRequest {
                nonce: Some("abc"),
                integrity: Some(integrity),
                parser_inserted: true,
            };
            assert_eq!(
                script_element_request_allowed(&policy, request),
                valid,
                "{policy:?}"
            );
            let endpoints = ContentSecurityPolicyReportingEndpoints::default();
            let script = content_security_policy_inline_script_element_violation_with_disposition_and_reporting_endpoints(
                &ContentSecurityPolicy::from(policy.as_str()), &protected_url(), source, request,
                ContentSecurityPolicyDisposition::Enforce, &endpoints,
            );
            assert_eq!(script.is_none(), valid, "inline script: {policy:?}");
            let style = content_security_policy_inline_style_element_violation_with_disposition_and_reporting_endpoints(
                &ContentSecurityPolicy::from(policy.as_str()), &protected_url(), source,
                ContentSecurityPolicyStyleElementRequest { nonce: Some("abc") },
                ContentSecurityPolicyDisposition::Enforce, &endpoints,
            );
            assert_eq!(style.is_none(), valid, "inline style: {policy:?}");

            // Invalid nonce/hash tokens must not disable unsafe-inline or
            // activate strict-dynamic and erase a URL allowlist either.
            let inline_sources = format!("'unsafe-inline' {token}");
            let inline_sources: Vec<_> = inline_sources.split_ascii_whitespace().collect();
            assert_eq!(
                inline_event_handler_source_list_allows(&inline_sources, "untrusted()"),
                !valid,
                "{inline_sources:?}",
            );
            let dynamic_sources = format!("'strict-dynamic' https://cdn.test {token}");
            let dynamic_sources: Vec<_> = dynamic_sources.split_ascii_whitespace().collect();
            assert_eq!(
                source_list_activates_strict_dynamic(&dynamic_sources),
                valid,
                "{dynamic_sources:?}",
            );
        }
    }
}

#[test]
fn csp_unsafe_hashes_requires_an_exact_keyword_and_hash_token() {
    let hash = "'sha256-wmuLCpoj8EMqfQlPnt5NIMgKkCK62CxAkAiewI0zZps='";
    for (sources, allowed) in [
        (format!("'unsafe-hashes' {hash}"), true),
        (format!("\u{000b}'unsafe-hashes' {hash}"), false),
        (format!("'unsafe-hashes'\u{000b} {hash}"), false),
        (format!("'unsafe-hashes' \u{000b}{hash}"), false),
        (format!("'unsafe-hashes' {hash}\u{000b}"), false),
    ] {
        for kind in [
            ContentSecurityPolicyNonUrlKind::DocumentInlineEventHandler,
            ContentSecurityPolicyNonUrlKind::DocumentInlineStyleAttribute,
        ] {
            let violation = content_security_policy_inline_source_violation_with_disposition_and_reporting_endpoints(
                &format!("default-src 'none'; {} {sources}", kind.effective_directive()).into(),
                &protected_url(), kind, "t1.done();",
                ContentSecurityPolicyDisposition::Enforce,
                &ContentSecurityPolicyReportingEndpoints::default(),
            );
            assert_eq!(violation.is_none(), allowed, "{kind:?}: {sources:?}");
        }
    }
}

#[test]
fn csp_report_sample_and_trusted_types_eval_preserve_source_tokens() {
    for (prefix, suffix, valid) in [
        ("", "", true),
        ("\t", "\r\u{000c}", true),
        ("\u{000b}", "", false),
        ("", "\u{000b}", false),
    ] {
        let policy = format!("default-src 'none'; script-src {prefix}'report-sample'{suffix}");
        for disposition in [
            ContentSecurityPolicyDisposition::Enforce,
            ContentSecurityPolicyDisposition::Report,
        ] {
            let violation = content_security_policy_non_url_violation_with_source(
                &ContentSecurityPolicy::from(policy.as_str()),
                &protected_url(),
                ContentSecurityPolicyNonUrlKind::Eval,
                Some("blocked()"),
                disposition,
                &ContentSecurityPolicyReportingEndpoints::default(),
            )
            .expect("report-sample does not authorize evaluation");
            assert_eq!(violation.original_policy, policy);
            assert_eq!(violation.disposition, disposition);
            assert_eq!(
                violation.sample,
                if valid { "blocked()" } else { "" },
                "{policy:?}"
            );
        }
        let policy = format!(
            "require-trusted-types-for 'script'; script-src {prefix}'trusted-types-eval'{suffix}"
        );
        assert_eq!(
            content_security_policy_allows_trusted_types_eval(std::slice::from_ref(
                &ContentSecurityPolicy::from(policy.as_str())
            )),
            valid,
            "{policy:?}"
        );
    }
}

#[test]
fn frame_ancestors_requires_every_ancestor_to_match() {
    let protected_url = "https://child.test/frame.html";
    let ancestors = [Some("https://child.test"), Some("https://top.test")];

    let violation =
        frame_ancestors_violation(&["frame-ancestors 'self'"], protected_url, &ancestors)
            .expect("a cross-origin top ancestor must violate 'self'");
    assert_eq!(violation.effective_directive, "frame-ancestors");
    assert_eq!(violation.blocked_uri, protected_url);

    assert!(frame_ancestors_violation(&["frame-ancestors *"], protected_url, &ancestors).is_none());
    assert!(
        frame_ancestors_violation(
            &["frame-ancestors https://child.test https://top.test"],
            protected_url,
            &ancestors,
        )
        .is_none()
    );
    assert!(
        frame_ancestors_violation(
            &["frame-ancestors https://child.test"],
            protected_url,
            &ancestors,
        )
        .is_some()
    );
}

#[test]
fn frame_ancestors_has_no_fallback_and_allows_top_level_documents() {
    assert!(
        frame_ancestors_violation(
            &["default-src 'none'"],
            "https://child.test/frame.html",
            &[Some("https://top.test")],
        )
        .is_none()
    );
    assert!(
        frame_ancestors_violation(
            &["frame-ancestors 'none'"],
            "https://child.test/frame.html",
            &[],
        )
        .is_none()
    );
}

#[test]
fn every_frame_ancestors_policy_must_allow_embedding() {
    let violation = frame_ancestors_violation(
        &["frame-ancestors *", "frame-ancestors 'none'"],
        "https://child.test/frame.html",
        &[Some("https://top.test")],
    )
    .expect("one blocking policy must block the response");
    assert_eq!(violation.original_policy, "frame-ancestors 'none'");
}

#[test]
fn frame_ancestors_matches_origins_and_rejects_opaque_ancestors() {
    assert!(
        frame_ancestors_violation(
            &["frame-ancestors https://top.test/path"],
            "https://child.test/frame.html",
            &[Some("https://top.test")],
        )
        .is_some(),
        "a host source containing a path must not match an ancestor origin"
    );
    assert!(
        frame_ancestors_violation(
            &["frame-ancestors *"],
            "https://child.test/frame.html",
            &[None],
        )
        .is_some(),
        "an opaque ancestor origin must not match a source expression"
    );
}

#[test]
fn worker_src_none_blocks_worker_constructor() {
    assert!(!allowed(
        "worker-src 'none'; script-src 'self'",
        ContentSecurityPolicyResourceKind::WorkerConstructor,
        "https://app.test/worker.js"
    ));
}

#[test]
fn document_font_requests_use_font_src_and_default_src() {
    let kind = ContentSecurityPolicyResourceKind::DocumentFont;
    assert!(!allowed(
        "font-src 'none'; connect-src *",
        kind,
        "https://app.test/font.woff2"
    ));
    assert!(allowed(
        "font-src 'self'; default-src 'none'; connect-src 'none'",
        kind,
        "https://app.test/font.woff2"
    ));
    assert!(!allowed(
        "font-src 'self'; default-src *",
        kind,
        "https://cdn.test/font.woff2"
    ));
    assert!(!allowed(
        "default-src 'none'; connect-src *",
        kind,
        "https://app.test/font.woff2"
    ));
    assert!(allowed(
        "default-src 'self'; connect-src 'none'",
        kind,
        "https://app.test/font.woff2"
    ));
}

#[test]
fn worker_constructor_uses_script_src_and_default_src_fallbacks() {
    assert!(!allowed(
        "script-src 'none'",
        ContentSecurityPolicyResourceKind::WorkerConstructor,
        "https://app.test/worker.js"
    ));
    assert!(!allowed(
        "default-src 'none'",
        ContentSecurityPolicyResourceKind::WorkerConstructor,
        "https://app.test/worker.js"
    ));
    assert!(allowed(
        "default-src 'self'",
        ContentSecurityPolicyResourceKind::WorkerConstructor,
        "https://app.test/worker.js"
    ));
}

#[test]
fn worker_constructor_uses_child_src_before_script_src_fallback() {
    assert!(!allowed(
        "child-src 'none'; script-src 'self'",
        ContentSecurityPolicyResourceKind::WorkerConstructor,
        "https://app.test/worker.js"
    ));
    assert!(allowed(
        "child-src https://workers.test; script-src 'none'",
        ContentSecurityPolicyResourceKind::WorkerConstructor,
        "https://workers.test/worker.js"
    ));
}

#[test]
fn document_script_element_uses_script_src_elem_fallbacks() {
    assert!(!allowed(
        "script-src-elem 'none'; script-src 'self'",
        ContentSecurityPolicyResourceKind::DocumentScriptElement,
        "https://app.test/app.js"
    ));
    assert!(allowed(
        "script-src 'self'; default-src 'none'",
        ContentSecurityPolicyResourceKind::DocumentScriptElement,
        "https://app.test/app.js"
    ));
    assert!(!allowed(
        "default-src 'none'",
        ContentSecurityPolicyResourceKind::DocumentScriptElement,
        "https://app.test/app.js"
    ));
}

#[test]
fn worker_src_takes_precedence_for_worker_constructors() {
    assert!(allowed(
        "default-src 'none'; script-src 'none'; worker-src 'self'",
        ContentSecurityPolicyResourceKind::WorkerConstructor,
        "https://app.test/worker.js"
    ));
    assert!(!allowed(
        "default-src *; script-src *; worker-src 'none'",
        ContentSecurityPolicyResourceKind::WorkerConstructor,
        "https://app.test/worker.js"
    ));
}

#[test]
fn url_violation_reports_effective_directive_and_original_policy() {
    let violation = content_security_policy_url_violation_with_redirect_status(
        &[
            "connect-src https://api.test".into(),
            "default-src 'none'".into(),
        ],
        &protected_url(),
        &request_url("https://blocked.test/data.json"),
        ContentSecurityPolicyResourceKind::WorkerConnect,
        ContentSecurityPolicyRedirectStatus::NoRedirect,
    )
    .expect("blocked worker connect should produce violation");

    assert_eq!(violation.effective_directive, "connect-src");
    assert_eq!(violation.blocked_uri, "https://blocked.test/data.json");
    assert_eq!(violation.document_uri, "https://app.test/page.html");
    assert_eq!(violation.original_policy, "connect-src https://api.test");
    assert_eq!(
        violation.disposition,
        ContentSecurityPolicyDisposition::Enforce
    );
}

#[test]
fn url_violation_strips_credentials_and_fragments_from_report_urls() {
    let protected_url =
        request_url("https://user:secret@app.test/page.html?document=1#current-section");
    let blocked_url =
        request_url("https://other:secret@blocked.test/image.png?asset=1#pixel-fragment");
    let violation = content_security_policy_url_violation_with_redirect_status(
        &["img-src 'none'".into()],
        &protected_url,
        &blocked_url,
        ContentSecurityPolicyResourceKind::DocumentImage,
        ContentSecurityPolicyRedirectStatus::NoRedirect,
    )
    .expect("blocked image should produce violation");

    assert_eq!(
        violation.document_uri,
        "https://app.test/page.html?document=1"
    );
    assert_eq!(
        violation.blocked_uri,
        "https://blocked.test/image.png?asset=1"
    );
    assert_eq!(
        violation.source_file,
        "https://app.test/page.html?document=1"
    );
}

#[test]
fn url_violation_redacts_data_and_blob_urls_to_their_schemes() {
    for (policy, url, expected) in [
        ("media-src 'self'", "data:video/mp4;base64,AAAA", "data"),
        (
            "media-src https://example.com",
            "blob:https://app.test/id",
            "blob",
        ),
    ] {
        let violation = content_security_policy_url_violation_with_redirect_status(
            &[policy.to_owned().into()],
            &protected_url(),
            &request_url(url),
            ContentSecurityPolicyResourceKind::DocumentMedia,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
        )
        .expect("blocked media URL should produce violation");

        assert_eq!(violation.effective_directive, "media-src");
        assert_eq!(violation.blocked_uri, expected);
    }
}

#[test]
fn redirected_url_violation_reports_original_blocked_uri() {
    let original_url = request_url("https://app.test/redirect");
    let final_url = request_url("https://blocked.test/data.json");
    let violation = content_security_policy_url_violation_for_checked_url_with_redirect_status_disposition_and_reporting_endpoints(
        &["connect-src 'self'".into()],
        &protected_url(),
        &final_url,
        &original_url,
        ContentSecurityPolicyResourceKind::WorkerConnect,
        ContentSecurityPolicyRedirectStatus::FollowedRedirect,
        ContentSecurityPolicyDisposition::Enforce,
        &ContentSecurityPolicyReportingEndpoints::default(),
    )
    .expect("redirected blocked worker connect should produce violation");

    assert_eq!(violation.effective_directive, "connect-src");
    assert_eq!(violation.blocked_uri, "https://app.test/redirect");
}

#[test]
fn self_source_matches_same_origin_only() {
    assert!(allowed(
        "script-src 'self'",
        ContentSecurityPolicyResourceKind::WorkerScript,
        "https://app.test/worker-import.js"
    ));
    assert!(!allowed(
        "script-src 'self'",
        ContentSecurityPolicyResourceKind::WorkerScript,
        "https://cdn.test/worker-import.js"
    ));
}

#[test]
fn worker_module_imports_use_their_request_specific_directive_fallbacks() {
    let cross_origin = "https://cdn.test/worker-import.js";

    assert!(!allowed(
        "worker-src 'self'; script-src *",
        ContentSecurityPolicyResourceKind::WorkerStaticModuleImport,
        cross_origin,
    ));
    assert!(allowed(
        "worker-src *; script-src 'self'",
        ContentSecurityPolicyResourceKind::WorkerStaticModuleImport,
        cross_origin,
    ));
    assert!(!allowed(
        "script-src-elem 'self'; script-src *",
        ContentSecurityPolicyResourceKind::WorkerDynamicModuleImport,
        cross_origin,
    ));
    assert!(allowed(
        "worker-src 'self'; script-src *",
        ContentSecurityPolicyResourceKind::WorkerDynamicModuleImport,
        cross_origin,
    ));

    let violation = content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints(
        &["script-src 'self'".into()],
        &protected_url(),
        &request_url(cross_origin),
        ContentSecurityPolicyResourceKind::WorkerDynamicModuleImport,
        ContentSecurityPolicyRedirectStatus::NoRedirect,
        ContentSecurityPolicyDisposition::Enforce,
        &ContentSecurityPolicyReportingEndpoints::default(),
    )
    .expect("script-src fallback should block the cross-origin dynamic import");
    assert_eq!(violation.effective_directive, "script-src-elem");
}

#[test]
fn self_source_uses_csp_secure_upgrade_rules() {
    let protected = Url::parse("http://app.test/page.html").unwrap();
    assert!(content_security_policy_allows_url(
        &["connect-src 'self'".into()],
        &protected,
        &request_url("https://app.test/api"),
        ContentSecurityPolicyResourceKind::WorkerConnect,
    ));
    assert!(content_security_policy_allows_url(
        &["connect-src 'self'".into()],
        &protected,
        &request_url("ws://app.test/socket"),
        ContentSecurityPolicyResourceKind::WorkerConnect,
    ));
    assert!(content_security_policy_allows_url(
        &["connect-src 'self'".into()],
        &protected,
        &request_url("wss://app.test/socket"),
        ContentSecurityPolicyResourceKind::WorkerConnect,
    ));

    let protected = Url::parse("https://app.test/page.html").unwrap();
    assert!(content_security_policy_allows_url(
        &["connect-src 'self'".into()],
        &protected,
        &request_url("wss://app.test/socket"),
        ContentSecurityPolicyResourceKind::WorkerConnect,
    ));
    assert!(!content_security_policy_allows_url(
        &["connect-src 'self'".into()],
        &protected,
        &request_url("ws://app.test/socket"),
        ContentSecurityPolicyResourceKind::WorkerConnect,
    ));
    assert!(!content_security_policy_allows_url(
        &["connect-src 'self'".into()],
        &Url::parse("http://app.test/page.html").unwrap(),
        &request_url("ftp://app.test/resource"),
        ContentSecurityPolicyResourceKind::WorkerConnect,
    ));
}

#[test]
fn connect_src_and_default_src_control_worker_connect_requests() {
    assert!(!allowed(
        "connect-src 'none'",
        ContentSecurityPolicyResourceKind::WorkerConnect,
        "https://app.test/api"
    ));
    assert!(!allowed(
        "default-src 'none'",
        ContentSecurityPolicyResourceKind::WorkerConnect,
        "https://app.test/api"
    ));
    assert!(allowed(
        "connect-src 'self'; default-src 'none'",
        ContentSecurityPolicyResourceKind::WorkerConnect,
        "https://app.test/api"
    ));
}

#[test]
fn wildcard_source_does_not_match_local_schemes() {
    assert!(allowed(
        "script-src *",
        ContentSecurityPolicyResourceKind::WorkerScript,
        "https://app.test/worker-import.js"
    ));
    assert!(!allowed(
        "script-src *",
        ContentSecurityPolicyResourceKind::WorkerScript,
        "data:text/javascript,onconnect=()=>{}"
    ));
}

#[test]
fn scheme_sources_use_csp_secure_upgrade_rules() {
    assert!(allowed(
        "connect-src http:",
        ContentSecurityPolicyResourceKind::WorkerConnect,
        "https://cdn.test/api"
    ));
    assert!(allowed(
        "connect-src ws:",
        ContentSecurityPolicyResourceKind::WorkerConnect,
        "wss://cdn.test/socket"
    ));
    assert!(!allowed(
        "connect-src https:",
        ContentSecurityPolicyResourceKind::WorkerConnect,
        "wss://cdn.test/socket"
    ));
    assert!(!allowed(
        "connect-src http:",
        ContentSecurityPolicyResourceKind::WorkerConnect,
        "ws://cdn.test/socket"
    ));
}

#[test]
fn websocket_source_schemes_match_http_without_allowing_reverse_transitions() {
    for (source_scheme, matches) in [
        ("http", [true, true, false, false]),
        ("https", [false, true, false, false]),
        ("ws", [true, true, true, true]),
        ("wss", [false, true, false, true]),
    ] {
        for (request_scheme, expected) in ["http", "https", "ws", "wss"].into_iter().zip(matches) {
            for kind in [
                ContentSecurityPolicyResourceKind::DocumentConnect,
                ContentSecurityPolicyResourceKind::WorkerConnect,
            ] {
                for scheme in [source_scheme.to_owned(), source_scheme.to_ascii_uppercase()] {
                    let request = format!("{request_scheme}://cdn.test:8443/socket");
                    for source in [
                        format!("{scheme}:"),
                        format!("{scheme}://cdn.test:*/socket"),
                        format!("{scheme}://*.test:*/socket"),
                    ] {
                        assert_eq!(
                            allowed(&format!("connect-src {source}"), kind, &request),
                            expected,
                            "{source} with {request} for {kind:?}",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn websocket_to_http_host_sources_preserve_port_host_and_path_checks() {
    for authority in ["cdn.test", "*.test"] {
        for (source_scheme, source_port, request, expected) in [
            ("ws", "", "http://cdn.test/socket", true),
            ("ws", ":80", "http://cdn.test/socket", true),
            ("ws", ":8080", "http://cdn.test:8080/socket", true),
            ("ws", ":8080", "http://cdn.test:8081/socket", false),
            ("ws", "", "https://cdn.test/socket", true),
            ("wss", "", "https://cdn.test/socket", true),
            ("wss", ":443", "https://cdn.test/socket", true),
            ("wss", ":8443", "https://cdn.test:8443/socket", true),
            ("wss", ":8443", "https://cdn.test:9443/socket", false),
            ("ws", ":*", "https://other.example:8443/socket", false),
            ("ws", ":*", "https://cdn.test:8443/other", false),
            ("wss", ":*", "http://cdn.test:8443/socket", false),
        ] {
            let policy = format!("connect-src {source_scheme}://{authority}{source_port}/socket");
            for kind in [
                ContentSecurityPolicyResourceKind::DocumentConnect,
                ContentSecurityPolicyResourceKind::WorkerConnect,
            ] {
                assert_eq!(
                    allowed(&policy, kind, request),
                    expected,
                    "{policy} with {request}"
                );
            }
        }
    }
}

#[test]
fn websocket_scheme_compatibility_does_not_make_self_origins_equal() {
    for (protected, request, expected) in [
        ("ws://app.test/", "ws://app.test/socket", true),
        ("ws://app.test/", "http://app.test/socket", false),
        ("ws://app.test/", "https://app.test/socket", true),
        ("wss://app.test/", "https://app.test/socket", true),
        ("wss://app.test/", "http://app.test/socket", false),
    ] {
        assert_eq!(
            content_security_policy_allows_url(
                &["connect-src 'self'".into()],
                &Url::parse(protected).unwrap(),
                &request_url(request),
                ContentSecurityPolicyResourceKind::DocumentConnect,
            ),
            expected,
            "'self' in {protected} with {request}",
        );
    }
}

#[test]
fn source_path_matching_uses_encoded_paths() {
    assert!(allowed(
        "script-src https://app.test/trusted/",
        ContentSecurityPolicyResourceKind::WorkerScript,
        "https://app.test/trusted/worker.js"
    ));
    assert!(!allowed(
        "script-src https://app.test/trusted/",
        ContentSecurityPolicyResourceKind::WorkerScript,
        "https://app.test/trusted%2Fevil.js"
    ));
}

#[test]
fn host_sources_reject_scheme_relative_urls() {
    for source in ["//cdn.test", "//cdn.test:*", "//*.test", "//*.test:*"] {
        for scheme in ["https", "http"] {
            assert!(!allowed(
                &format!("script-src {source}"),
                ContentSecurityPolicyResourceKind::DocumentScriptElement,
                &format!("{scheme}://cdn.test/app.js")
            ));
        }
    }
}

#[test]
fn invalid_host_sources_cannot_allow_initial_or_redirected_requests() {
    for (source, request) in [
        (
            "https://cdn.test:*/foo,bar",
            "https://cdn.test:8443/foo,bar",
        ),
        ("https://*.test/foo,bar", "https://cdn.test/foo,bar"),
        ("https://cdn.test/foo,bar", "https://cdn.test/foo,bar"),
        ("https://user@cdn.test", "https://cdn.test/asset"),
        ("https://%63dn.test", "https://cdn.test/asset"),
        ("https://bad_host.test", "https://bad_host.test/asset"),
        (
            "https://*.bad_host.test:*",
            "https://cdn.bad_host.test:8443/asset",
        ),
        ("https://bad..test", "https://bad..test/asset"),
        ("https://*.test..:*", "https://cdn.test:8443/asset"),
        ("https://cdn.test:", "https://cdn.test/asset"),
        ("https://cdn.test:+443", "https://cdn.test/asset"),
        ("https://cdn.test:65536", "https://cdn.test/asset"),
        ("https://cdn.test/%", "https://cdn.test/%"),
        ("https://cdn.test:*/%6g", "https://cdn.test:8443/%6g"),
        ("https://*.test/%a", "https://cdn.test/%a"),
        ("https://cdn.test//asset", "https://cdn.test//asset"),
        (
            "https://cdn.test/asset\\file",
            "https://cdn.test/asset/file",
        ),
        ("https://cdn.test/[asset]", "https://cdn.test/[asset]"),
        ("https://cdn.test/asset\u{000b}", "https://cdn.test/asset"),
        ("\u{000b}https://cdn.test", "https://cdn.test/asset"),
        ("https://127.1", "https://127.0.0.1/asset"),
    ] {
        for redirect_status in [
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            ContentSecurityPolicyRedirectStatus::FollowedRedirect,
        ] {
            assert!(
                !source_expression_matches(
                    source,
                    &protected_url(),
                    &request_url(request),
                    redirect_status,
                ),
                "{source:?} must not match {request:?} ({redirect_status:?})"
            );
            assert!(
                !content_security_policy_allows_url_with_redirect_status(
                    &[format!("default-src 'none'; img-src {source}").into()],
                    &protected_url(),
                    &request_url(request),
                    ContentSecurityPolicyResourceKind::DocumentImage,
                    redirect_status,
                ),
                "{source:?} must not allow {request:?} ({redirect_status:?})"
            );
        }
    }
}

#[test]
fn host_source_paths_decode_segments_for_exact_and_wildcard_sources() {
    for authority in ["cdn.test", "cdn.test:*", "*.test", "*.test:*", "*", "*:*"] {
        for (source_path, request_path, expected) in [
            ("/%61sset", "/asset", true),
            ("/asset", "/%61sset", true),
            ("/%61sset/", "/asset/file", true),
            ("/%61sset/", "/asset", false),
            ("/asset", "/ASSET", false),
            ("/asset", "/asset/", false),
            ("/a%2fb", "/a%2Fb", true),
            ("/a%2fb", "/a/b", false),
            ("/a/", "/a%2Fb", false),
            ("/a/", "/a//b", true),
            ("/a//", "/a/b", false),
            ("/a%252fb", "/a%2fb", false),
            ("/a+b", "/a%20b", false),
            ("/a%2bb", "/a+b", true),
            ("/a%2cb", "/a,b", true),
            ("/a%3bb", "/a;b", true),
            ("/%ff", "/%FF", true),
            ("/%ff", "/%fe", false),
            ("/%e4%b8%ad", "/中", true),
            ("/a/../asset", "/asset", false),
            ("/a/%2e%2e/asset", "/asset", false),
            ("/./asset", "/asset", false),
            ("/path://asset", "/path://asset", true),
        ] {
            let policy = format!("img-src https://{authority}{source_path}");
            let request = format!("https://cdn.test{request_path}");
            assert_eq!(
                allowed(
                    &policy,
                    ContentSecurityPolicyResourceKind::DocumentImage,
                    &request
                ),
                expected,
                "{policy} with {request}"
            );
        }
    }
}

#[test]
fn valid_host_source_grammar_preserves_ports_and_raw_hostnames() {
    for (source, request, expected) in [
        ("HTTPS://CDN.TEST:00443", "https://cdn.test/asset", true),
        (
            "cdn.test/path://asset",
            "https://cdn.test/path://asset",
            true,
        ),
        ("https://cdn.test:0", "https://cdn.test:0/asset", true),
        (
            "https://cdn.test:65535",
            "https://cdn.test:65535/asset",
            true,
        ),
        (
            "https://xn--bcher-kva.test",
            "https://bücher.test/asset",
            true,
        ),
        ("https://*.TEST.", "https://cdn.test./asset", true),
        ("https://*.TEST.", "https://cdn.test/asset", false),
        ("https://*.test", "https://cdn.test./asset", false),
        ("ftp://cdn.test", "ftp://cdn.test/asset", true),
        ("https://192.0.2.1", "https://192.0.2.1/asset", true),
        (
            "https://192.0.2.1:8443",
            "https://192.0.2.1:8443/asset",
            true,
        ),
    ] {
        assert_eq!(
            allowed(
                &format!("img-src {source}"),
                ContentSecurityPolicyResourceKind::DocumentImage,
                request
            ),
            expected,
            "{source} with {request}"
        );
    }
}

#[test]
fn csp_path_matching_preserves_empty_segments_and_byte_boundaries() {
    for (source, request, expected) in [
        ("", "", true),
        ("", "/asset", true),
        ("/", "", true),
        ("/", "/asset", true),
        ("/a/", "/a", false),
        ("/a/", "/a/", true),
        ("/a//", "/a/", false),
        ("/a//", "/a//b", true),
        ("/a%2fb/", "/a/b/file", false),
        ("/a%2fb/", "/a%2Fb/file", true),
        ("/%00", "/%00", true),
        ("/%00", "/", false),
        ("/%ff", "/%fe", false),
    ] {
        assert_eq!(
            csp_path_part_matches(source, request),
            expected,
            "{source:?} with {request:?}"
        );
    }
}

#[test]
fn host_sources_reject_query_and_fragment_components() {
    assert!(!allowed(
        "script-src https://cdn.test?ignored",
        ContentSecurityPolicyResourceKind::DocumentScriptElement,
        "https://cdn.test/app.js"
    ));
    assert!(!allowed(
        "script-src https://cdn.test/app.js#fragment",
        ContentSecurityPolicyResourceKind::DocumentScriptElement,
        "https://cdn.test/app.js"
    ));
}

#[test]
fn redirected_source_url_matching_ignores_path_but_not_host() {
    assert!(content_security_policy_allows_url_with_redirect_status(
        &["script-src https://cdn.test/trusted/".into()],
        &protected_url(),
        &request_url("https://cdn.test/evil.js"),
        ContentSecurityPolicyResourceKind::WorkerScript,
        ContentSecurityPolicyRedirectStatus::FollowedRedirect,
    ));
    assert!(!content_security_policy_allows_url_with_redirect_status(
        &["script-src https://cdn.test/trusted/".into()],
        &protected_url(),
        &request_url("https://evil.test/evil.js"),
        ContentSecurityPolicyResourceKind::WorkerScript,
        ContentSecurityPolicyRedirectStatus::FollowedRedirect,
    ));
}

#[test]
fn wildcard_host_source_matches_subdomains_only() {
    assert!(allowed(
        "script-src *.cdn.example.com",
        ContentSecurityPolicyResourceKind::WorkerScript,
        "https://assets.cdn.example.com/worker-import.js"
    ));
    assert!(allowed(
        "script-src https://*.cdn.example.com",
        ContentSecurityPolicyResourceKind::WorkerScript,
        "https://assets.cdn.example.com/worker-import.js"
    ));
    assert!(!allowed(
        "script-src *.cdn.example.com",
        ContentSecurityPolicyResourceKind::WorkerScript,
        "https://cdn.example.com/worker-import.js"
    ));
    assert!(!allowed(
        "script-src *.cdn.example.com",
        ContentSecurityPolicyResourceKind::WorkerScript,
        "https://evilcdn.example.com/worker-import.js"
    ));
}

#[test]
fn wildcard_host_source_honors_port_and_path() {
    assert!(allowed(
        "script-src https://*.cdn.example.com:*",
        ContentSecurityPolicyResourceKind::WorkerScript,
        "https://assets.cdn.example.com:8443/worker-import.js"
    ));
    assert!(!allowed(
        "script-src https://*.cdn.example.com",
        ContentSecurityPolicyResourceKind::WorkerScript,
        "https://assets.cdn.example.com:8443/worker-import.js"
    ));
    assert!(allowed(
        "script-src https://*.cdn.example.com/trusted/",
        ContentSecurityPolicyResourceKind::WorkerScript,
        "https://assets.cdn.example.com/trusted/worker-import.js"
    ));
    assert!(!allowed(
        "script-src https://*.cdn.example.com/trusted/",
        ContentSecurityPolicyResourceKind::WorkerScript,
        "https://assets.cdn.example.com/trusted%2Fevil.js"
    ));
}

#[test]
fn exact_host_port_wildcards_apply_across_resource_kinds() {
    for kind in [
        ContentSecurityPolicyResourceKind::DocumentConnect,
        ContentSecurityPolicyResourceKind::DocumentFrame,
        ContentSecurityPolicyResourceKind::DocumentImage,
        ContentSecurityPolicyResourceKind::DocumentManifest,
        ContentSecurityPolicyResourceKind::DocumentMedia,
        ContentSecurityPolicyResourceKind::DocumentScriptElement,
        ContentSecurityPolicyResourceKind::DocumentStyleElement,
        ContentSecurityPolicyResourceKind::WorkerConnect,
        ContentSecurityPolicyResourceKind::WorkerScript,
        ContentSecurityPolicyResourceKind::WorkerConstructor,
        ContentSecurityPolicyResourceKind::WorkerStaticModuleImport,
        ContentSecurityPolicyResourceKind::WorkerDynamicModuleImport,
    ] {
        for source in ["https://cdn.test:*", "http://cdn.test:*", "CDN.TEST:*"] {
            for port in ["", ":0", ":80", ":443", ":8443", ":65535"] {
                let policy = format!("default-src {source}");
                let request = format!("https://cdn.test{port}/asset");
                assert!(
                    allowed(&policy, kind, &request),
                    "{policy} should allow {request} for {kind:?}"
                );
            }
        }
    }
}

#[test]
fn exact_host_port_wildcards_preserve_other_source_restrictions() {
    for (source, request, expected) in [
        ("http://cdn.test:*", "http://cdn.test:8080/asset", true),
        ("https://CDN.TEST:*", "https://cdn.test:8443/asset", true),
        ("cdn.test:*", "https://cdn.test:8443/asset", true),
        ("cdn.test:*", "http://cdn.test:8080/asset", false),
        (
            "https://cdn.test:*",
            "https://sub.cdn.test:8443/asset",
            false,
        ),
        (
            "https://cdn.test:*",
            "https://evilcdn.test:8443/asset",
            false,
        ),
        ("https://cdn.test.:*", "https://cdn.test:8443/asset", false),
        ("https://cdn.test.:*", "https://cdn.test.:8443/asset", true),
        ("https://cdn.test:*", "http://cdn.test:8080/asset", false),
        ("http://cdn.test:*", "ws://cdn.test:8080/asset", false),
        ("ws://cdn.test:*", "wss://cdn.test:8443/asset", true),
        ("wss://cdn.test:*", "ws://cdn.test:8080/asset", false),
        (
            "https://cdn.test:*/trusted/",
            "https://cdn.test:8443/trusted/asset",
            true,
        ),
        (
            "https://cdn.test:*/trusted/",
            "https://cdn.test:8443/trusted%2Fevil",
            false,
        ),
        (
            "https://cdn.test:*/trusted/",
            "https://cdn.test:8443/other/asset",
            false,
        ),
        (
            "https://cdn.test:*/asset",
            "https://cdn.test:8443/asset?cache=1",
            true,
        ),
        (
            "https://cdn.test:*/asset",
            "https://cdn.test:8443/asset/extra",
            false,
        ),
        (
            "https://cdn.test:*/asset",
            "https://cdn.test:8443/ASSET",
            false,
        ),
        ("//cdn.test:*", "https://cdn.test:8443/asset", false),
        (
            "https://user@cdn.test:*",
            "https://cdn.test:8443/asset",
            false,
        ),
        ("https://%63dn.test:*", "https://cdn.test:8443/asset", false),
        (
            "https://cdn.test:*/asset?ignored",
            "https://cdn.test:8443/asset",
            false,
        ),
        (
            "https://cdn.test:*/asset#ignored",
            "https://cdn.test:8443/asset",
            false,
        ),
        ("https://cdn.test:*0", "https://cdn.test:8443/asset", false),
        (
            "https://bad_host.test:*",
            "https://bad_host.test:8443/asset",
            false,
        ),
        (
            "https://bad..host.test:*",
            "https://bad..host.test:8443/asset",
            false,
        ),
        ("https://127.0.0.1:*", "https://127.0.0.1:8443/asset", true),
        ("https://192.0.2.1:*", "https://192.0.2.1:8443/asset", false),
        ("https://[::1]:*", "https://[::1]:8443/asset", false),
    ] {
        let policy = format!("img-src {source}");
        assert_eq!(
            allowed(
                &policy,
                ContentSecurityPolicyResourceKind::DocumentImage,
                request
            ),
            expected,
            "{policy} with {request}"
        );
    }
}

#[test]
fn exact_host_port_wildcards_preserve_redirect_restrictions() {
    for (request, expected) in [
        ("https://cdn.test:9443/other/asset", true),
        ("https://other.test:9443/other/asset", false),
        ("http://cdn.test:8080/other/asset", false),
    ] {
        assert_eq!(
            content_security_policy_allows_url_with_redirect_status(
                &["script-src https://cdn.test:*/trusted/".into()],
                &protected_url(),
                &request_url(request),
                ContentSecurityPolicyResourceKind::DocumentScriptElement,
                ContentSecurityPolicyRedirectStatus::FollowedRedirect,
            ),
            expected,
            "a redirected wildcard-port request must still match scheme and host: {request}"
        );
    }
}

#[test]
fn host_sources_use_csp_secure_upgrade_rules() {
    assert!(allowed(
        "connect-src http://api.example.com",
        ContentSecurityPolicyResourceKind::WorkerConnect,
        "https://api.example.com/socket"
    ));
    assert!(allowed(
        "connect-src ws://api.example.com",
        ContentSecurityPolicyResourceKind::WorkerConnect,
        "wss://api.example.com/socket"
    ));
    assert!(!allowed(
        "connect-src https://api.example.com",
        ContentSecurityPolicyResourceKind::WorkerConnect,
        "wss://api.example.com/socket"
    ));
    assert!(!allowed(
        "connect-src http://api.example.com",
        ContentSecurityPolicyResourceKind::WorkerConnect,
        "https://api.example.com:80/socket"
    ));
}

#[test]
fn wildcard_host_sources_use_csp_secure_upgrade_rules() {
    assert!(allowed(
        "connect-src ws://*.cdn.example.com",
        ContentSecurityPolicyResourceKind::WorkerConnect,
        "wss://assets.cdn.example.com/socket"
    ));
    assert!(!allowed(
        "connect-src https://*.cdn.example.com",
        ContentSecurityPolicyResourceKind::WorkerConnect,
        "wss://assets.cdn.example.com/socket"
    ));
}

#[test]
fn all_policies_are_enforced() {
    let result = ensure_content_security_policy_allows_url(
        &["script-src 'self'".into(), "script-src 'none'".into()],
        &protected_url(),
        &request_url("https://app.test/worker-import.js"),
        ContentSecurityPolicyResourceKind::WorkerScript,
        || "blocked".to_owned(),
    );

    assert!(result.is_err());
}

#[test]
fn script_element_nonce_source_allows_external_script_url() {
    let violation =
        content_security_policy_script_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
            &["script-src 'nonce-abc'".into()],
            &protected_url(),
            &request_url("https://cdn.test/script.js"),
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            ContentSecurityPolicyDisposition::Enforce,
            &ContentSecurityPolicyReportingEndpoints::default(),
            ContentSecurityPolicyScriptElementRequest::parser_inserted_with_nonce(Some("abc")),
        );

    assert!(
        violation.is_none(),
        "matching script nonce should allow the script element request"
    );
}

#[test]
fn style_element_request_uses_style_src_elem_fallbacks_and_nonce() {
    let protected_url = Url::parse("https://example.test/page").unwrap();
    let request_url = Url::parse("https://cdn.test/app.css").unwrap();
    let check = |policy: &str, nonce: Option<&str>| {
        content_security_policy_style_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
            &[policy.to_owned().into()],
            &protected_url,
            &request_url,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            ContentSecurityPolicyDisposition::Enforce,
            &ContentSecurityPolicyReportingEndpoints::default(),
            ContentSecurityPolicyStyleElementRequest { nonce },
        )
    };

    assert!(check("style-src https://cdn.test", None).is_none());
    assert!(check("default-src https://cdn.test", None).is_none());
    let violation = check("style-src-elem 'none'; style-src https://cdn.test", None)
        .expect("style-src-elem must take precedence over style-src");
    assert_eq!(violation.effective_directive, "style-src-elem");
    assert!(
        check(
            "style-src-elem 'nonce-allowed'; style-src 'none'",
            Some("allowed")
        )
        .is_none(),
        "a matching link nonce must authorize the external style request"
    );
    assert!(
        check(
            "style-src-elem 'nonce-allowed'; style-src https://cdn.test",
            Some("blocked")
        )
        .is_some(),
        "a non-matching nonce must not bypass the operative style-src-elem directive"
    );
}

#[test]
fn script_element_strict_dynamic_allows_non_parser_inserted_script() {
    let violation =
        content_security_policy_script_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
            &["script-src 'strict-dynamic' 'nonce-abc'".into()],
            &protected_url(),
            &request_url("https://cdn.test/script.js"),
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            ContentSecurityPolicyDisposition::Enforce,
            &ContentSecurityPolicyReportingEndpoints::default(),
            ContentSecurityPolicyScriptElementRequest {
                nonce: None,
                integrity: None,
                parser_inserted: false,
            },
        );

    assert!(
        violation.is_none(),
        "strict-dynamic should allow script elements inserted by a running trusted script"
    );
}

#[test]
fn script_element_strict_dynamic_blocks_parser_inserted_host_source_without_nonce() {
    let violation =
        content_security_policy_script_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
            &["script-src 'strict-dynamic' 'nonce-abc' https://cdn.test".into()],
            &protected_url(),
            &request_url("https://cdn.test/script.js"),
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            ContentSecurityPolicyDisposition::Enforce,
            &ContentSecurityPolicyReportingEndpoints::default(),
            ContentSecurityPolicyScriptElementRequest {
                nonce: None,
                integrity: None,
                parser_inserted: true,
            },
        )
        .expect("parser-inserted script without nonce should be blocked");

    assert_eq!(violation.effective_directive, "script-src-elem");
}

#[test]
fn script_element_integrity_hash_source_allows_external_script_url() {
    let violation =
        content_security_policy_script_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
            &["script-src 'ShA256-testdigest'".into()],
            &protected_url(),
            &request_url("https://cdn.test/script.js"),
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            ContentSecurityPolicyDisposition::Enforce,
            &ContentSecurityPolicyReportingEndpoints::default(),
            ContentSecurityPolicyScriptElementRequest {
                nonce: None,
                integrity: Some("sha256-testdigest"),
                parser_inserted: true,
            },
        );

    assert!(
        violation.is_none(),
        "matching integrity metadata should satisfy a CSP hash source regardless of algorithm case"
    );
}

#[test]
fn script_element_integrity_requires_every_supported_hash_in_the_source_list() {
    for directive in ["script-src-elem", "script-src", "default-src"] {
        let policy = format!("{directive} 'ShA256-testdigest' 'sha384-strong' 'sha512-other'");
        for (integrity, expected) in [
            ("sha256-testdigest sha384-strong", true),
            ("sha256-testdigest sha256-testdigest", true),
            ("sha512-other sha384-strong sha256-testdigest", true),
            ("sha256-testdigest sha256-unknown", false),
            ("sha256-unknown sha256-testdigest", false),
            ("sha256-testdigest sha512-unknown", false),
            ("sha256-unknown sha384-strong", false),
        ] {
            assert_eq!(
                script_element_request_allowed(
                    &policy,
                    ContentSecurityPolicyScriptElementRequest {
                        nonce: None,
                        integrity: Some(integrity),
                        parser_inserted: true,
                    },
                ),
                expected,
                "{policy}: {integrity}"
            );
        }
    }
}

#[test]
fn script_element_integrity_uses_nonempty_parsed_sri_metadata() {
    for (integrity, expected) in [
        (None, false),
        (Some(" \t\r\n\x0c"), false),
        (Some("sha1-ignored sha512-***"), false),
        (Some("sha256-***"), false),
        (Some("SHA256-testdigest"), false),
        (Some("sha256-testdigest\u{a0}"), false),
        (Some("sha256-testdigest sha1-ignored"), true),
        (Some("sha256-testdigest sha512-***"), true),
        (Some("sha256-testdigest sha512-*** sha512-abc==="), false),
        (Some("sha256-testdigest sha512-A=AAA"), false),
        (Some("sha256-testdigest sha512-===="), false),
        (Some("sha256-testdigest?ignored sha-256-testdigest"), true),
        (Some(" \tsha256-testdigest\r\n\x0csha256-testdigest "), true),
    ] {
        assert_eq!(
            script_element_request_allowed(
                "script-src 'sha256-testdigest' 'sha256-***'",
                ContentSecurityPolicyScriptElementRequest {
                    nonce: None,
                    integrity,
                    parser_inserted: true,
                },
            ),
            expected,
            "{integrity:?}"
        );
    }
}

#[test]
fn script_element_integrity_compares_encoded_digests_literally() {
    for (source, integrity, expected) in [
        ("'sha256-YQ=='", "sha256-YQ==", true),
        ("'sha256-YQ=='", "sha256-YQ", false),
        ("'sha256-+w=='", "sha256--w==", false),
        ("'sha256--w=='", "sha256-+w==", false),
        ("'sha256--w=='", "sha256--w==", true),
        ("'sha256-/w=='", "sha256-_w==", false),
        ("'sha256-_w=='", "sha256-/w==", false),
        ("'sha256-YR=='", "sha256-YR==", true),
    ] {
        assert_eq!(
            script_element_request_allowed(
                &format!("script-src {source}"),
                ContentSecurityPolicyScriptElementRequest {
                    nonce: None,
                    integrity: Some(integrity),
                    parser_inserted: true,
                },
            ),
            expected,
            "{source}: {integrity}"
        );
    }
}

#[test]
fn partial_integrity_match_preserves_other_script_authorizations() {
    for (other_sources, nonce, parser_inserted, expected) in [
        ("'nonce-allowed'", Some("allowed"), true, true),
        ("https://cdn.test", None, true, true),
        ("'strict-dynamic'", None, false, true),
        ("'strict-dynamic' https://cdn.test", None, true, false),
    ] {
        assert_eq!(
            script_element_request_allowed(
                &format!("script-src 'sha256-testdigest' {other_sources}"),
                ContentSecurityPolicyScriptElementRequest {
                    nonce,
                    integrity: Some("sha256-testdigest sha256-unknown"),
                    parser_inserted,
                },
            ),
            expected,
            "{other_sources}: nonce={nonce:?}, parser_inserted={parser_inserted}"
        );
    }
}

#[test]
fn script_src_elem_directive_controls_strict_dynamic_script_element_requests() {
    let allowed =
        content_security_policy_script_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
            &["script-src-elem 'strict-dynamic' 'nonce-abc'; script-src 'nonce-abc'".into()],
            &protected_url(),
            &request_url("https://cdn.test/script.js"),
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            ContentSecurityPolicyDisposition::Enforce,
            &ContentSecurityPolicyReportingEndpoints::default(),
            ContentSecurityPolicyScriptElementRequest {
                nonce: None,
                integrity: None,
                parser_inserted: false,
            },
        );
    assert!(
        allowed.is_none(),
        "script-src-elem strict-dynamic should control script element requests"
    );

    let violation =
        content_security_policy_script_element_url_violation_with_redirect_status_disposition_reporting_endpoints_and_request(
            &["script-src 'strict-dynamic' 'nonce-abc'; script-src-elem 'nonce-abc'".into()],
            &protected_url(),
            &request_url("https://cdn.test/script.js"),
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            ContentSecurityPolicyDisposition::Enforce,
            &ContentSecurityPolicyReportingEndpoints::default(),
            ContentSecurityPolicyScriptElementRequest {
                nonce: None,
                integrity: None,
                parser_inserted: false,
            },
        )
        .expect("script-src-elem without strict-dynamic should block this request");

    assert_eq!(violation.effective_directive, "script-src-elem");
}

#[test]
fn report_uri_endpoints_resolve_against_protected_url() {
    let endpoints = content_security_policy_report_uri_endpoints(
        &ContentSecurityPolicy::from(
            "connect-src 'none'; report-uri /csp-report https://reports.test/collect",
        ),
        &protected_url(),
    );

    assert_eq!(
        endpoints,
        vec![
            "https://app.test/csp-report".to_owned(),
            "https://reports.test/collect".to_owned()
        ]
    );
}

#[test]
fn report_to_suppresses_legacy_report_uri_transport() {
    let endpoints = content_security_policy_report_uri_endpoints(
        &ContentSecurityPolicy::from("connect-src 'none'; report-uri /legacy; report-to default"),
        &protected_url(),
    );

    assert!(endpoints.is_empty());
}

#[test]
fn reporting_endpoints_header_resolves_groups_against_protected_url() {
    let endpoints = content_security_policy_reporting_endpoints_from_headers(
        &[
            (
                "Reporting-Endpoints".to_owned(),
                b"default=\"/reports\", csp=\"https://reports.test/csp\"".to_vec(),
            ),
            (
                "reporting-endpoints".to_owned(),
                b"default=\"/override\", ignored=ftp://reports.test/nope".to_vec(),
            ),
        ],
        &protected_url(),
    );

    assert_eq!(
        endpoints.endpoint_for_group("default"),
        Some("https://app.test/override")
    );
    assert_eq!(
        endpoints.endpoint_for_group("csp"),
        Some("https://reports.test/csp")
    );
    assert_eq!(endpoints.endpoint_for_group("ignored"), None);
}

#[test]
fn report_to_uses_first_group_endpoint_and_suppresses_report_uri() {
    let reporting_endpoints = content_security_policy_reporting_endpoints_from_headers(
        &[(
            "Reporting-Endpoints".to_owned(),
            b"primary=\"/reports/primary\", secondary=\"/reports/secondary\"".to_vec(),
        )],
        &protected_url(),
    );
    let violation =
        content_security_policy_url_violation_with_redirect_status_disposition_and_reporting_endpoints(
            &[
                "connect-src 'none'; report-uri /legacy; report-to primary secondary"
                    .to_owned().into(),
            ],
            &protected_url(),
            &request_url("https://api.test/data.json"),
            ContentSecurityPolicyResourceKind::WorkerConnect,
            ContentSecurityPolicyRedirectStatus::NoRedirect,
            ContentSecurityPolicyDisposition::Enforce,
            &reporting_endpoints,
        )
        .expect("blocked URL should produce violation");

    assert!(violation.report_uri_endpoints.is_empty());
    assert_eq!(
        violation.report_to_endpoints,
        vec!["https://app.test/reports/primary".to_owned()]
    );
}

#[test]
fn violation_report_body_uses_csp_report_shape() {
    let violation = content_security_policy_url_violation_with_redirect_status(
        &["connect-src 'none'; report-uri /csp-report".into()],
        &protected_url(),
        &request_url("https://api.test/data.json"),
        ContentSecurityPolicyResourceKind::WorkerConnect,
        ContentSecurityPolicyRedirectStatus::NoRedirect,
    )
    .expect("blocked URL should produce violation");
    let body = content_security_policy_violation_report_body(
        &ContentSecurityPolicyViolationEventFields::from_url_violation(&violation),
    );
    let value: serde_json::Value = serde_json::from_str(&body).expect("report body JSON");

    assert_eq!(
        value,
        serde_json::json!({
            "csp-report": {
                "document-uri": "https://app.test/page.html",
                "referrer": "",
                "violated-directive": "connect-src",
                "effective-directive": "connect-src",
                "original-policy": "connect-src 'none'; report-uri /csp-report",
                "disposition": "enforce",
                "blocked-uri": "https://api.test/data.json",
                "source-file": "https://app.test/page.html",
                "status-code": 0,
                "script-sample": "",
            }
        })
    );
    assert_eq!(
        violation.report_uri_endpoints,
        vec!["https://app.test/csp-report".to_owned()]
    );
}

#[test]
fn reporting_api_report_body_uses_csp_violation_shape() {
    let violation = content_security_policy_url_violation_with_redirect_status(
        &["connect-src 'none'".into()],
        &protected_url(),
        &request_url("https://api.test/data.json"),
        ContentSecurityPolicyResourceKind::WorkerConnect,
        ContentSecurityPolicyRedirectStatus::NoRedirect,
    )
    .expect("blocked URL should produce violation");
    let body = content_security_policy_reporting_api_report_body(
        &ContentSecurityPolicyViolationEventFields::from_url_violation(&violation),
    );
    let value: serde_json::Value = serde_json::from_str(&body).expect("report body JSON");

    assert_eq!(value[0]["type"], "csp-violation");
    assert_eq!(value[0]["url"], "https://app.test/page.html");
    assert_eq!(
        value[0]["body"]["documentURL"],
        "https://app.test/page.html"
    );
    assert_eq!(value[0]["body"]["blockedURL"], "https://api.test/data.json");
    assert_eq!(value[0]["body"]["effectiveDirective"], "connect-src");
    assert_eq!(value[0]["body"]["originalPolicy"], "connect-src 'none'");
    assert_eq!(value[0]["body"]["disposition"], "enforce");
}

#[test]
fn non_url_inline_script_uses_script_src_elem_fallbacks() {
    let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
    assert!(
        content_security_policy_non_url_violation_with_source(
            &ContentSecurityPolicy::from("script-src-elem 'unsafe-inline'; script-src 'none'"),
            &protected_url(),
            ContentSecurityPolicyNonUrlKind::DocumentInlineScript,
            None,
            ContentSecurityPolicyDisposition::Enforce,
            &reporting_endpoints,
        )
        .is_none()
    );

    let violation = content_security_policy_non_url_violation_with_source(
        &ContentSecurityPolicy::from("script-src 'self'; default-src 'unsafe-inline'"),
        &protected_url(),
        ContentSecurityPolicyNonUrlKind::DocumentInlineScript,
        None,
        ContentSecurityPolicyDisposition::Report,
        &reporting_endpoints,
    )
    .expect("script-src should block inline script without unsafe-inline");
    assert_eq!(violation.effective_directive, "script-src-elem");
    assert_eq!(violation.blocked_uri, "inline");
    assert_eq!(
        violation.disposition,
        ContentSecurityPolicyDisposition::Report
    );
}

#[test]
fn inline_script_element_accepts_matching_nonce_or_source_hash() {
    let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
    let source = "globalThis.inlineScriptRan = true;";
    let matching_hash = "'sha256-BAF+pD4OL3OpwYDBVElncnexa2ZpNAjS3yn9BmiJEto='";
    let check = |policy: &str, nonce: Option<&str>| {
        content_security_policy_inline_script_element_violation_with_disposition_and_reporting_endpoints(
            &ContentSecurityPolicy::from(policy),
            &protected_url(),
            source,
            ContentSecurityPolicyScriptElementRequest {
                nonce,
                integrity: None,
                parser_inserted: true,
            },
            ContentSecurityPolicyDisposition::Enforce,
            &reporting_endpoints,
        )
    };

    assert!(check("script-src 'nonce-abc'", Some("abc")).is_none());
    assert!(check("script-src 'nonce-abc'", Some("wrong")).is_some());
    assert!(check(&format!("script-src {matching_hash}"), None).is_none());
    assert!(
        content_security_policy_inline_script_element_violation_with_disposition_and_reporting_endpoints(
            &format!("script-src {matching_hash}").into(),
            &protected_url(),
            "globalThis.inlineScriptRan = false;",
            ContentSecurityPolicyScriptElementRequest::default(),
            ContentSecurityPolicyDisposition::Enforce,
            &reporting_endpoints,
        )
        .is_some()
    );
}

#[test]
fn inline_script_element_unsafe_inline_obeys_nonce_and_strict_dynamic_precedence() {
    let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
    let check = |policy: &str, parser_inserted: bool| {
        content_security_policy_inline_script_element_violation_with_disposition_and_reporting_endpoints(
            &ContentSecurityPolicy::from(policy),
            &protected_url(),
            "globalThis.inlineScriptRan = true;",
            ContentSecurityPolicyScriptElementRequest {
                nonce: None,
                integrity: None,
                parser_inserted,
            },
            ContentSecurityPolicyDisposition::Enforce,
            &reporting_endpoints,
        )
    };

    assert!(check("script-src 'unsafe-inline'", true).is_none());
    assert!(check("script-src 'unsafe-inline' 'nonce-abc'", true).is_some());
    assert!(check("script-src 'unsafe-inline' 'strict-dynamic'", true).is_some());
    assert!(check("script-src 'unsafe-inline' 'strict-dynamic'", false).is_none());
}

#[test]
fn inline_style_element_uses_style_src_elem_and_accepts_nonce_or_hash() {
    let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
    let source = "p { color: green; }";
    let matching_hash = "'sha256-FSRZotz4y83Ib8ZaoVj9eXKaeWXVUawM8zAPfYeYySs='";
    let check = |policy: &str, nonce: Option<&str>| {
        content_security_policy_inline_style_element_violation_with_disposition_and_reporting_endpoints(
            &ContentSecurityPolicy::from(policy),
            &protected_url(),
            source,
            ContentSecurityPolicyStyleElementRequest { nonce },
            ContentSecurityPolicyDisposition::Enforce,
            &reporting_endpoints,
        )
    };

    assert!(check("style-src-elem 'nonce-abc'; style-src 'none'", Some("abc")).is_none());
    assert!(
        check(
            "style-src-elem 'nonce-abc'; style-src 'unsafe-inline'",
            None
        )
        .is_some()
    );
    assert!(check(&format!("style-src {matching_hash}"), None).is_none());
    assert!(check("default-src 'unsafe-inline'", None).is_none());
    assert!(check("style-src 'unsafe-inline' 'nonce-abc'", None).is_some());
}

#[test]
fn inline_hash_source_algorithm_names_are_ascii_case_insensitive() {
    let source = "p { color: green; }";
    let cases = [
        ("SHA256", "FSRZotz4y83Ib8ZaoVj9eXKaeWXVUawM8zAPfYeYySs="),
        (
            "sHa384",
            "XObbzlg3meDKX3QpZZz8hg0V39bxZI/QQ3qFg6emWOdBZinTtjOls/GimTwBU+WQ",
        ),
        (
            "Sha512",
            "yYhoHoYeH6+9nQONNTy2KAd3tps87iT2d7E58PcEnU5YmJh2am3blPT0gLXu3qnVxDvF/qQKibav9lIK3pCz1A==",
        ),
    ];

    for (algorithm, digest) in cases {
        let expression = format!("'{algorithm}-{digest}'");
        let hash_source = csp_hash_source_value(&expression)
            .expect("supported hash algorithm should parse case-insensitively");
        assert!(inline_source_matches_hash(source, hash_source));
    }
    assert!(csp_hash_source_value("'SHA1-digest'").is_none());
}

#[test]
fn inline_hashes_normalize_base64url_symbols_without_decoding_the_digest() {
    let source = "globalThis.__inlineHashRuns += 1;/*☃ 8*/";
    for (algorithm, encoded) in [
        ("sHa256", "1u5siURgPyHwZ-QAD8UUU8_PSFeCLQfQgff1wmWe30c="),
        (
            "sHa384",
            "dfgfWBTXEJmr8n1u3heMCmQspfZzXztxsa6b_uRqupCtR1-hV5y1NIRIuk1GgyfX",
        ),
        (
            "sHa512",
            "yuIija81LFQ7iw1A1DtvkYIsDvqIgpOAzeloywzVVvfCZdYs43Z-Dh0lhILbiEstq4O56dtDfie_iALMCVjEMA==",
        ),
    ] {
        let matches = |source: &str, encoded: &str| {
            let expression = format!("'{algorithm}-{encoded}'");
            inline_source_matches_hash(source, csp_hash_source_value(&expression).unwrap())
        };
        for encoded in [
            encoded.to_owned(),
            encoded.replace('-', "+"),
            encoded.replace('_', "/"),
            encoded.replace('-', "+").replace('_', "/"),
        ] {
            assert!(matches(source, &encoded), "{algorithm}: {encoded}");
            assert!(!matches(&format!("{source} "), &encoded));
            assert!(!matches(source, &encoded.to_ascii_uppercase()));
            assert!(!matches(source, &format!("{encoded}=")));
            if encoded.ends_with('=') {
                assert!(!matches(source, encoded.trim_end_matches('=')));
            }
        }
    }
    // Changing unused padding bits can decode to the same bytes, but CSP
    // compares encoded strings after replacing only '-' and '_'.
    assert!(!inline_source_matches_hash(
        source,
        csp_hash_source_value("'sha256-1u5siURgPyHwZ-QAD8UUU8_PSFeCLQfQgff1wmWe30d='",).unwrap()
    ));
}

#[test]
fn style_and_navigation_hash_checks_accept_base64url_and_require_unsafe_hashes() {
    let endpoints = ContentSecurityPolicyReportingEndpoints::default();
    let style_source = ".target { color: green; }/*☃ 0*/";
    let style_policy = "style-src 'sha256-GIrKCpP-iQOfwzOEC4HO_QF0a8OAlKUdqbDq5l8WXn0='";
    for (source, allowed) in [
        (style_source.to_owned(), true),
        (format!("{style_source} "), false),
    ] {
        let violation = content_security_policy_inline_style_element_violation_with_disposition_and_reporting_endpoints(
            &ContentSecurityPolicy::from(style_policy), &protected_url(), &source, ContentSecurityPolicyStyleElementRequest::default(),
            ContentSecurityPolicyDisposition::Enforce, &endpoints,
        );
        assert_eq!(violation.is_none(), allowed);
    }
    for (kind, source, encoded) in [
        (
            ContentSecurityPolicyNonUrlKind::DocumentInlineStyleAttribute,
            "color: green;/*☃ 8*/",
            "GjRTlK2I8F4B9NVwz22_fJvEUruslF4nw-V2jUp0LIM=",
        ),
        (
            ContentSecurityPolicyNonUrlKind::DocumentInlineNavigation,
            "javascript:/*☃ 0*/void(0)",
            "r92XonOGS4-_MCpEOXhFyBgS4-xKf3v6fZMeHTK873c=",
        ),
    ] {
        for unsafe_hashes in ["", "'unsafe-hashes'"] {
            let policy = format!("default-src {unsafe_hashes} 'sha256-{encoded}'");
            for (source, matching) in [(source.to_owned(), true), (format!("{source} "), false)] {
                let violation = content_security_policy_inline_source_violation_with_disposition_and_reporting_endpoints(
                    &ContentSecurityPolicy::from(policy.as_str()), &protected_url(), kind, &source,
                    ContentSecurityPolicyDisposition::Enforce, &endpoints,
                );
                assert_eq!(
                    violation.is_none(),
                    matching && !unsafe_hashes.is_empty(),
                    "{kind:?}: {policy}"
                );
            }
        }
    }
}

#[test]
fn inline_hash_base64url_normalization_does_not_normalize_nonces() {
    for (nonce, allowed) in [("-_/+", true), ("+//+", false), ("-_--", false)] {
        assert_eq!(
            script_element_request_allowed(
                "script-src 'nonce--_/+'",
                ContentSecurityPolicyScriptElementRequest {
                    nonce: Some(nonce),
                    ..Default::default()
                },
            ),
            allowed,
            "{nonce}"
        );
    }
}

#[test]
fn inline_style_attribute_uses_style_src_attr_and_requires_unsafe_hashes() {
    let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
    let source = "background: green";
    let matching_hash = "'sha256-S0VSqEOmzmyOifPfat2sJ7ELOgkldAEbaXlvi5iMqjc='";
    let check = |policy: &str| {
        content_security_policy_inline_source_violation_with_disposition_and_reporting_endpoints(
            &ContentSecurityPolicy::from(policy),
            &protected_url(),
            ContentSecurityPolicyNonUrlKind::DocumentInlineStyleAttribute,
            source,
            ContentSecurityPolicyDisposition::Enforce,
            &reporting_endpoints,
        )
    };

    assert!(check("style-src-attr 'unsafe-inline'; style-src 'none'").is_none());
    assert!(check("style-src-attr 'none'; style-src 'unsafe-inline'").is_some());
    assert!(check(&format!("style-src 'unsafe-hashes' {matching_hash}")).is_none());
    assert!(check(&format!("style-src {matching_hash}")).is_some());
    assert!(check("default-src 'unsafe-inline'").is_none());
}

#[test]
fn inline_style_violation_sample_requires_report_sample_and_is_clipped() {
    let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
    let source = "0123456789012345678901234567890123456789extra";
    let check_element = |policy: &str| {
        content_security_policy_inline_style_element_violation_with_disposition_and_reporting_endpoints(
            &ContentSecurityPolicy::from(policy),
            &protected_url(),
            source,
            ContentSecurityPolicyStyleElementRequest::default(),
            ContentSecurityPolicyDisposition::Enforce,
            &reporting_endpoints,
        )
        .expect("inline style should be blocked")
    };
    let check_attribute = |policy: &str| {
        content_security_policy_inline_source_violation_with_disposition_and_reporting_endpoints(
            &ContentSecurityPolicy::from(policy),
            &protected_url(),
            ContentSecurityPolicyNonUrlKind::DocumentInlineStyleAttribute,
            source,
            ContentSecurityPolicyDisposition::Enforce,
            &reporting_endpoints,
        )
        .expect("inline style attribute should be blocked")
    };

    assert_eq!(check_element("style-src 'none'").sample, "");
    assert_eq!(
        check_element("style-src 'none' 'report-sample'").sample,
        "0123456789012345678901234567890123456789"
    );
    assert_eq!(
        check_attribute("style-src-attr 'none' 'report-sample'").sample,
        "0123456789012345678901234567890123456789"
    );
}

#[test]
fn inline_event_handler_uses_script_src_attr_fallbacks() {
    let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
    let check = |policy: &str, source: &str| {
        content_security_policy_inline_source_violation_with_disposition_and_reporting_endpoints(
            &ContentSecurityPolicy::from(policy),
            &protected_url(),
            ContentSecurityPolicyNonUrlKind::DocumentInlineEventHandler,
            source,
            ContentSecurityPolicyDisposition::Enforce,
            &reporting_endpoints,
        )
    };

    assert!(
        check(
            "script-src-attr 'unsafe-inline'; script-src 'none'",
            "run() "
        )
        .is_none()
    );
    let violation = check(
        "script-src-attr 'none'; script-src 'unsafe-inline'",
        "run()",
    )
    .expect("script-src-attr should take precedence over script-src");
    assert_eq!(violation.effective_directive, "script-src-attr");
    assert_eq!(violation.blocked_uri, "inline");
    assert!(check("script-src 'unsafe-inline'; default-src 'none'", "run()").is_none());
    assert!(check("default-src 'unsafe-inline'", "run()").is_none());
}

#[test]
fn inline_event_handler_hash_requires_unsafe_hashes_and_an_exact_digest() {
    let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
    let source = "t1.done();";
    let matching_hash = "'sha256-wmuLCpoj8EMqfQlPnt5NIMgKkCK62CxAkAiewI0zZps='";
    let check = |policy: &str, source: &str| {
        content_security_policy_inline_source_violation_with_disposition_and_reporting_endpoints(
            &ContentSecurityPolicy::from(policy),
            &protected_url(),
            ContentSecurityPolicyNonUrlKind::DocumentInlineEventHandler,
            source,
            ContentSecurityPolicyDisposition::Enforce,
            &reporting_endpoints,
        )
    };

    assert!(
        check(
            &format!("script-src 'unsafe-hashes' {matching_hash}"),
            source,
        )
        .is_none()
    );
    assert!(check(&format!("script-src {matching_hash}"), source).is_some());
    assert!(
        check(
            &format!("script-src 'unsafe-hashes' {matching_hash}"),
            "t1.done()",
        )
        .is_some()
    );
}

#[test]
fn inline_navigation_hashes_the_complete_javascript_url() {
    let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
    let source = "javascript:opener.postMessage('pass', '*')";
    let matching_hash = "'sha256-IIiAJ8UuliU8o1qAv6CV4P3R8DeTf/v3MrsCwXW171Y='";
    let check = |policy: &str, source: &str| {
        content_security_policy_inline_source_violation_with_disposition_and_reporting_endpoints(
            &ContentSecurityPolicy::from(policy),
            &protected_url(),
            ContentSecurityPolicyNonUrlKind::DocumentInlineNavigation,
            source,
            ContentSecurityPolicyDisposition::Enforce,
            &reporting_endpoints,
        )
    };

    assert!(
        check(
            &format!("script-src 'unsafe-hashes' {matching_hash}"),
            source,
        )
        .is_none()
    );
    assert!(check(&format!("script-src {matching_hash}"), source).is_some());
    let violation = check(
        &format!("script-src-elem 'none'; script-src 'unsafe-hashes' {matching_hash}"),
        source,
    )
    .expect("script-src-elem should take precedence for inline navigation");
    assert_eq!(violation.effective_directive, "script-src-elem");
    assert_eq!(violation.blocked_uri, "inline");
}

#[test]
fn inline_event_handler_unsafe_inline_is_ignored_when_a_nonce_or_hash_is_present() {
    let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
    for policy in [
        "script-src 'unsafe-inline' 'nonce-abc'",
        "script-src 'unsafe-inline' 'sha256-wmuLCpoj8EMqfQlPnt5NIMgKkCK62CxAkAiewI0zZps='",
    ] {
        assert!(
            content_security_policy_inline_source_violation_with_disposition_and_reporting_endpoints(
                &ContentSecurityPolicy::from(policy),
                &protected_url(),
                ContentSecurityPolicyNonUrlKind::DocumentInlineEventHandler,
                "untrusted()",
                ContentSecurityPolicyDisposition::Enforce,
                &reporting_endpoints,
            )
            .is_some(),
            "{policy}",
        );
    }
}

#[test]
fn non_url_wasm_eval_uses_script_src_or_default_src() {
    let reporting_endpoints = ContentSecurityPolicyReportingEndpoints::default();
    for policy in [
        "script-src 'wasm-unsafe-eval'; default-src 'none'",
        "script-src 'unsafe-eval'",
        "default-src 'unsafe-eval'",
    ] {
        assert!(
            content_security_policy_non_url_violation_with_source(
                &ContentSecurityPolicy::from(policy),
                &protected_url(),
                ContentSecurityPolicyNonUrlKind::WasmEval,
                None,
                ContentSecurityPolicyDisposition::Enforce,
                &reporting_endpoints,
            )
            .is_none(),
            "{policy}"
        );
    }

    let violation = content_security_policy_non_url_violation_with_source(
        &ContentSecurityPolicy::from("default-src 'self'"),
        &protected_url(),
        ContentSecurityPolicyNonUrlKind::WasmEval,
        None,
        ContentSecurityPolicyDisposition::Enforce,
        &reporting_endpoints,
    )
    .expect("default-src should block wasm eval without unsafe eval source");
    assert_eq!(violation.effective_directive, "script-src");
    assert_eq!(violation.blocked_uri, "wasm-eval");
}

#[test]
fn trusted_types_directive_filters_policy_names() {
    let allowed = |policies: &[&str], name: &str, is_duplicate: bool| {
        policies
            .iter()
            .all(|policy| policy_allows_trusted_type_policy_name(&ContentSecurityPolicy::from(*policy), name, is_duplicate))
    };

    assert!(allowed(&[], "SomeName", false));
    assert!(allowed(&[], "SomeName", true));
    assert!(allowed(&["default-src 'none'"], "SomeName", false));
    assert!(allowed(
        &["trusted-types SomeName OtherName"],
        "SomeName",
        false
    ));
    assert!(allowed(
        &["trusted-types * 'aLLow-dUPLIcates'"],
        "SomeName",
        true
    ));
    assert!(allowed(
        &["trusted-types 'none' SomeName"],
        "SomeName",
        false
    ));
    assert!(!allowed(&["trusted-types"], "SomeName", false));
    assert!(!allowed(&["trusted-types 'nONe'"], "SomeName", false));
    assert!(!allowed(&["trusted-types SomeName"], "SomeName", true));
    assert!(!allowed(&["trusted-types SomeName"], "default", false));
    assert!(!allowed(
        &["trusted-types SomeName", "trusted-types OtherName"],
        "SomeName",
        false
    ));
}

#[test]
fn trusted_types_csp_name_grammar_matches_only_valid_tokens() {
    // Directive grammar errors and policy delimiters are tested separately: they discard the
    // directive before its tt-policy-name tokens are interpreted.
    for byte in [b'\t', b'\n', b'\x0c', b'\r']
        .into_iter()
        .chain(b' '..=b'~')
        .filter(|byte| !matches!(*byte, b',' | b';'))
    {
        let name = format!("policy{}name", char::from(byte));
        let policy = format!("trusted-types {name}");
        let expected = byte.is_ascii_alphanumeric() || b"-#=_/@.%".contains(&byte);
        assert_eq!(
            policy_allows_trusted_type_policy_name(
                &ContentSecurityPolicy::from(policy.as_str()),
                &name,
                false
            ),
            expected,
            "invalid tt-policy-name byte {byte:#04x} must not match literally",
        );
    }
    for name in ["none", "allow-duplicates", "A-z_09#=/@.%"] {
        assert!(policy_allows_trusted_type_policy_name(
            &format!("trusted-types {name}").into(),
            name,
            false
        ));
    }
    assert!(policy_allows_trusted_type_policy_name(
        &ContentSecurityPolicy::from("trusted-types valid policy*name"),
        "valid",
        false
    ));
    assert!(!policy_allows_trusted_type_policy_name(
        &ContentSecurityPolicy::from("trusted-types valid policy*name"),
        "policy*name",
        false
    ));
    for wildcard in ["policy*", "**"] {
        assert!(!policy_allows_trusted_type_policy_name(
            &format!("trusted-types {wildcard}").into(),
            "valid",
            false
        ));
    }
    assert!(!policy_allows_trusted_type_policy_name(
        &ContentSecurityPolicy::from("trusted-types valid x'allow-duplicates'"),
        "valid",
        true,
    ));
}

#[test]
fn trusted_types_csp_name_grammar_does_not_restrict_unlisted_or_wildcard_names() {
    for name in [
        "",
        "policy*name",
        "policy$name",
        "política",
        "ポリシー",
        "\0",
    ] {
        for policy in ["", "trusted-types *"] {
            assert!(policy_allows_trusted_type_policy_name(
                &ContentSecurityPolicy::from(policy),
                name,
                false
            ));
        }
        assert!(!policy_allows_trusted_type_policy_name(
            &ContentSecurityPolicy::from("trusted-types *"), name, true,
        ));
        assert!(policy_allows_trusted_type_policy_name(
            &ContentSecurityPolicy::from("trusted-types * 'allow-duplicates'"), name, true,
        ));
    }
}

#[test]
fn csp_directive_parser_discards_invalid_values_without_replacing_first_directives() {
    for directive in [
        "trusted-types allowed política",
        "trusted-types\u{a0}allowed",
        "\u{a0}trusted-types allowed",
        "trusted-types allowed\u{a0}",
        "require-trusted-types-for 'script' ポリシー",
        "script-src 'none' https://例子.test",
    ] {
        assert!(
            ContentSecurityPolicy::from(directive).is_empty(),
            "{directive}"
        );
    }

    let directives = ContentSecurityPolicy::from(
        "trusted-types invalid política; TRUSTED-TYPES allowed; trusted-types *; \
         script-src 'none'; img-src https://例子.test; img-src 'self'",
    );
    assert!(directive_source_list(&directives, TRUSTED_TYPES).is_none());
    assert_eq!(
        directive_source_list(&directives, SCRIPT_SRC),
        Some(vec!["'none'"]),
    );
    assert!(directive_source_list(&directives, IMG_SRC).is_none());
}

#[test]
fn csp_directive_parser_uses_only_ascii_whitespace_around_names() {
    for space in ['\t', '\n', '\u{000c}', '\r', ' '] {
        let policy = format!("{space}TrUsTeD-TyPeS{space}allowed{space}");
        let directives = ContentSecurityPolicy::from(policy.as_str());
        assert_eq!(
            directive_source_list(&directives, TRUSTED_TYPES),
            Some(vec!["allowed"]),
        );
    }
    for invalid in ['\0', '\u{000b}', '\u{001f}', '\u{007f}'] {
        for policy in [
            format!("{invalid}trusted-types allowed"),
            format!("trusted-types{invalid} allowed"),
        ] {
            assert!(
                directive_source_list(&ContentSecurityPolicy::from(policy.as_str()), TRUSTED_TYPES)
                    .is_none(),
                "{policy:?}",
            );
        }
    }
}

#[test]
fn trusted_types_eval_requires_enforcement_and_an_operative_keyword() {
    let allowed = |policies: &[&str]| {
        content_security_policy_allows_trusted_types_eval(
            &policies
                .iter()
                .map(|policy| (*policy).into())
                .collect::<Vec<_>>(),
        )
    };

    assert!(allowed(&[
        "require-trusted-types-for 'script'; script-src 'trusted-types-eval'"
    ]));
    assert!(allowed(&[
        "require-trusted-types-for 'script'",
        "default-src 'trusted-types-eval'"
    ]));
    assert!(!allowed(&["script-src 'trusted-types-eval'"]));
    assert!(!allowed(&[
        "require-trusted-types-for 'script'; default-src 'trusted-types-eval'; script-src 'self'"
    ]));
    assert!(!allowed(&[
        "require-trusted-types-for 'script'; script-src 'unsafe-eval'"
    ]));
}

#[test]
fn sandbox_directive_forces_opaque_origin_without_allow_same_origin() {
    let forces_opaque = |policies: &[&str]| {
        content_security_policy_forces_opaque_origin(
            &policies
                .iter()
                .map(|policy| (*policy).into())
                .collect::<Vec<_>>(),
        )
    };

    assert!(!forces_opaque(&[]));
    assert!(!forces_opaque(&["script-src 'self'"]));
    assert!(forces_opaque(&["sandbox"]));
    assert!(forces_opaque(&["sandbox allow-scripts"]));
    assert!(!forces_opaque(&["sandbox allow-scripts allow-same-origin"]));
    assert!(!forces_opaque(&["sandbox allow-scripts ALLOW-SAME-ORIGIN"]));
}

#[test]
fn sandbox_directive_sandboxes_document_domain_even_with_allow_same_origin() {
    let sandboxes_document_domain = |policies: &[&str]| {
        content_security_policy_sandboxes_document_domain(
            &policies
                .iter()
                .map(|policy| (*policy).into())
                .collect::<Vec<_>>(),
        )
    };

    assert!(!sandboxes_document_domain(&[]));
    assert!(!sandboxes_document_domain(&["script-src 'self'"]));
    assert!(sandboxes_document_domain(&["sandbox"]));
    assert!(sandboxes_document_domain(&["sandbox allow-scripts"]));
    assert!(sandboxes_document_domain(&[
        "sandbox allow-scripts allow-same-origin"
    ]));
    assert!(sandboxes_document_domain(&[
        "sandbox allow-scripts ALLOW-SAME-ORIGIN"
    ]));
}

#[test]
fn sandbox_directive_allows_scripts_only_when_all_active_sandboxes_allow_it() {
    let allows_scripts = |policies: &[&str]| {
        content_security_policy_sandbox_allows_scripts(
            &policies
                .iter()
                .map(|policy| (*policy).into())
                .collect::<Vec<_>>(),
        )
    };

    assert!(allows_scripts(&[]));
    assert!(allows_scripts(&["script-src 'self'"]));
    assert!(!allows_scripts(&["sandbox"]));
    assert!(allows_scripts(&["sandbox allow-scripts"]));
    assert!(allows_scripts(&["sandbox ALLOW-SCRIPTS"]));
    assert!(!allows_scripts(&[
        "sandbox allow-scripts",
        "sandbox allow-same-origin"
    ]));
}

#[test]
fn sandbox_directive_allows_modals_only_when_all_active_sandboxes_allow_it() {
    let allows_modals = |policies: &[&str]| {
        content_security_policy_sandbox_allows_modals(
            &policies
                .iter()
                .map(|policy| (*policy).into())
                .collect::<Vec<_>>(),
        )
    };

    assert!(allows_modals(&[]));
    assert!(allows_modals(&["script-src 'self'"]));
    assert!(!allows_modals(&["sandbox"]));
    assert!(allows_modals(&["sandbox allow-modals"]));
    assert!(allows_modals(&["sandbox ALLOW-MODALS"]));
    assert!(!allows_modals(&[
        "sandbox allow-modals",
        "sandbox allow-scripts"
    ]));
}

#[test]
fn sandbox_directive_allows_popups_to_escape_only_when_all_active_sandboxes_allow_it() {
    let allows_popups_to_escape = |policies: &[&str]| {
        content_security_policy_sandbox_allows_popups_to_escape(
            &policies
                .iter()
                .map(|policy| (*policy).into())
                .collect::<Vec<_>>(),
        )
    };

    assert!(!allows_popups_to_escape(&[]));
    assert!(!allows_popups_to_escape(&["script-src 'self'"]));
    assert!(!allows_popups_to_escape(&["sandbox"]));
    assert!(allows_popups_to_escape(&[
        "sandbox allow-popups-to-escape-sandbox"
    ]));
    assert!(allows_popups_to_escape(&[
        "sandbox ALLOW-POPUPS-TO-ESCAPE-SANDBOX"
    ]));
    assert!(!allows_popups_to_escape(&[
        "sandbox allow-popups-to-escape-sandbox",
        "sandbox allow-scripts"
    ]));
}


#[test]
fn base_uri_uses_its_own_source_list_and_reports_violations() {
    let protected_url = Url::parse("https://page.test/path/page.html").unwrap();
    for (policy, request, allowed) in [
        ("default-src 'none'", "https://other.test/base/", true),
        ("base-uri 'none'", "https://page.test/base/", false),
        ("base-uri 'self'", "https://page.test/base/", true),
        ("base-uri 'self'", "https://other.test/base/", false),
        (
            "base-uri https://cdn.test:0443/",
            "https://cdn.test/base/",
            true,
        ),
        (
            "base-uri https://cdn%2Etest/",
            "https://cdn.test/base/",
            false,
        ),
    ] {
        let violation =
            content_security_policy_url_violation_with_redirect_status_and_disposition(
                &[policy.into()],
                &protected_url,
                &Url::parse(request).unwrap(),
                ContentSecurityPolicyResourceKind::DocumentBase,
                ContentSecurityPolicyRedirectStatus::NoRedirect,
                ContentSecurityPolicyDisposition::Report,
            );
        assert_eq!(violation.is_none(), allowed, "{policy}: {request}");
        if let Some(violation) = violation {
            assert_eq!(violation.effective_directive, "base-uri");
            assert_eq!(violation.blocked_uri, request);
            assert_eq!(violation.original_policy, policy);
            assert_eq!(
                violation.disposition,
                ContentSecurityPolicyDisposition::Report
            );
        }
    }
}

#[test]
fn sandbox_top_navigation_permissions_intersect_with_activation_requirements() {
    for (policies, without_activation, with_activation) in [
        (vec![], true, true),
        (vec!["sandbox"], false, false),
        (vec!["sandbox allow-top-navigation"], true, true),
        (
            vec!["sandbox allow-top-navigation-by-user-activation"],
            false,
            true,
        ),
        (
            vec![
                "sandbox allow-top-navigation",
                "sandbox allow-top-navigation-by-user-activation",
            ],
            false,
            true,
        ),
        (
            vec!["sandbox allow-top-navigation", "sandbox allow-scripts"],
            false,
            false,
        ),
    ] {
        let policies = policies.into_iter().map(ContentSecurityPolicy::from).collect::<Vec<_>>();
        assert_eq!(
            content_security_policy_sandbox_allows_top_navigation(&policies, false),
            without_activation,
            "{policies:?}"
        );
        assert_eq!(
            content_security_policy_sandbox_allows_top_navigation(&policies, true),
            with_activation,
            "{policies:?}"
        );
    }
}
