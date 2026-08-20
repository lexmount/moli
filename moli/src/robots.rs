//! `robots.txt` enforcement for the CLI fetch command.
//!
//! `--obey-robots` gates the top-level navigation only. Subresources a page
//! pulls in are not checked, because a partially loaded document is a worse
//! answer than a clearly refused one, and the flag exists to decide whether to
//! visit a page at all.

use anyhow::{Context, Result, bail};
use moli_cookie_jar::new_shared_browser_cookie_store;
use moli_fetch::{FetchClient, FetchConfig, Request};
use moli_robots::{RobotsPolicy, robots_request_target, robots_txt_url};
use url::Url;

/// Refuses `target` when the origin's `robots.txt` disallows it.
///
/// Callers must only reach this when `--obey-robots` is set.
pub(crate) async fn ensure_fetch_allowed(fetch_config: &FetchConfig, target: &Url) -> Result<()> {
    let Some(robots_url) = robots_txt_url(target) else {
        // Only HTTP(S) origins publish a robots.txt. A `file:` or `data:`
        // target has no policy to obey, which is not the same as a policy that
        // refuses.
        return Ok(());
    };

    let policy = load_policy(fetch_config, &robots_url).await;
    let user_agent = fetch_config.user_agent();
    if policy.allows(user_agent, &robots_request_target(target)) {
        return Ok(());
    }

    if policy == RobotsPolicy::DisallowAll {
        bail!(
            "`{robots_url}` could not be read, so --obey-robots treats the entire origin as \
             disallowed (RFC 9309 §2.3.1.4); re-run without --obey-robots to fetch anyway"
        );
    }

    bail!(
        "`{target}` is disallowed by `{robots_url}` for user agent `{user_agent}`; \
         re-run without --obey-robots to fetch anyway"
    )
}

async fn load_policy(fetch_config: &FetchConfig, robots_url: &Url) -> RobotsPolicy {
    match fetch_robots_txt(fetch_config, robots_url).await {
        Ok((status, body)) => RobotsPolicy::from_http_status(status, &body),
        Err(error) => {
            // An origin that will not serve robots.txt is "unreachable" rather
            // than "unrestricted", so the failure is reported through the
            // policy instead of aborting the run with a transport error.
            tracing::debug!(
                url = %robots_url,
                error = %error,
                "treating robots.txt as unreachable"
            );
            RobotsPolicy::unreachable()
        }
    }
}

async fn fetch_robots_txt(fetch_config: &FetchConfig, robots_url: &Url) -> Result<(u16, String)> {
    let mut config = fetch_config.clone();
    // The robots.txt request must never re-enter this check.
    config.set_obey_robots(false);

    // robots.txt is origin-scoped rather than session-scoped, so it is fetched
    // with an empty cookie jar and a client that is torn down immediately.
    let client = FetchClient::new(&config, new_shared_browser_cookie_store());
    let response = client
        .fetch(Request::get(robots_url.as_str())?)
        .await
        .with_context(|| format!("failed to fetch `{robots_url}`"));
    let _ = client.shutdown();

    let response = response?;
    Ok((response.status, response.body_text().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn non_http_targets_skip_the_check() {
        // A `file:` target must not be refused just because it cannot carry a
        // robots policy. No network access happens on this path.
        let config = FetchConfig::default();
        for raw in ["file:///tmp/page.html", "data:text/html,hi", "about:blank"] {
            let target = Url::parse(raw).expect("valid url");
            let result = ensure_fetch_allowed(&config, &target).await;
            assert!(result.is_ok(), "{raw} should be allowed: {result:?}");
        }
    }
}
