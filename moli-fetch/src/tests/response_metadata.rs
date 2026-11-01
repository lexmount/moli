use super::*;
use crate::{FetchResponseType, RedirectInfo, RedirectSource, RequestMode, ResponseHead};

fn redirected_head(allow_redirect_timing: bool) -> ResponseHead {
    let home = Url::parse("https://home.test/asset").unwrap();
    let cross = Url::parse("https://cross.test/hop").unwrap();
    let redirect = |from_url, to_url, headers| RedirectInfo {
        source: RedirectSource::Network,
        from_url,
        to_url,
        status: 302,
        headers,
        network_extra_info_available: false,
        request_extra_info: None,
        dispatched_request: None,
        response_extra_info: None,
        redirect_has_extra_info: false,
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        from_cache: false,
        negotiated_http_version: None,
    };
    let tao = vec![("Timing-Allow-Origin".to_owned(), b"*".to_vec())];
    ResponseHead {
        request_include_credentials: false,

        final_url: home.clone(),
        status: 200,
        status_text: None,
        headers: tao.clone(),
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        redirected: true,
        redirect_chain: vec![
            redirect(home.clone(), cross.clone(), Vec::new()),
            redirect(
                cross,
                home,
                if allow_redirect_timing {
                    tao
                } else {
                    Vec::new()
                },
            ),
        ],
        from_cache: false,
        cache_state: Default::default(),
        preload_state: Default::default(),
        negotiated_http_version: None,
    }
}

#[test]
fn response_metadata_preserves_redirect_taint_and_sticky_timing_authorization() {
    let origin = moli_url::WebOrigin::from_url(&Url::parse("https://home.test/").unwrap());
    for mode in [RequestMode::Cors, RequestMode::NoCors] {
        for allowed in [false, true] {
            let head = redirected_head(allowed);
            let metadata = head.fetch_metadata(&origin, mode);
            assert_eq!(metadata.timing_allow_passed, allowed);
            let expected = if mode == RequestMode::Cors {
                FetchResponseType::Cors
            } else {
                FetchResponseType::Opaque
            };
            assert_eq!(metadata.response_type, expected);
            assert_eq!(head.fetch_response_type(&origin, mode), expected);
            assert_eq!(
                head.fetch_response_type_with_web_security(&origin, mode, false),
                expected
            );
            assert_eq!(
                metadata
                    .url_list
                    .iter()
                    .map(Url::as_str)
                    .collect::<Vec<_>>(),
                [
                    "https://home.test/asset",
                    "https://cross.test/hop",
                    "https://home.test/asset",
                ]
            );
            assert!(metadata.redirected);
        }
    }
}

#[test]
fn response_metadata_url_list_keeps_real_redirects_to_the_same_url() {
    let mut head = redirected_head(true);
    head.redirect_chain.truncate(1);
    head.redirect_chain[0].to_url = head.final_url.clone();
    assert_eq!(
        head.url_list().to_vec(),
        [head.final_url.clone(), head.final_url.clone()]
    );
}

#[test]
fn disabled_web_security_changes_network_readability_without_forging_identity_or_timing() {
    let origin = moli_url::WebOrigin::from_url(&Url::parse("https://home.test/").unwrap());
    for mode in [
        RequestMode::Cors,
        RequestMode::NoCors,
        RequestMode::SameOrigin,
    ] {
        let head = redirected_head(false);
        let strict = head.fetch_metadata(&origin, mode);
        let readable = head.fetch_metadata_with_web_security(&origin, mode, true);
        assert_eq!(readable.response_type, FetchResponseType::Basic);
        assert_eq!(
            head.fetch_response_type_with_web_security(&origin, mode, true),
            FetchResponseType::Basic
        );
        assert_eq!(readable.url_list, strict.url_list);
        assert_eq!(readable.redirected, strict.redirected);
        assert_eq!(readable.timing_allow_passed, strict.timing_allow_passed);
        let mut redirect = head;
        redirect.status = 302;
        redirect
            .headers
            .push(("Location".into(), b"/next".to_vec()));
        assert_eq!(
            redirect.fetch_response_type(&origin, mode),
            FetchResponseType::OpaqueRedirect
        );
        for disabled in [false, true] {
            assert_eq!(
                redirect.fetch_response_type_with_web_security(&origin, mode, disabled),
                FetchResponseType::OpaqueRedirect
            );
        }
        assert_eq!(
            redirect
                .fetch_metadata_with_web_security(&origin, mode, true)
                .response_type,
            FetchResponseType::OpaqueRedirect
        );
    }
}
