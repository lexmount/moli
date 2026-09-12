use moli_fetch::RedirectInfo;
use url::Url;

pub(crate) fn ensure_worker_script_redirect_chain_same_origin(
    initiator_url: &Url,
    redirect_chain: &[RedirectInfo],
    final_url: &Url,
) -> Result<(), String> {
    if !matches!(initiator_url.scheme(), "http" | "https") {
        return Ok(());
    }
    for redirect in redirect_chain {
        if !moli_url::same_origin(initiator_url, &redirect.from_url) {
            return Err(format!(
                "cross-origin redirect from `{}` is not allowed.",
                redirect.from_url
            ));
        }
        if !moli_url::same_origin(initiator_url, &redirect.to_url) {
            return Err(format!(
                "cross-origin redirect to `{}` is not allowed.",
                redirect.to_url
            ));
        }
    }
    if !moli_url::same_origin(initiator_url, final_url) {
        return Err(format!(
            "cross-origin redirect to `{final_url}` is not allowed."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(raw: &str) -> Url {
        Url::parse(raw).unwrap()
    }

    fn redirect(from_url: &Url, to_url: &Url) -> RedirectInfo {
        RedirectInfo {
            source: moli_fetch::RedirectSource::Network,
            from_url: from_url.clone(),
            to_url: to_url.clone(),
            status: 302,
            headers: Vec::new(),
            network_extra_info_available: true,
            request_extra_info: None,
            response_extra_info: None,
            redirect_has_extra_info: true,
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        }
    }

    #[test]
    fn worker_script_redirect_chain_rejects_cross_origin_final_url() {
        let initiator = url("https://app.test/page.html");
        let same_origin = url("https://app.test/worker.js");
        let cross_origin = url("https://evil.test/worker.js");

        assert!(
            ensure_worker_script_redirect_chain_same_origin(&initiator, &[], &same_origin).is_ok()
        );
        assert!(
            ensure_worker_script_redirect_chain_same_origin(&initiator, &[], &cross_origin)
                .is_err()
        );
    }

    #[test]
    fn worker_script_redirect_chain_rejects_cross_origin_intermediate_hop() {
        let initiator = url("https://app.test/page.html");
        let same_origin_redirect = url("https://app.test/redirect");
        let cross_origin_redirect = url("https://evil.test/redirect");
        let same_origin_final = url("https://app.test/worker.js");
        let chain = vec![
            redirect(&same_origin_redirect, &cross_origin_redirect),
            redirect(&cross_origin_redirect, &same_origin_final),
        ];

        assert!(
            ensure_worker_script_redirect_chain_same_origin(&initiator, &chain, &same_origin_final)
                .is_err()
        );
    }

    #[test]
    fn worker_script_redirect_chain_accepts_same_origin_hops() {
        let initiator = url("https://app.test/page.html");
        let same_origin_redirect = url("https://app.test/redirect");
        let same_origin_middle = url("https://app.test/middle");
        let same_origin_final = url("https://app.test/worker.js");
        let chain = vec![
            redirect(&same_origin_redirect, &same_origin_middle),
            redirect(&same_origin_middle, &same_origin_final),
        ];

        assert!(
            ensure_worker_script_redirect_chain_same_origin(&initiator, &chain, &same_origin_final)
                .is_ok()
        );
    }

    #[test]
    fn worker_script_redirect_chain_skips_opaque_initiators() {
        let initiator = url("data:text/html,hello");
        let cross_origin = url("https://evil.test/worker.js");

        assert!(
            ensure_worker_script_redirect_chain_same_origin(&initiator, &[], &cross_origin).is_ok()
        );
    }
}
