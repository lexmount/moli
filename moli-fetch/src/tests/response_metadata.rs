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
            assert_eq!(
                metadata.response_type,
                if mode == RequestMode::Cors {
                    FetchResponseType::Cors
                } else {
                    FetchResponseType::Opaque
                }
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
